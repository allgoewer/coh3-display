pub mod applog;
#[cfg(windows)]
pub mod autostart;
pub mod display_link;
pub mod firmware;
pub mod game;
pub mod i18n;
pub mod icon;
pub mod log_parser;
pub mod maps;
pub mod model;
pub mod relic;
pub mod render;
pub mod settings;
pub mod tray_model;
pub mod update;
pub mod watcher;

use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::applog::log;
use crate::log_parser::LogEvent;
use crate::model::{MatchInfo, OwnStats, Phase};
use crate::relic::RelicClient;
use crate::watcher::LogTail;

/// How long the opponents stay on the display after leaving a match.
pub const LINGER: Duration = Duration::from_secs(5 * 60);
/// The game writes its log many times a minute. A match with no end marker whose log has
/// been quiet this long was cut short by a crash.
pub const QUIET_LIMIT: Duration = Duration::from_secs(10 * 60);

/// When the player left the latest match, as far as the tracker can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// Already over when the log was first read; there is no telling how long ago.
    BeforeStartup,
    /// Seen as it happened, this long ago.
    Ago(Duration),
}

/// Decides whether a match is still worth showing.
pub fn phase(left: Option<Left>, log_quiet_for: Option<Duration>, linger: Duration) -> Phase {
    match left {
        None if log_quiet_for.is_some_and(|quiet| quiet >= QUIET_LIMIT) => Phase::Over,
        None => Phase::Live,
        Some(Left::Ago(ago)) if ago < linger => Phase::Lingering,
        Some(_) => Phase::Over,
    }
}

/// How often the game's process is looked for, and its player's own ratings are refreshed.
const PROCESS_CHECK: Duration = Duration::from_secs(5);
const OWN_STATS_REFRESH: Duration = Duration::from_secs(120);
const OWN_STATS_RETRY: Duration = Duration::from_secs(30);

/// Whose ratings to show: the logged-in account, as long as the game is really running.
/// The log alone cannot tell, because a game that crashed never logs that it closed.
pub fn player_in_game(logged_in: Option<u64>, process_running: bool) -> Option<u64> {
    logged_in.filter(|_| process_running)
}

/// Follows the log forever and publishes the latest match, with stats and its phase, on `tx`,
/// and on `own_tx` the player's own ratings for as long as the game is running.
pub async fn track(
    mut tail: LogTail,
    client: RelicClient,
    tx: watch::Sender<Option<MatchInfo>>,
    own_tx: watch::Sender<Option<OwnStats>>,
    interval: Duration,
    linger: Duration,
) {
    let mut current: Option<MatchInfo> = None;
    // `None` while the match is on; otherwise when it was left (`None` inside: before startup).
    let mut left: Option<Option<Instant>> = None;
    let mut first_poll = true;
    let mut logged_in: Option<u64> = None;
    let mut process: Option<(Instant, bool)> = None;
    // When the own ratings were last asked for, and when to ask again.
    let mut own_asked: Option<(Instant, Duration)> = None;
    loop {
        match tail.poll() {
            Ok(events) => {
                // Only the newest match matters, also when several were found on startup.
                let mut started = None;
                for event in events {
                    match event {
                        LogEvent::Started(logged) => {
                            started = Some(logged);
                            left = None;
                        }
                        LogEvent::LoggedIn(profile_id) => logged_in = Some(profile_id),
                        LogEvent::Left | LogEvent::Closed => {
                            if event == LogEvent::Closed {
                                logged_in = None;
                            }
                            if left.is_none() {
                                left = Some((!first_poll).then(Instant::now));
                                // The match that just ended has moved the ratings.
                                own_asked = None;
                            }
                        }
                    }
                }
                first_poll = false;
                if let Some(logged) = started {
                    current = Some(client.enrich(&logged).await);
                }
            }
            Err(err) => log(format!("Could not read {}: {err}", tail.path().display())),
        }

        if let Some(info) = &mut current {
            let left = left.map(|at| at.map_or(Left::BeforeStartup, |at| Left::Ago(at.elapsed())));
            info.phase = phase(left, tail.quiet_for(), linger);
            let changed = tx.send_if_modified(|published| {
                let changed = published.as_ref() != Some(&*info);
                if changed {
                    *published = Some(info.clone());
                }
                changed
            });
            if changed && tx.is_closed() {
                return;
            }
        }

        if process.is_none_or(|(at, _)| at.elapsed() >= PROCESS_CHECK) {
            process = Some((Instant::now(), logged_in.is_some() && game::is_running()));
        }
        let player = player_in_game(logged_in, process.is_some_and(|(_, running)| running));
        match player {
            // During a match the opponents are shown, and the ratings do not move.
            Some(profile_id) if current.as_ref().is_none_or(|info| info.phase != Phase::Live) => {
                if own_asked.is_none_or(|(at, wait)| at.elapsed() >= wait) {
                    let wait = match client.own_stats(profile_id).await {
                        Ok(own) => {
                            own_tx.send_if_modified(|published| {
                                let changed = published.as_ref() != Some(&own);
                                if changed {
                                    *published = Some(own);
                                }
                                changed
                            });
                            OWN_STATS_REFRESH
                        }
                        Err(err) => {
                            log(format!("Could not fetch the player's own ratings: {err:#}"));
                            OWN_STATS_RETRY
                        }
                    };
                    own_asked = Some((Instant::now(), wait));
                }
            }
            Some(_) => {}
            None => {
                own_asked = None;
                own_tx.send_if_modified(|published| published.take().is_some());
            }
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn own_ratings_need_a_login_and_a_running_game() {
        assert_eq!(player_in_game(Some(63411), true), Some(63411));
        // Closed, or never logged in.
        assert_eq!(player_in_game(None, true), None);
        // Crashed: the log still says logged in, but the process is gone.
        assert_eq!(player_in_game(Some(63411), false), None);
    }

    #[test]
    fn a_match_is_live_until_it_is_left() {
        assert_eq!(phase(None, Some(Duration::from_secs(2)), LINGER), Phase::Live);
        // The log's age may be unknown; that alone ends nothing.
        assert_eq!(phase(None, None, LINGER), Phase::Live);
    }

    #[test]
    fn opponents_linger_for_a_while_after_leaving() {
        assert_eq!(phase(Some(Left::Ago(Duration::ZERO)), None, LINGER), Phase::Lingering);
        assert_eq!(phase(Some(Left::Ago(4 * MINUTE)), None, LINGER), Phase::Lingering);
        assert_eq!(phase(Some(Left::Ago(5 * MINUTE)), None, LINGER), Phase::Over);
        assert_eq!(phase(Some(Left::Ago(MINUTE)), None, Duration::from_secs(10)), Phase::Over);
    }

    #[test]
    fn a_match_that_ended_before_startup_is_over() {
        assert_eq!(phase(Some(Left::BeforeStartup), Some(Duration::ZERO), LINGER), Phase::Over);
    }

    #[test]
    fn a_quiet_log_means_the_game_died_mid_match() {
        assert_eq!(phase(None, Some(9 * MINUTE), LINGER), Phase::Live);
        assert_eq!(phase(None, Some(10 * MINUTE), LINGER), Phase::Over);
    }
}
