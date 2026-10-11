//! Device side of the display protocol described in `PROTOCOL.md`.
//!
//! Only `core` and `heapless`, so the tracker's tests can include this file directly.

use heapless::{String, Vec};

pub const MAX_PLAYERS: usize = 8;
pub const MAX_LINE: usize = 128;
/// The brightness of a match whose frame does not say.
pub const NORMAL_BRIGHTNESS: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
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
    /// 1v1 and 2v2, each for the factions A, B, D, G; `None` where there are no games.
    pub ratings: [[Option<u16>; 4]; 2],
}

/// What the host can ask the screen to show.
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
                    Some(OwnStats { name: fit(fields.next().unwrap_or("")), brightness: 1, ratings: [[None; 4]; 2] });
                None
            }
            "R" => {
                let row = match fields.next() {
                    Some("1v1") => 0,
                    Some("2v2") => 1,
                    _ => return None,
                };
                if let Some(stats) = &mut self.pending_stats {
                    for rating in &mut stats.ratings[row] {
                        *rating = fields.next().and_then(|rating| rating.parse().ok());
                    }
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
                let stats = self.pending_stats.take().map(Frame::Stats);
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

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}
