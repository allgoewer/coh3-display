//! What the host asks the screen to show: frames, as text lines.

use core::fmt;
use core::time::Duration;

use heapless::{String, Vec};

pub const MAX_PLAYERS: usize = 8;
pub const MAX_LINE: usize = 128;
/// How many characters of a player's name the display is sent.
pub const MAX_NAME: usize = 24;
/// Map titles get more room: the longest is "Überquerung des Rapido-Flusses".
pub const MAX_MAP: usize = 32;
/// The brightness of a match whose frame does not say.
pub const NORMAL_BRIGHTNESS: u8 = 2;
/// The host repeats the current frame this often, so the display recovers on its own after a
/// reset or replug.
pub const RESEND: Duration = Duration::from_secs(5);
/// A display that has heard no frame for this long goes idle: the host is gone.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// `A` american, `B` british, `D` dak, `G` german, `?` anything else.
    pub faction: char,
    pub elo: Option<u16>,
    pub wins: Option<u32>,
    pub losses: Option<u32>,
    /// Positive for a win streak, negative for a loss streak.
    pub streak: Option<i32>,
    /// Two-letter code, empty when unknown.
    pub country: String<4>,
    pub name: String<48>,
}

impl Player {
    /// Rounded percentage of games won.
    pub fn winrate(&self) -> Option<u32> {
        let (wins, losses) = (self.wins?, self.losses?);
        let games = wins + losses;
        (games > 0).then(|| (wins * 100 + games / 2) / games)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub mode: String<8>,
    pub map: String<48>,
    /// How bright to show the match: 1 low, 2 normal, 3 high.
    pub brightness: u8,
    pub players: Vec<Player, MAX_PLAYERS>,
}

/// The player's own ratings, for the time the game sits in its menu.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnStats {
    pub name: String<48>,
    /// How bright to show them, as for a match. A frame that does not say gets 1 (low).
    pub brightness: u8,
    /// One page per mode the host sent: the team size (1 for 1v1, up to 4), and the rating
    /// for each of the factions A, B, D, G; `None` where there are no games.
    pub pages: Vec<(u8, [Option<u16>; 4]), 4>,
}

/// What the host can ask the screen to show. Written with `Display`, read by a [`Parser`].
// No heap on the display to box the match in.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Match(Match),
    Stats(OwnStats),
    /// Nothing worth showing: the match is over, or there is none.
    Idle,
}

/// Copies as many whole characters of `text` as fit.
fn fit<const N: usize>(text: &str) -> String<N> {
    let mut out = String::new();
    for c in text.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

/// Cuts text down to what the display can show: no control characters, Latin-1 only, at
/// most `max_chars` of them and no more than fit.
pub fn display_text<const N: usize>(text: &str, max_chars: usize) -> String<N> {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()).take(max_chars) {
        if out.push(if (c as u32) <= 0xFF { c } else { '?' }).is_err() {
            break;
        }
    }
    out
}

/// A number, or `-` where there is none.
fn write_field(f: &mut fmt::Formatter<'_>, value: Option<impl fmt::Display>) -> fmt::Result {
    match value {
        Some(value) => write!(f, "\t{value}"),
        None => f.write_str("\t-"),
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Frame::Match(game) => {
                write!(f, "M\t{}\t{}\nB\t{}\n", game.mode, game.map, game.brightness)?;
                for player in &game.players {
                    write!(f, "P\t{}", player.faction)?;
                    write_field(f, player.elo)?;
                    write_field(f, player.wins)?;
                    write_field(f, player.losses)?;
                    write_field(f, player.streak)?;
                    write_field(f, Some(&player.country).filter(|country| !country.is_empty()))?;
                    writeln!(f, "\t{}", player.name)?;
                }
                f.write_str("E\n")
            }
            Frame::Stats(own) => {
                write!(f, "S\t{}\nB\t{}\n", own.name, own.brightness)?;
                // One line, and on the display one page, per mode.
                for (team_size, ratings) in &own.pages {
                    write!(f, "R\t{team_size}v{team_size}")?;
                    for rating in ratings {
                        write_field(f, *rating)?;
                    }
                    f.write_str("\n")?;
                }
                f.write_str("E\n")
            }
            Frame::Idle => f.write_str("I\n"),
        }
    }
}

fn parse_player<'a>(mut fields: impl Iterator<Item = &'a str>) -> Option<Player> {
    Some(Player {
        faction: fields.next()?.chars().next()?,
        elo: fields.next()?.parse().ok(),
        wins: fields.next()?.parse().ok(),
        losses: fields.next()?.parse().ok(),
        streak: fields.next()?.parse().ok(),
        country: match fields.next()? {
            "-" => String::new(),
            code => fit(code),
        },
        name: fit(fields.next().unwrap_or("")),
    })
}

/// Assembles frames from protocol lines.
pub struct Parser {
    pending: Option<Match>,
    pending_stats: Option<OwnStats>,
}

impl Parser {
    pub const fn new() -> Self {
        Self { pending: None, pending_stats: None }
    }

