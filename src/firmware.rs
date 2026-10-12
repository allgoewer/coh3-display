//! The display firmware carried inside the app: finding out what a board runs, installing
//! or updating it over USB, and handing it WiFi credentials.

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use display_protocol::board::{Command, MAX_PASSWORD, MAX_SSID, MIN_PASSWORD, Status, WIFI_STORED};
use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
use espflash::flasher::{FlashData, FlashSettings, Flasher};
use espflash::image_format::ImageFormat;
use espflash::image_format::idf::IdfBootloaderFormat;
use espflash::target::{Chip, ProgressCallbacks};
use serialport::{FlowControl, SerialPort, SerialPortType};

use crate::i18n::Lang;

/// The firmware built into this exe.
#[derive(Debug, Clone, Copy)]
pub struct Embedded {
    pub version: &'static str,
    elf: &'static [u8],
}

#[cfg(embedded_firmware)]
pub fn embedded() -> Option<Embedded> {
    Some(Embedded { version: env!("COH3_FIRMWARE_VERSION"), elf: include_bytes!(env!("COH3_FIRMWARE_ELF")) })
}

#[cfg(not(embedded_firmware))]
pub fn embedded() -> Option<Embedded> {
    None
}

/// What a board says about its WiFi.
pub use display_protocol::board::WifiState as BoardWifi;

/// What a board said about itself in answer to `S`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardStatus {
    /// `None` for firmware from before it reported a version.
    pub version: Option<String>,
    pub wifi: BoardWifi,
}

/// Parses `STATUS fw=0.2.0 wifi=up ip=10.0.0.5`.
pub fn parse_status(line: &str) -> Option<BoardStatus> {
    let status = Status::parse(line)?;
    Some(BoardStatus { version: status.version.map(str::to_string), wifi: status.wifi })
}

/// How the firmware on a board compares with the one in this exe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirmwareState {
    /// The board did not answer: blank, or running something else.
    Missing,
    /// Older than the embedded firmware; `None` if it predates version reporting.
    Outdated(Option<String>),
    Current(String),
}

fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

pub fn classify(status: Option<&BoardStatus>, embedded: Option<&str>) -> FirmwareState {
    let Some(status) = status else {
        return FirmwareState::Missing;
    };
    match (&status.version, embedded) {
        (None, _) => FirmwareState::Outdated(None),
        (Some(installed), Some(embedded)) if version_key(installed) < version_key(embedded) => {
            FirmwareState::Outdated(Some(installed.clone()))
        }
        (Some(installed), _) => FirmwareState::Current(installed.clone()),
    }
}

/// Reads until a line with `prefix` arrives or `timeout` passes.
fn read_line_with(port: &mut dyn SerialPort, prefix: &str, timeout: Duration) -> io::Result<Option<String>> {
    let deadline = Instant::now() + timeout;
    let mut received = Vec::new();
    let mut buf = [0u8; 128];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(n) => received.extend_from_slice(&buf[..n]),
            Err(err) if err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err),
        }
        let text = String::from_utf8_lossy(&received);
        // Only complete lines count; boot messages may come first.
        if let Some(line) = text.split_inclusive('\n').find(|l| l.ends_with('\n') && l.starts_with(prefix)) {
            return Ok(Some(line.trim_end().to_string()));
        }
    }
    Ok(None)
}

/// Asks the board what it is running. `None` means it did not answer like a display.
pub fn probe(port: &mut dyn SerialPort) -> io::Result<Option<BoardStatus>> {
    // A board that was just plugged in or reset needs a moment to boot.
    for _ in 0..3 {
        // The empty line in front ends whatever the board may have half received.
        port.write_all(format!("\n{}", Command::Status).as_bytes())?;
        port.flush()?;
        if let Some(line) = read_line_with(port, "STATUS", Duration::from_millis(700))? {
            return Ok(parse_status(&line));
        }
    }
    Ok(None)
}

