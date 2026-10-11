use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::model::{Faction, LoggedMatch, MatchInfo, OwnStats, Phase, PlayerInfo, PlayerStats};

const BASE_URL: &str = "https://coh3-api.reliclink.com";
const ATTEMPTS: u32 = 3;

/// Leaderboard for a team size (1..=4) and faction, as used by coh3stats.com.
pub fn leaderboard_id(team_size: usize, faction: Faction) -> Option<u32> {
    let ids = match faction {
        Faction::American => [2130255, 2130300, 2130329, 2130353],
        Faction::British => [2130257, 2130302, 2130331, 2130356],
        Faction::Dak => [2130259, 2130304, 2130333, 2130358],
        Faction::German => [2130261, 2130306, 2130335, 2130360],
    };
    ids.get(team_size.checked_sub(1)?).copied()
}

#[derive(Debug, Deserialize)]
pub struct PersonalStatResponse {
    result: ApiResult,
    #[serde(default, rename = "statGroups")]
    stat_groups: Vec<StatGroup>,
    #[serde(default, rename = "leaderboardStats")]
    leaderboard_stats: Vec<LeaderboardStat>,
}

#[derive(Debug, Deserialize)]
struct ApiResult {
    code: i32,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct StatGroup {
    #[serde(default)]
    members: Vec<Member>,
}

#[derive(Debug, Deserialize)]
struct Member {
    profile_id: u64,
    personal_statgroup_id: u64,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    country: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LeaderboardStat {
    statgroup_id: u64,
    leaderboard_id: u32,
    wins: u32,
    losses: u32,
    streak: i32,
    rating: i64,
}

#[derive(Debug, Clone)]
pub struct RelicClient {
    http: reqwest::Client,
}

impl RelicClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("coh3-display/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http })
    }