    /// Feeds one line without its newline. Returns the frame when the line completes one.
    pub fn feed_line(&mut self, line: &str) -> Option<Frame> {
        let mut fields = line.trim_end_matches('\r').split('\t');
        match fields.next()? {
            "S" => {
                self.pending = None;
                self.pending_stats =
                    Some(OwnStats { name: fit(fields.next().unwrap_or("")), brightness: 1, pages: Vec::new() });
                None
            }
            "R" => {
                let team_size = match fields.next() {
                    Some("1v1") => 1,
                    Some("2v2") => 2,
                    Some("3v3") => 3,
                    Some("4v4") => 4,
                    _ => return None,
                };
                if let Some(stats) = &mut self.pending_stats {
                    let mut ratings = [None; 4];
                    for rating in &mut ratings {
                        *rating = fields.next().and_then(|rating| rating.parse().ok());
                    }
                    // More lines than there are modes: the rest is dropped.
                    let _ = stats.pages.push((team_size, ratings));
                }
                None
            }
            "M" => {
                self.pending_stats = None;
                self.pending = Some(Match {
                    mode: fit(fields.next().unwrap_or("")),
                    map: fit(fields.next().unwrap_or("")),
                    brightness: NORMAL_BRIGHTNESS,
                    players: Vec::new(),
                });
                None
            }
            "B" => {
                let level = fields.next().and_then(|level| level.parse().ok()).filter(|level| (1..=3).contains(level));
                if let Some(level) = level {
                    if let Some(pending) = &mut self.pending {
                        pending.brightness = level;
                    }
                    if let Some(pending) = &mut self.pending_stats {
                        pending.brightness = level;
                    }
                }
                None
            }
            "P" => {
                if let (Some(pending), Some(player)) = (&mut self.pending, parse_player(fields)) {
                    // More than MAX_PLAYERS: the rest is dropped.
                    let _ = pending.players.push(player);
                }
                None
            }
            "E" => {
                // Ratings without a single mode are nothing to show.
                let stats = self.pending_stats.take();
                let stats = stats.map(|stats| if stats.pages.is_empty() { Frame::Idle } else { Frame::Stats(stats) });
                self.pending.take().map(Frame::Match).or(stats)
            }
            "I" => {
                self.pending = None;
                self.pending_stats = None;
                Some(Frame::Idle)
            }
            _ => None,
        }
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// The display's answer to a frame, the same on every transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Frame received and shown.
    Shown,
    /// Frame received, but the board could not reach its OLED over I2C.
    NoOled,
}

impl Reply {
    pub const fn as_bytes(self) -> &'static [u8] {
        match self {
            Reply::Shown => b"OK\n",
            Reply::NoOled => b"ERR display\n",
        }
    }

