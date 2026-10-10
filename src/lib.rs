pub mod applog;
#[cfg(windows)]
pub mod autostart;
pub mod display_link;
pub mod firmware;
pub mod i18n;
pub mod icon;
pub mod log_parser;
pub mod maps;
pub mod model;
pub mod relic;
pub mod render;
pub mod settings;
pub mod tray_model;
pub mod watcher;

use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::applog::log;
use crate::log_parser::LogEvent;
use crate::model::{MatchInfo, Phase};
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

/// Follows the log forever and publishes the latest match, with stats and its phase, on `tx`.
pub async fn track(
    mut tail: LogTail,
    client: RelicClient,
    tx: watch::Sender<Option<MatchInfo>>,
    interval: Duration,
    linger: Duration,
) {
    let mut current: Option<MatchInfo> = None;
    // `None` while the match is on; otherwise when it was left (`None` inside: before startup).
    let mut left: Option<Option<Instant>> = None;
    let mut first_poll = true;
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
                        LogEvent::Left if left.is_none() => {
                            left = Some((!first_poll).then(Instant::now));
                        }
                        LogEvent::Left => {}
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
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

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