/// Like [`probe`], but a board that is present yet cannot be talked to counts as not
/// answering. A blank ESP32 restarts in a loop, which makes writes to its port fail.
pub fn probe_present(port: &mut dyn SerialPort, port_name: &str) -> io::Result<Option<BoardStatus>> {
    match probe(port) {
        Ok(status) => Ok(status),
        Err(err) => {
            let present = serialport::available_ports()
                .is_ok_and(|ports| ports.iter().any(|candidate| candidate.port_name == port_name));
            if present { Ok(None) } else { Err(err) }
        }
    }
}

/// Why the board would not accept WiFi credentials. The wording is in [`crate::i18n`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    SsidLength,
    PasswordTooLong,
    PasswordTooShort,
    ControlCharacters,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Lang::En.credential_error(*self))
    }
}

impl std::error::Error for CredentialError {}

/// Checks credentials against what the board accepts, so mistakes are reported before sending.
pub fn check_credentials(ssid: &str, password: &str) -> Result<(), CredentialError> {
    if ssid.is_empty() || ssid.len() > MAX_SSID {
        return Err(CredentialError::SsidLength);
    }
    if password.len() > MAX_PASSWORD {
        return Err(CredentialError::PasswordTooLong);
    }
    if !password.is_empty() && password.len() < MIN_PASSWORD {
        return Err(CredentialError::PasswordTooShort);
    }
    if ssid.chars().chain(password.chars()).any(char::is_control) {
        return Err(CredentialError::ControlCharacters);
    }
    Ok(())
}

/// Stores WiFi credentials on the board, which then restarts and joins the network.
pub fn set_wifi(port: &mut dyn SerialPort, ssid: &str, password: &str) -> Result<()> {
    check_credentials(ssid, password)?;
    port.write_all(format!("\n{}", Command::SetWifi { ssid, password }).as_bytes())?;
    port.flush()?;
    match read_line_with(port, "", Duration::from_secs(2))?.as_deref() {
        Some(reply) if reply == WIFI_STORED.trim_end() => Ok(()),
        Some(other) => bail!("the display refused the WiFi settings ({other})"),
        None => bail!("the display did not answer"),
    }
}

/// Reports the progress of the application image; the bootloader and partition table before
/// it are too small to be worth showing.
struct Progress<'a> {
    report: &'a mut dyn FnMut(u8),
    total: usize,
    is_app: bool,
}

impl ProgressCallbacks for Progress<'_> {
    fn init(&mut self, addr: u32, total: usize) {
        self.is_app = addr >= 0x1_0000;
        self.total = total.max(1);
    }

    fn update(&mut self, current: usize) {
        if self.is_app {
            (self.report)((current * 100 / self.total).min(100) as u8);
        }
    }

    fn verifying(&mut self) {}

    fn finish(&mut self, _skipped: bool) {
        if self.is_app {
            (self.report)(100);
        }
    }
}

