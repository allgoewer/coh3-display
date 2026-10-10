use std::fmt::Write;

use crate::model::{MatchInfo, PlayerInfo};

fn stats_columns(player: &PlayerInfo) -> String {
    if player.ai {
        return "AI".to_string();
    }
    let Some(stats) = &player.stats else {
        return "ELO    -   unrated".to_string();
    };
    let winrate = match stats.winrate() {
        Some(rate) => format!("{:.0}%", rate * 100.0),
        None => "-".to_string(),
    };
    let record = format!("{}W {}L {winrate}", stats.wins, stats.losses);
    format!("ELO {:>4}   {record:<16}   streak {:+}", stats.rating, stats.streak)
}

/// Renders a match as plain text, one block per team.
pub fn render(info: &MatchInfo) -> String {
    let mut out = String::new();

    let _ = write!(out, "{} · {}", info.map, info.mode);
    if let Some(match_type) = &info.match_type {
        let _ = write!(out, " ({match_type})");
    }
    if let Some(started_at) = &info.started_at {
        let _ = write!(out, " · {}", started_at.split('.').next().unwrap_or(started_at));
    }
    out.push('\n');
    if let Some(err) = &info.stats_error {
        let _ = writeln!(out, "Stats unavailable: {err}");
    }

    let name_width = info.players.iter().map(|p| p.name.chars().count()).max().unwrap_or(0);
    let faction_width = info.players.iter().map(|p| p.faction.len()).max().unwrap_or(0);

    let mut teams: Vec<u8> = info.players.iter().map(|p| p.team).collect();
    teams.sort_unstable();
    teams.dedup();
    for team in teams {
        let _ = writeln!(out, "Team {team}");
        for player in info.players.iter().filter(|p| p.team == team) {
            let line = format!(
                "  {:<name_width$}   {:<faction_width$}   {}   {}",
                player.name,
                player.faction,
                stats_columns(player),
                player.country.as_deref().unwrap_or(""),
            );
            let _ = writeln!(out, "{}", line.trim_end());
        }
    }
    out
}