    /// Reads a datagram that carries a reply.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.starts_with(b"OK") {
            Some(Reply::Shown)
        } else if data.starts_with(b"ERR") {
            Some(Reply::NoOled)
        } else {
            None
        }
    }

    /// Looks for a reply in what a serial port delivered so far, which may hold other output.
    pub fn find(received: &[u8]) -> Option<Self> {
        if received.windows(3).any(|w| w == b"OK\n") {
            Some(Reply::Shown)
        } else if received.windows(4).any(|w| w == b"ERR ") {
            Some(Reply::NoOled)
        } else {
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn player(faction: char, name: &str, stats: Option<(u16, u32, u32, i32)>) -> Player {
        Player {
            faction,
            elo: stats.map(|stats| stats.0),
            wins: stats.map(|stats| stats.1),
            losses: stats.map(|stats| stats.2),
            streak: stats.map(|stats| stats.3),
            country: fit(if stats.is_some() { "de" } else { "" }),
            name: fit(name),
        }
    }

    pub(crate) fn two_v_two() -> Match {
        Match {
            mode: fit("2v2"),
            map: fit("El Alamein"),
            brightness: 3,
            players: Vec::from_iter([
                player('G', "Nullgerät", Some((1369, 44, 28, -1))),
                player('D', "ZOCKER748", None),
                player('A', "Von_Dolofon", Some((1284, 164, 135, 3))),
                player('?', "", None),
            ]),
        }
    }

    pub(crate) fn own() -> OwnStats {
        OwnStats {
            name: fit("Nullgerät"),
            brightness: 2,
            pages: Vec::from_iter([
                (1, [Some(1207), None, None, None]),
                (2, [Some(1408), None, None, Some(1369)]),
                (4, [Some(1250), Some(1100), None, Some(1310)]),
            ]),
        }
    }

    fn parse(text: &str) -> Option<Frame> {
        let mut parser = Parser::new();
        text.lines().find_map(|line| parser.feed_line(line))
    }

    #[test]
    fn a_match_reads_back_as_it_was_written() {
        let frame = Frame::Match(two_v_two());
        let text = frame.to_string();
        assert_eq!(
            text,
            "M\t2v2\tEl Alamein\n\
             B\t3\n\
             P\tG\t1369\t44\t28\t-1\tde\tNullgerät\n\
             P\tD\t-\t-\t-\t-\t-\tZOCKER748\n\
             P\tA\t1284\t164\t135\t3\tde\tVon_Dolofon\n\
             P\t?\t-\t-\t-\t-\t-\t\n\
             E\n"
        );
        assert!(text.lines().all(|line| line.len() <= MAX_LINE));
        assert_eq!(parse(&text), Some(frame));
    }

    #[test]
    fn the_longest_lines_fit() {
        let wide = Player {
            faction: 'G',
            elo: Some(u16::MAX),
            wins: Some(u32::MAX),
            losses: Some(u32::MAX),
            streak: Some(i32::MIN),
            country: fit("de"),
            name: display_text(&"ü".repeat(40), MAX_NAME),
        };
        assert_eq!(wide.name.chars().count(), MAX_NAME);
        let map = display_text(&"ü".repeat(40), MAX_MAP);
        let frame =
            Frame::Match(Match { mode: fit("4v4"), map, brightness: 2, players: Vec::from_iter([wide.clone()]) });
        let text = frame.to_string();
        assert!(text.lines().all(|line| line.len() <= MAX_LINE));
        assert_eq!(parse(&text), Some(frame));
    }

    #[test]
    fn own_ratings_read_back_as_they_were_written() {
        let frame = Frame::Stats(own());
        let text = frame.to_string();
        assert_eq!(
            text,
            "S\tNullgerät\nB\t2\nR\t1v1\t1207\t-\t-\t-\nR\t2v2\t1408\t-\t-\t1369\nR\t4v4\t1250\t1100\t-\t1310\nE\n"
        );
        assert_eq!(parse(&text), Some(frame));

        // An app from before the ratings carried a brightness gets them dimmed, as it expects.
        let old = parse("S\tNullgerät\nR\t1v1\t1207\t-\t-\t-\nE");
        assert!(matches!(old, Some(Frame::Stats(old)) if old.brightness == 1));
        // Ratings without a single mode are nothing to show.
        let empty = Frame::Stats(OwnStats { pages: Vec::new(), ..own() });
        assert_eq!(parse(&empty.to_string()), Some(Frame::Idle));
    }

    #[test]
    fn idle_is_a_frame_of_its_own_and_drops_a_half_sent_match() {
        assert_eq!(Frame::Idle.to_string(), "I\n");
        let mut parser = Parser::new();
        assert_eq!(parser.feed_line("M\t2v2\talamein"), None);
        assert_eq!(parser.feed_line("I"), Some(Frame::Idle));
        assert_eq!(parser.feed_line("E"), None);
    }

    #[test]
    fn brightness_is_normal_unless_the_frame_says() {
        let brightness = |text: &str| match parse(text) {
            Some(Frame::Match(parsed)) => parsed.brightness,
            other => panic!("frame ends with E, got {other:?}"),
        };
        for level in 1..=3 {
            let frame = Frame::Match(Match { brightness: level, ..two_v_two() });
            assert_eq!(brightness(&frame.to_string()), level);
        }
        // A frame from an older app has no such line, and nonsense is not taken at its word.
        assert_eq!(brightness("M\t2v2\tEl Alamein\nE"), 2);
        assert_eq!(brightness("M\t2v2\tEl Alamein\nB\tx\nE"), 2);
        assert_eq!(brightness("M\t2v2\tEl Alamein\nB\t9\nE"), 2);
    }

    #[test]
    fn players_beyond_the_eighth_are_dropped() {
        let mut text = std::string::String::from("M\t4v4\tEl Alamein\n");
        for _ in 0..10 {
            text.push_str("P\tG\t-\t-\t-\t-\t-\tx\n");
        }
        text.push_str("E\n");
        assert!(matches!(parse(&text), Some(Frame::Match(parsed)) if parsed.players.len() == MAX_PLAYERS));
    }

    #[test]
    fn works_out_the_winrate() {
        let parsed = two_v_two();
        assert_eq!(parsed.players[0].winrate(), Some(61));
        assert_eq!(parsed.players[1].winrate(), None);
    }

    #[test]
    fn text_is_made_safe_for_the_display() {
        let name: String<48> = display_text("Tab\there Paweł and a very long tail", MAX_NAME);
        assert_eq!(name.as_str(), "Tabhere Pawe? and a very");
        // Never more than fits, and never half a character.
        let short: String<4> = display_text("äöü", MAX_NAME);
        assert_eq!(short.as_str(), "äö");
    }

    #[test]
    fn replies_are_recognised() {
        for reply in [Reply::Shown, Reply::NoOled] {
            assert_eq!(Reply::parse(reply.as_bytes()), Some(reply));
            assert_eq!(Reply::find(reply.as_bytes()), Some(reply));
        }
        assert_eq!(Reply::parse(b"COH3DISPLAY 1 f0f5bd2d1b88\n"), None);
        // Over serial a reply may follow boot messages, and is not complete without its end.
        assert_eq!(Reply::find(b"ESP-ROM:esp32c6\nOK\n"), Some(Reply::Shown));
        assert_eq!(Reply::find(b"O"), None);
        assert_eq!(Reply::find(b"OK"), None);
    }
}
