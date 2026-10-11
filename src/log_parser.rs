use crate::model::{LoggedMatch, LoggedPlayer};

/// Splits `(I) [22:11:45.050] [000015420]: message` into the time of day and the message.
fn split_prefix(line: &str) -> (Option<&str>, &str) {
    match line.split_once("]: ") {
        Some((head, message)) => {
            let time = head
                .split_once('[')
                .and_then(|(_, rest)| rest.split_once(']'))
                .map(|(time, _)| time);
            (time, message)
        }
        None => (None, line),
    }
}

/// Parses `<slot> <name> <profile_id> <team> <faction>`. Names may contain spaces (or be
/// empty), so everything but the slot is taken from the right.
fn parse_player(rest: &str, ai: bool) -> Option<LoggedPlayer> {
    let (slot, rest) = rest.split_once(' ')?;
    let mut fields = rest.rsplitn(4, ' ');
    let faction = fields.next()?;
    let team = fields.next()?;
    let profile_id = fields.next()?;
    let name = fields.next().unwrap_or("");
    Some(LoggedPlayer {
        slot: slot.parse().ok()?,
        name: name.to_string(),
        profile_id: profile_id.parse::<i64>().ok().filter(|id| *id > 0).map(|id| id as u64),
        team: team.parse().ok()?,
        faction: faction.to_string(),
        ai,
    })
}

/// What a log line can tell about the course of a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    /// A match was set up and is loading.
    Started(LoggedMatch),
    /// The player left the match they were in and is back in the menu.
    Left,
    /// The game logged the account in; from here on it is running, in the menu or a match.
    LoggedIn(u64),
    /// The game was closed, which also ends any match.
    Closed,
}

/// Builds matches out of log lines fed one at a time.
#[derive(Debug, Default)]
pub struct MatchAssembler {
    current: Option<LoggedMatch>,
    pending_match_type: Option<String>,
    local_profile_id: Option<u64>,
}