/// Installs the embedded firmware on the board at `port_name` and restarts it. The port
/// must not be open elsewhere. `report` receives the progress in percent.
pub fn install(port_name: &str, report: &mut dyn FnMut(u8)) -> Result<()> {
    let firmware = embedded().context("this build of the app carries no firmware")?;

    let port_info = serialport::available_ports()?
        .into_iter()
        .find(|port| port.port_name == port_name)
        .and_then(|port| match port.port_type {
            SerialPortType::UsbPort(info) => Some(info),
            _ => None,
        })
        .with_context(|| format!("{port_name} is not a USB serial port"))?;
    let serial = serialport::new(port_name, 115_200)
        .flow_control(FlowControl::None)
        .open_native()
        .with_context(|| format!("could not open {port_name}"))?;
    let connection = Connection::new(
        serial,
        port_info,
        ResetAfterOperation::HardReset,
        ResetBeforeOperation::DefaultReset,
        115_200,
    );
    let mut flasher =
        Flasher::connect(connection, true, true, true, None, None).context("could not reach the board's bootloader")?;

    let chip = flasher.chip();
    if chip != Chip::Esp32c6 {
        // Leave a foreign board as it was found.
        let _ = flasher.connection().reset_after(true, chip);
        bail!("the board on {port_name} is an {chip}, but the display firmware is for the ESP32-C6");
    }
    let crystal = chip.xtal_frequency(flasher.connection())?;
    let flash_size = flasher.flash_detect().ok().flatten();
    let flash_data = FlashData::new(FlashSettings::new(None, flash_size, None), 0, None, chip, crystal);
    let image = IdfBootloaderFormat::new(firmware.elf, &flash_data, None, None, None, None)?;

    let mut progress = Progress { report, total: 1, is_app: false };
    flasher.load_image_to_flash(&mut progress, ImageFormat::EspIdf(image)).context("writing the firmware failed")?;
    flasher.connection().reset_after(true, chip)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The firmware's own credential record code, compiled into the host tests.
    #[path = "../../../firmware/src/credentials.rs"]
    #[allow(dead_code)]
    mod device;

    #[test]
    fn parses_status_replies() {
        assert_eq!(
            parse_status("STATUS fw=0.2.0 wifi=up ip=10.204.28.172\n"),
            Some(BoardStatus {
                version: Some("0.2.0".to_string()),
                wifi: BoardWifi::Up("10.204.28.172".parse().ok())
            })
        );
        assert_eq!(
            parse_status("STATUS fw=0.2.0 wifi=unset"),
            Some(BoardStatus { version: Some("0.2.0".to_string()), wifi: BoardWifi::Unset })
        );
        // Firmware from before versions were reported.
        assert_eq!(
            parse_status("STATUS wifi=connecting"),
            Some(BoardStatus { version: None, wifi: BoardWifi::Connecting })
        );
        assert_eq!(parse_status("OK"), None);
        assert_eq!(parse_status("STATUS wifi=sideways"), None);
        assert_eq!(parse_status("ESP-ROM:esp32c6-20220919"), None);
    }

    #[test]
    fn compares_firmware_versions() {
        let status = |version: Option<&str>| BoardStatus { version: version.map(str::to_string), wifi: BoardWifi::Unset };

        assert_eq!(classify(None, Some("0.2.0")), FirmwareState::Missing);
        assert_eq!(classify(Some(&status(None)), Some("0.2.0")), FirmwareState::Outdated(None));
        assert_eq!(
            classify(Some(&status(Some("0.1.9"))), Some("0.2.0")),
            FirmwareState::Outdated(Some("0.1.9".to_string()))
        );
        assert_eq!(classify(Some(&status(Some("0.2.0"))), Some("0.2.0")), FirmwareState::Current("0.2.0".to_string()));
        // Numeric, not alphabetic; and a newer board is left alone.
        assert_eq!(
            classify(Some(&status(Some("0.10.0"))), Some("0.9.0")),
            FirmwareState::Current("0.10.0".to_string())
        );
        // Without embedded firmware there is nothing to update to.
        assert_eq!(classify(Some(&status(Some("0.1.0"))), None), FirmwareState::Current("0.1.0".to_string()));
    }

    #[test]
    fn rejects_credentials_the_board_would_refuse() {
        assert!(check_credentials("Home", "correct horse").is_ok());
        assert!(check_credentials("Open network", "").is_ok());
        assert!(check_credentials("", "password1").is_err());
        assert!(check_credentials("Home", "short").is_err());
        assert!(check_credentials("Home", "has\ttab in it").is_err());
        assert!(check_credentials(&"x".repeat(33), "password1").is_err());
        assert!(check_credentials("Home", &"x".repeat(64)).is_err());
    }

    #[test]
    fn credential_records_round_trip_and_reject_damage() {
        let credentials = device::Credentials::new("Mein WLAN", "geheim-äöü-1234").unwrap();
        let mut record = credentials.encode();
        assert_eq!(device::Credentials::decode(&record), Some(credentials));

        record[10] ^= 0x01;
        assert_eq!(device::Credentials::decode(&record), None);
        // Erased flash.
        assert_eq!(device::Credentials::decode(&[0xFF; device::RECORD_SIZE]), None);
        assert!(device::Credentials::new("", "whatever").is_none());
        assert!(device::Credentials::new(&"x".repeat(33), "").is_none());
    }
}
