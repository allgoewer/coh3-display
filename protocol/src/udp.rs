//! Finding displays on the LAN, and frames as they travel in datagrams.

use core::fmt;
use core::time::Duration;

use crate::frame::{Frame, MAX_LINE, Parser};

/// Displays broadcast their beacon to this port.
pub const BEACON_PORT: u16 = 42424;
/// Displays receive frames on this port.
pub const FRAME_PORT: u16 = 42425;
const BEACON_PREFIX: &str = "COH3DISPLAY 1 ";
/// Broadcast by the PC; a display answers it with its beacon, sent straight back. Unlike the
/// display's own broadcast, that answer gets through a firewall that blocks unsolicited traffic.
pub const PROBE: &[u8] = b"COH3DISPLAY? 1\n";
/// How often a display broadcasts its beacon.
pub const BEACON_INTERVAL: Duration = Duration::from_secs(2);
/// How often the PC broadcasts its probe.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(2);
/// The PC forgets a display whose beacons stopped for this long.
pub const DISPLAY_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether a datagram asks the display to send its beacon straight back.
pub fn is_probe(data: &[u8]) -> bool {
    data.starts_with(b"COH3DISPLAY?")
}

/// What a display announces itself with: its MAC address.
pub struct Beacon(pub [u8; 6]);

impl fmt::Display for Beacon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(BEACON_PREFIX)?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str("\n")
    }
}

/// Returns the display's ID (its MAC address) if `data` is a beacon.
pub fn parse_beacon(data: &[u8]) -> Option<&str> {
    let id = core::str::from_utf8(data).ok()?.trim_end().strip_prefix(BEACON_PREFIX)?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// A frame as it goes into a UDP datagram, where the display's firmware is not known. One
/// from before 1.3.0 does not know the ratings frame and would not answer it, so an idle
/// line goes in front: old firmware goes idle on it, new firmware takes the last frame.
pub struct Datagram<'a>(pub &'a Frame);

impl fmt::Display for Datagram<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.0, Frame::Stats(_)) {
            Frame::Idle.fmt(f)?;
        }
        self.0.fmt(f)
    }
}

/// Parses a UDP datagram, which carries one whole frame.
pub fn parse_datagram(data: &[u8]) -> Option<Frame> {
    let text = core::str::from_utf8(data).ok()?;
    let mut parser = Parser::new();
    let mut frame = None;
    for line in text.split('\n').filter(|line| line.len() <= MAX_LINE) {
        if let Some(complete) = parser.feed_line(line) {
            frame = Some(complete);
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::tests::{own, two_v_two};

    #[test]
    fn beacons_are_recognised() {
        let beacon = Beacon([0xf0, 0xf5, 0xbd, 0x2d, 0x1b, 0x88]).to_string();
        assert_eq!(beacon, "COH3DISPLAY 1 f0f5bd2d1b88\n");
        assert_eq!(parse_beacon(beacon.as_bytes()), Some("f0f5bd2d1b88"));
        assert_eq!(parse_beacon(b"COH3DISPLAY 1 \n"), None);
        assert_eq!(parse_beacon(b"COH3DISPLAY 2 f0f5bd2d1b88\n"), None);
        assert_eq!(parse_beacon(b"OK\n"), None);
    }

    #[test]
    fn a_probe_is_not_a_beacon() {
        assert!(is_probe(PROBE));
        assert!(!is_probe(b"COH3DISPLAY 1 f0f5bd2d1b88\n"));
        assert_eq!(parse_beacon(PROBE), None);
    }

    #[test]
    fn ratings_follow_an_idle_line_for_older_firmware() {
        let stats = Frame::Stats(own());
        let packet = Datagram(&stats).to_string();
        assert!(packet.starts_with("I\nS\t"));
        assert_eq!(parse_datagram(packet.as_bytes()), Some(stats));

        // A match goes out as it is.
        let game = Frame::Match(two_v_two());
        assert_eq!(Datagram(&game).to_string(), game.to_string());
        assert_eq!(parse_datagram(game.to_string().as_bytes()), Some(game));
        assert_eq!(parse_datagram(Datagram(&Frame::Idle).to_string().as_bytes()), Some(Frame::Idle));
    }
}