    /// Fetches personal stats for all given profiles in one request.
    pub async fn personal_stats(&self, profile_ids: &[u64]) -> Result<PersonalStatResponse> {
        let ids: Vec<String> = profile_ids.iter().map(u64::to_string).collect();
        let url = format!(
            "{BASE_URL}/community/leaderboard/GetPersonalStat?title=coh3&profile_ids=[{}]",
            ids.join(",")
        );

        let mut attempt = 1;
        loop {
            match self.request(&url).await {
                Ok(response) => return Ok(response),
                Err(err) if attempt < ATTEMPTS => {
                    crate::applog::log(format!("Relic API request failed (attempt {attempt}/{ATTEMPTS}): {err:#}"));
                    tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
                    attempt += 1;
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn request(&self, url: &str) -> Result<PersonalStatResponse> {
        let response: PersonalStatResponse = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("unexpected response from Relic API")?;
        if response.result.code != 0 {
            bail!("Relic API returned {} ({})", response.result.code, response.result.message);
        }
        Ok(response)
    }

    /// Looks up the 1v1 and 2v2 ratings of one player.
    pub async fn own_stats(&self, profile_id: u64) -> Result<OwnStats> {
        Ok(own_stats(&self.personal_stats(&[profile_id]).await?, profile_id))
    }

    /// Looks up every human player of `logged` and combines the result with the log data.
    pub async fn enrich(&self, logged: &LoggedMatch) -> MatchInfo {
        let ids: Vec<u64> = logged.players.iter().filter_map(|p| p.profile_id).collect();
        match self.personal_stats(&ids).await {
            Ok(response) => build_match_info(logged, Some(&response), None),
            Err(err) => build_match_info(logged, None, Some(format!("{err:#}"))),
        }
    }
}

/// Picks a player's 1v1 and 2v2 ratings for every faction out of a response.
pub fn own_stats(response: &PersonalStatResponse, profile_id: u64) -> OwnStats {
    let member = response
        .stat_groups
        .iter()
        .flat_map(|group| &group.members)
        .find(|member| member.profile_id == profile_id);
    let rating = |team_size: usize, faction: Faction| {
        let (member, leaderboard) = (member?, leaderboard_id(team_size, faction)?);
        response
            .leaderboard_stats
            .iter()
            .find(|row| row.statgroup_id == member.personal_statgroup_id && row.leaderboard_id == leaderboard)
            .map(|row| row.rating)
    };
    OwnStats {
        name: member.map(|member| member.alias.clone()).unwrap_or_default(),
        ratings: [1, 2].map(|team_size| Faction::ALL.map(|faction| rating(team_size, faction))),
    }
}

/// Joins the logged players with their leaderboard entry for this match's mode and their faction.
pub fn build_match_info(
    logged: &LoggedMatch,
    response: Option<&PersonalStatResponse>,
    stats_error: Option<String>,
) -> MatchInfo {
    let team_size = logged.team_size();

    let players = logged
        .players
        .iter()
        .map(|player| {
            let member = response.zip(player.profile_id).and_then(|(response, id)| {
                response
                    .stat_groups
                    .iter()
                    .flat_map(|group| &group.members)
                    .find(|member| member.profile_id == id)
            });
            let leaderboard = Faction::from_log(&player.faction)
                .and_then(|faction| leaderboard_id(team_size, faction));
            let stats = response.zip(member).zip(leaderboard).and_then(|((response, member), leaderboard)| {
                response
                    .leaderboard_stats
                    .iter()
                    .find(|row| row.statgroup_id == member.personal_statgroup_id && row.leaderboard_id == leaderboard)
                    .map(|row| PlayerStats {
                        rating: row.rating,
                        wins: row.wins,
                        losses: row.losses,
                        streak: row.streak,
                    })
            });

            PlayerInfo {
                slot: player.slot,
                name: player.name.clone(),
                team: player.team,
                faction: player.faction.clone(),
                ai: player.ai,
                profile_id: player.profile_id,
                country: member.and_then(|m| m.country.clone()).filter(|c| !c.is_empty()),
                stats,
            }
        })
        .collect();

    MatchInfo {
        map: logged.map().to_string(),
        mode: format!("{team_size}v{team_size}"),
        match_type: logged.match_type.clone(),
        win_condition: logged.win_condition.clone(),
        started_at: logged.started_at.clone(),
        players,
        local_profile_id: logged.local_profile_id,
        stats_error,
        phase: Phase::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LoggedPlayer;

    const RESPONSE: &str = include_str!("../tests/fixtures/personal_stat.json");

    fn player(slot: u8, name: &str, profile_id: Option<u64>, team: u8, faction: &str) -> LoggedPlayer {
        LoggedPlayer {
            slot,
            name: name.to_string(),
            profile_id,
            team,
            faction: faction.to_string(),
            ai: profile_id.is_none(),
        }
    }

    fn two_v_two() -> LoggedMatch {
        LoggedMatch {
            scenario: "data:scenarios\\multiplayer\\alamein_4p\\alamein_4p".to_string(),
            win_condition: Some("VictoryPoint".to_string()),
            match_type: Some("21/2v2_unranked".to_string()),
            started_at: Some("22:11:45.050".to_string()),
            players: vec![
                player(0, "Nullgerät", Some(63411), 0, "germans"),
                player(1, "Von_Dolofon", Some(15127), 1, "americans"),
                player(2, "ZOCKER748", Some(873412), 0, "germans"),
                player(3, "CPU - Standard", None, 1, "americans"),
            ],
            local_profile_id: Some(63411),
        }
    }

    #[test]
    fn leaderboard_ids_match_coh3stats() {
        assert_eq!(leaderboard_id(1, Faction::American), Some(2130255));
        assert_eq!(leaderboard_id(2, Faction::German), Some(2130306));
        assert_eq!(leaderboard_id(4, Faction::British), Some(2130356));
        assert_eq!(leaderboard_id(0, Faction::Dak), None);
        assert_eq!(leaderboard_id(5, Faction::Dak), None);
    }

    #[test]
    fn picks_the_leaderboard_for_mode_and_faction() {
        let response: PersonalStatResponse = serde_json::from_str(RESPONSE).unwrap();
        let info = build_match_info(&two_v_two(), Some(&response), None);

        assert_eq!(info.map, "alamein_4p");
        assert_eq!(info.mode, "2v2");
        let me = &info.players[0];
        assert_eq!(me.country.as_deref(), Some("de"));
        // 2v2 german, not 1v1 german or 2v2 american.
        let stats = me.stats.as_ref().unwrap();
        assert_eq!((stats.rating, stats.wins, stats.losses, stats.streak), (1369, 44, 28, -1));
        assert!((stats.winrate().unwrap() - 44.0 / 72.0).abs() < 1e-6);
    }

    #[test]
    fn collects_a_players_own_ratings() {
        let response: PersonalStatResponse = serde_json::from_str(RESPONSE).unwrap();
        let own = own_stats(&response, 63411);
        assert_eq!(own.name, "Nullgerät");
        // 1v1: only USF. 2v2: USF and Wehrmacht.
        assert_eq!(own.ratings[0], [Some(1207), None, None, None]);
        assert_eq!(own.ratings[1], [Some(1408), None, None, Some(1369)]);

        // Someone the response does not know has no name and no ratings.
        assert_eq!(own_stats(&response, 1), OwnStats::default());
    }

    #[test]
    fn players_missing_from_the_response_are_unrated() {
        let response: PersonalStatResponse = serde_json::from_str(RESPONSE).unwrap();
        let info = build_match_info(&two_v_two(), Some(&response), None);
        assert_eq!(info.players[1].stats, None);
        assert_eq!(info.players[3].stats, None);
        assert!(info.players[3].ai);
    }

    #[test]
    fn api_failure_keeps_the_players() {
        let info = build_match_info(&two_v_two(), None, Some("offline".to_string()));
        assert_eq!(info.players.len(), 4);
        assert!(info.players.iter().all(|p| p.stats.is_none()));
        assert_eq!(info.stats_error.as_deref(), Some("offline"));
    }
}
