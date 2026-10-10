use core::fmt::Write as _;

use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::{Read, Write};
use esp_hal::Async;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_storage::FlashStorage;
use heapless::{String, Vec};

use crate::credentials::Credentials;
use crate::net::{WifiState, wifi_state};
use crate::protocol::{MAX_LINE, Parser};
use crate::{FRAME, frame_reply, storage};

/// `STATUS fw=0.2.0 wifi=up ip=192.168.1.23` — the answer to an `S` line.
fn status_line() -> String<64> {
    let mut status: String<64> = String::new();
    let _ = write!(status, "STATUS fw={} ", env!("CARGO_PKG_VERSION"));
    let _ = match wifi_state() {
        WifiState::Off => writeln!(status, "wifi=off"),
        WifiState::Unset => writeln!(status, "wifi=unset"),
        WifiState::Connecting => writeln!(status, "wifi=connecting"),
        WifiState::Up([a, b, c, d]) => writeln!(status, "wifi=up ip={a}.{b}.{c}.{d}"),
    };
    status
}

/// Handles `W <ssid> <password>`: stores the credentials (an empty SSID wipes them).
/// Returns whether flash was written, in which case the board must restart to use them.
fn store_wifi(flash: &mut FlashStorage<'_>, fields: &str) -> bool {
    let mut fields = fields.split('\t');
    let ssid = fields.next().unwrap_or("");
    let password = fields.next().unwrap_or("");
    if fields.next().is_some() {
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
            let reply: Option<&[u8]> = match core::str::from_utf8(&line).map(|text| text.trim_end_matches('\r')) {
                Ok("S") if !overflow => {
                    status = status_line();
                    Some(status.as_bytes())
                }
                Ok(text) if !overflow && (text == "W" || text.starts_with("W\t")) => {
                    restart = store_wifi(&mut flash, text.get(2..).unwrap_or(""));
                    Some(if restart { b"OK\n" } else { b"ERR wifi\n" })
                }
                Ok(text) if !overflow => parser.feed_line(text).map(|frame| {
                    FRAME.signal(frame);
                    frame_reply()
                }),
                _ => None,
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
