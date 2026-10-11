use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Faction {
    American,
    British,
    Dak,
    German,
}

impl Faction {
    /// In the order the display lists them.
    pub const ALL: [Faction; 4] = [Faction::American, Faction::British, Faction::Dak, Faction::German];

    /// Maps the faction name used in `warnings.log` to a playable faction.
    pub fn from_log(name: &str) -> Option<Self> {
        match name {
            "americans" => Some(Self::American),
            "british_africa" | "british" => Some(Self::British),
            "afrika_korps" => Some(Self::Dak),
            "germans" => Some(Self::German),
            _ => None,
        }
    }
}

/// A player exactly as the game logged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedPlayer {
    pub slot: u8,
    pub name: String,
    /// Relic profile ID; `None` for AI players.
    pub profile_id: Option<u64>,
    pub team: u8,
    pub faction: String,
    pub ai: bool,
}

/// A match start block from `warnings.log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedMatch {
    pub scenario: String,
    pub win_condition: Option<String>,
    /// e.g. `21/2v2_unranked`, when the game logged one before the match.
    pub match_type: Option<String>,
    /// Local time of day from the log line, `hh:mm:ss.mmm`.
    pub started_at: Option<String>,
    pub players: Vec<LoggedPlayer>,
    /// Profile of the account running the game, from the log's login line.
    pub local_profile_id: Option<u64>,
}

impl LoggedMatch {
    pub fn map(&self) -> &str {
        self.scenario.rsplit(['\\', '/']).next().unwrap_or(&self.scenario)
    }

    /// Size of the largest team, which decides the leaderboard (1v1 .. 4v4).
    pub fn team_size(&self) -> usize {
        let mut sizes = std::collections::HashMap::new();
        for player in &self.players {
            *sizes.entry(player.team).or_insert(0usize) += 1;
        }
        sizes.into_values().max().unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerStats {
    /// ELO on the leaderboard for this match's mode and the player's faction.
    pub rating: i64,
    pub wins: u32,
    pub losses: u32,
    /// Positive for a win streak, negative for a loss streak.
    pub streak: i32,
}

impl PlayerStats {
    pub fn winrate(&self) -> Option<f32> {
        let games = self.wins + self.losses;
        (games > 0).then(|| self.wins as f32 / games as f32)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerInfo {
    pub slot: u8,
    pub name: String,
    pub team: u8,
    pub faction: String,
    pub ai: bool,
    pub profile_id: Option<u64>,
    pub country: Option<String>,
    /// `None` when the player has no games on that leaderboard, or stats could not be fetched.
    pub stats: Option<PlayerStats>,
}

/// The local player's own ratings, shown while the game is open but no match is on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct OwnStats {
    /// The player's name; empty when the API did not give one.
    pub name: String,
    /// ELO for 1v1 and 2v2 (in this order), each for the factions in [`Faction::ALL`]'s
    /// order. `None` where the player has no games.
    pub ratings: [[Option<i64>; 4]; 2],
}

/// Where a match stands, which decides whether the display shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Loading or being played.
    #[default]
    Live,
    /// Left a moment ago; the opponents stay up for a while.
    Lingering,
    /// Long over, or over for an unknown time.
    Over,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchInfo {
    pub map: String,
    /// `1v1` .. `4v4`
    pub mode: String,
    pub match_type: Option<String>,
    pub win_condition: Option<String>,
    pub started_at: Option<String>,
    pub players: Vec<PlayerInfo>,
    /// Profile of the account running the game; absent from `players` when watching a replay.
    pub local_profile_id: Option<u64>,
    /// Set when the Relic API call failed; players then carry no stats.
    pub stats_error: Option<String>,
    pub phase: Phase,
}
