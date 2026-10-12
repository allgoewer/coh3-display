//! Managing the board itself, which only works over serial: what it runs, and its WiFi
//! credentials.

use core::fmt;
use core::net::Ipv4Addr;

/// The longest network name the board stores, in bytes.
pub const MAX_SSID: usize = 32;
/// The longest password the board is sent, in bytes; an empty one means an open network.
pub const MAX_PASSWORD: usize = 63;
/// The shortest password that is not empty.
pub const MIN_PASSWORD: usize = 8;

/// The board's answers to [`Command::SetWifi`]. After the first it restarts.
pub const WIFI_STORED: &str = "OK\n";
pub const WIFI_REFUSED: &str = "ERR wifi\n";

/// A line that asks something of the board rather than of its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    /// Answered with a [`Status`].
    Status,
    /// Stores WiFi credentials. An empty password means an open network, an empty SSID
    /// wipes what is stored.
    SetWifi { ssid: &'a str, password: &'a str },
}

impl<'a> Command<'a> {
    /// Reads a line without its newline; `None` if it is something else, like part of a frame.
    pub fn parse(line: &'a str) -> Option<Self> {
        if line == "S" {
            return Some(Command::Status);
        }
        let fields = match line.strip_prefix('W')? {
            "" => "",
            fields => fields.strip_prefix('\t')?,
        };
        // Whatever follows the SSID is the password; one with a tab in it is refused.
        let (ssid, password) = fields.split_once('\t').unwrap_or((fields, ""));
        Some(Command::SetWifi { ssid, password })
    }
}

impl fmt::Display for Command<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Status => f.write_str("S\n"),
            Command::SetWifi { ssid, password } => writeln!(f, "W\t{ssid}\t{password}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiState {
    /// No credentials on the board.
    Unset,
    /// Credentials present but unusable, or the radio failed.
    Off,
    Connecting,
    /// Connected, with the address DHCP handed out.
    Up(Option<Ipv4Addr>),
}

/// What a board says about itself in answer to [`Command::Status`]:
/// `STATUS fw=0.2.0 wifi=up ip=10.0.0.5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status<'a> {
    /// `None` for firmware from before it reported a version.
    pub version: Option<&'a str>,
    pub wifi: WifiState,
}

impl<'a> Status<'a> {
    pub fn parse(line: &'a str) -> Option<Self> {
        let mut fields = line.trim().split(' ');
        if fields.next()? != "STATUS" {
            return None;
        }
        let (mut version, mut wifi, mut ip) = (None, None, None);
        for field in fields {
            match field.split_once('=')? {
                ("fw", value) => version = Some(value),
                ("wifi", value) => wifi = Some(value),
                ("ip", value) => ip = value.parse().ok(),
                _ => {}
            }
        }
        let wifi = match wifi? {
            "unset" => WifiState::Unset,
            "off" => WifiState::Off,
            "connecting" => WifiState::Connecting,
            "up" => WifiState::Up(ip),
            _ => return None,
        };
        Some(Status { version, wifi })
    }
}

impl fmt::Display for Status<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("STATUS ")?;
        if let Some(version) = self.version {
            write!(f, "fw={version} ")?;
        }
        match self.wifi {
            WifiState::Unset => f.write_str("wifi=unset\n"),
            WifiState::Off => f.write_str("wifi=off\n"),
            WifiState::Connecting => f.write_str("wifi=connecting\n"),
            WifiState::Up(Some(ip)) => writeln!(f, "wifi=up ip={ip}"),
            WifiState::Up(None) => f.write_str("wifi=up\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_round_trip() {
        let up = Status { version: Some("0.2.0"), wifi: WifiState::Up(Some(Ipv4Addr::new(10, 204, 28, 172))) };
        assert_eq!(up.to_string(), "STATUS fw=0.2.0 wifi=up ip=10.204.28.172\n");
        for wifi in [WifiState::Unset, WifiState::Off, WifiState::Connecting, WifiState::Up(None), up.wifi] {
            let status = Status { version: Some("1.4.0"), wifi };
            assert_eq!(Status::parse(&status.to_string()), Some(status));
        }
        assert_eq!(Status::parse("STATUS fw=0.2.0 wifi=unset").map(|status| status.wifi), Some(WifiState::Unset));
    }

    #[test]
    fn reads_what_older_firmware_and_other_boards_say() {
        // Firmware from before versions were reported.
        let old = Status { version: None, wifi: WifiState::Connecting };
        assert_eq!(Status::parse("STATUS wifi=connecting"), Some(old));
        assert_eq!(old.to_string(), "STATUS wifi=connecting\n");
        assert_eq!(Status::parse("OK"), None);
        assert_eq!(Status::parse("STATUS wifi=sideways"), None);
        assert_eq!(Status::parse("ESP-ROM:esp32c6-20220919"), None);
    }

    #[test]
    fn commands_round_trip() {
        let set = Command::SetWifi { ssid: "Mein WLAN", password: "geheim-äöü-1234" };
        assert_eq!(set.to_string(), "W\tMein WLAN\tgeheim-äöü-1234\n");
        let open = Command::SetWifi { ssid: "Open network", password: "" };
        for command in [Command::Status, set, open] {
            assert_eq!(Command::parse(command.to_string().trim_end_matches('\n')), Some(command));
        }
        assert_eq!(Command::Status.to_string(), "S\n");
    }

    #[test]
    fn tells_commands_from_frames() {
        // `W` alone wipes the credentials.
        assert_eq!(Command::parse("W"), Some(Command::SetWifi { ssid: "", password: "" }));
        assert_eq!(Command::parse("W\tHome"), Some(Command::SetWifi { ssid: "Home", password: "" }));
        // A field too many ends up in the password, where the board refuses the tab.
        assert_eq!(Command::parse("W\ta\tb\tc"), Some(Command::SetWifi { ssid: "a", password: "b\tc" }));
        // The start of a ratings frame is not a request for the status.
        assert_eq!(Command::parse("S\tNullgerät"), None);
        for line in ["", "E", "I", "M\t2v2\tEl Alamein", "Wx", "STATUS wifi=up"] {
            assert_eq!(Command::parse(line), None, "{line}");
        }
    }
}
