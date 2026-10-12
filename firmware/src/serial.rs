use core::fmt::Write as _;

use display_protocol::Parser;
use display_protocol::board::{Command, Status, WIFI_REFUSED, WIFI_STORED};
use display_protocol::frame::MAX_LINE;
use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::{Read, Write};
use esp_hal::Async;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_storage::FlashStorage;
use heapless::{String, Vec};

use crate::credentials::Credentials;
use crate::net::wifi_state;
use crate::{FRAME, frame_reply, storage};

/// `STATUS fw=0.2.0 wifi=up ip=192.168.1.23` — the answer to a status request.
fn status_line() -> String<64> {
    let mut status: String<64> = String::new();
    let _ = write!(status, "{}", Status { version: Some(env!("CARGO_PKG_VERSION")), wifi: wifi_state() });
    status
}

/// Stores the credentials of a `W` line (an empty SSID wipes them). Returns whether flash
/// was written, in which case the board must restart to use them.
fn store_wifi(flash: &mut FlashStorage<'_>, ssid: &str, password: &str) -> bool {
    // A tab in the password is a field too many.
    if password.contains('\t') {
        return false;
    }
    match Credentials::new(ssid, password) {
        Some(credentials) => storage::store(flash, Some(&credentials)),
        None if ssid.is_empty() => storage::store(flash, None),
        None => false,
    }
}

/// Reads protocol lines from the host and publishes completed frames on [`FRAME`].
#[embassy_executor::task]
pub async fn serial_task(usb: UsbSerialJtag<'static, Async>, mut flash: FlashStorage<'static>) {
    let (mut rx, mut tx) = usb.split();
    let mut parser = Parser::new();
    let mut line: Vec<u8, MAX_LINE> = Vec::new();
    let mut overflow = false;
    let mut chunk = [0u8; 64];

    loop {
        let Ok(received) = rx.read(&mut chunk).await;
        for &byte in &chunk[..received] {
            if byte != b'\n' {
                overflow |= line.push(byte).is_err();
                continue;
            }
            let status;
            let mut restart = false;
            let text = core::str::from_utf8(&line).ok().filter(|_| !overflow).map(|text| text.trim_end_matches('\r'));
            let reply: Option<&[u8]> = match text.map(|text| (Command::parse(text), text)) {
                Some((Some(Command::Status), _)) => {
                    status = status_line();
                    Some(status.as_bytes())
                }
                Some((Some(Command::SetWifi { ssid, password }), _)) => {
                    restart = store_wifi(&mut flash, ssid, password);
                    Some(if restart { WIFI_STORED.as_bytes() } else { WIFI_REFUSED.as_bytes() })
                }
                Some((None, text)) => parser.feed_line(text).map(|frame| {
                    FRAME.signal(frame);
                    frame_reply()
                }),
                None => None,
            };
            line.clear();
            overflow = false;

            if let Some(reply) = reply {
                // Do not stall on a host that stopped reading.
                let _ = with_timeout(Duration::from_millis(100), async {
                    let _ = tx.write_all(reply).await;
                    let _ = tx.flush().await;
                })
                .await;
            }
            if restart {
                // Let the answer leave before the USB connection drops.
                Timer::after(Duration::from_millis(200)).await;
                esp_hal::system::software_reset();
            }
        }
    }
}