impl MatchAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one log line. A match is reported once the line after its last player has been seen.
    pub fn feed(&mut self, line: &str) -> Option<LogEvent> {
        let (time, message) = split_prefix(line);

        if message.starts_with("GameApp::SetState : new (UnloadingGame)") {
            return Some(self.finish().map_or(LogEvent::Left, LogEvent::Started));
        }
        if message.starts_with("Application closed") {
            return Some(self.finish().map_or(LogEvent::Closed, LogEvent::Started));
        }

        if let Some(game) = message.strip_prefix("GAME -- ") {
            if let Some(scenario) = game.strip_prefix("Scenario: ") {
                let finished = self.finish().map(LogEvent::Started);
                // The main menu is a scene of its own; loading it means any match is over.
                let menu = scenario.contains("\\frontend\\").then_some(LogEvent::Left);
                self.current = Some(LoggedMatch {
                    scenario: scenario.trim().to_string(),
                    win_condition: None,
                    match_type: None,
                    started_at: time.map(str::to_string),
                    players: Vec::new(),
                    local_profile_id: self.local_profile_id,
                });
                return finished.or(menu);
            }
            if let Some(current) = &mut self.current {
                if let Some(name) = game.strip_prefix("Win Condition Name: ") {
                    current.win_condition = Some(name.trim().to_string());
                    return None;
                }
                if game.starts_with("Win Condition Qualified Name: ") {
                    return None;
                }
                let player = if let Some(rest) = game.strip_prefix("Human Player: ") {
                    parse_player(rest, false)
                } else if let Some(rest) = game.strip_prefix("AI Player: ") {
                    parse_player(rest, true)
                } else {
                    None
                };
                if let Some(player) = player {
                    current.players.push(player);
                    return None;
                }
            }
        } else if let Some((_, rest)) = message.split_once("matchTypeID: ") {
            self.pending_match_type = rest.split_whitespace().next().map(str::to_string);
        } else if let Some((_, rest)) = message.split_once("LocalProfileID: ") {
            self.local_profile_id = rest.trim().parse().ok();
            let logged_in = self.local_profile_id.map(LogEvent::LoggedIn);
            return self.finish().map(LogEvent::Started).or(logged_in);
        }

        self.finish().map(LogEvent::Started)
    }

    /// Closes the open block if it has players. Menu scenes and blocks without a real
    /// player are dropped.
    fn finish(&mut self) -> Option<LoggedMatch> {
        if self.current.as_ref()?.players.is_empty() {
            return None;
        }
        let mut finished = self.current.take()?;
        let frontend = finished.scenario.contains("\\frontend\\");
        if frontend || finished.players.iter().all(|p| p.profile_id.is_none()) {
            return None;
        }
        finished.match_type = self.pending_match_type.take();
        Some(finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = include_str!("../tests/fixtures/warnings_excerpt.log");

    fn parse_all(log: &str) -> Vec<LoggedMatch> {
        let mut assembler = MatchAssembler::new();
        log.lines()
            .filter_map(|line| match assembler.feed(line) {
                Some(LogEvent::Started(started)) => Some(started),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ignores_frontend_scene_and_finds_real_matches() {
        let matches = parse_all(LOG);
        let maps: Vec<_> = matches.iter().map(|m| m.map()).collect();
        assert_eq!(maps, ["elst_outskirts_4p", "alamein_4p"]);
    }

    #[test]
    fn parses_names_with_spaces_and_umlauts() {
        let matches = parse_all(LOG);
        let first = &matches[0].players[0];
        assert_eq!(first.name, "Heartless Jäger");
        assert_eq!(first.profile_id, Some(317504));
        assert_eq!(first.team, 0);
        assert_eq!(first.faction, "germans");
        assert!(!first.ai);
    }

    #[test]
    fn captures_match_metadata() {
        let matches = parse_all(LOG);
        // The replay block has no preceding matchTypeID line.
        assert_eq!(matches[0].match_type, None);

        let last = &matches[1];
        assert_eq!(last.match_type.as_deref(), Some("21/2v2_unranked"));
        assert_eq!(last.win_condition.as_deref(), Some("VictoryPoint"));
        assert_eq!(last.started_at.as_deref(), Some("22:11:45.050"));
        assert_eq!(last.local_profile_id, Some(63411));
        assert_eq!(last.team_size(), 2);
        let names: Vec<_> = last.players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Nullgerät", "Von_Dolofon", "ZOCKER748", "Saulek"]);
    }

    #[test]
    fn match_is_only_emitted_after_the_block_ends() {
        let mut assembler = MatchAssembler::new();
        let block = [
            "(I) [22:11:45.050] [000015420]: GAME -- Scenario: data:scenarios\\multiplayer\\alamein_4p\\alamein_4p",
            "(I) [22:11:45.050] [000015420]: GAME -- Human Player: 0 Nullgerät 63411 0 germans",
            "(I) [22:11:45.050] [000015420]: GAME -- Human Player: 1 Saulek 3034 1 americans",
        ];
        for line in block {
            assert_eq!(assembler.feed(line), None);
        }
        let done = assembler.feed("(I) [22:11:45.050] [000015420]: GameApp::SetState : new (LoadingGame) old (UnloadedGame)");
        let Some(LogEvent::Started(started)) = done else {
            panic!("expected the match, got {done:?}");
        };
        assert_eq!(started.players.len(), 2);
    }

    #[test]
    fn reports_the_login() {
        let mut assembler = MatchAssembler::new();
        let first = LOG.lines().next().unwrap();
        assert_eq!(assembler.feed(first), Some(LogEvent::LoggedIn(63411)));
    }

    #[test]
    fn reports_leaving_a_match() {
        let mut assembler = MatchAssembler::new();
        let leaving = [
            "(I) [00:41:00.123] [000015420]: GameApp::SetState : new (UnloadingGame) old (Game)",
            "(I) [00:41:06.000] [000015420]: GAME -- Scenario: data:scenarios\\frontend\\showcase\\italy\\showcase_italy",
        ];
        for line in leaving {
            assert_eq!(assembler.feed(line), Some(LogEvent::Left), "{line}");
        }
        assert_eq!(assembler.feed("Application closed without errors"), Some(LogEvent::Closed));
        // Loading into a match and the result screen are not leaving.
        let staying = [
            "(I) [00:06:48.000] [000015420]: GameApp::SetState : new (Game) old (LoadingGame)",
            "(I) [00:40:28.000] [000015420]: GameWorld::SetGameOver: MatchStatus[2] WinReason[2]:",
            "(I) [00:40:28.000] [000015420]: MOD -- Game Over at frame 16152",
        ];
        for line in staying {
            assert_eq!(assembler.feed(line), None, "{line}");
        }
    }

    #[test]
    fn ai_players_have_no_profile_id() {
        let player = parse_player("1 CPU - Standard -1 1 americans", true).unwrap();
        assert_eq!(player.name, "CPU - Standard");
        assert_eq!(player.profile_id, None);
        assert!(player.ai);
    }
}
