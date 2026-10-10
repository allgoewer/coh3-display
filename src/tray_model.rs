//! What the tray menu shows, worked out from the app's state without touching any GUI.

use crate::display_link::{Ack, LinkStatus, UsbLink, short_map, shown_players};
use crate::firmware::{BoardWifi, FirmwareState};
use crate::model::{Faction, MatchInfo, Phase, PlayerInfo};
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Toggle {
    Usb,
    Wifi,
    ShowOwnTeam,
    AutoUpdateFirmware,
    KeepLastMatch,
    Autostart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    InstallFirmware,
    SetupWifi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Information only, not clickable.
    Label(String),
    Separator,
    /// Opens the player's coh3stats.com page when a profile is known.
    Player { label: String, profile_id: Option<u64> },
    Check { label: &'static str, checked: bool, toggle: Toggle },
    /// Something to do with the board on USB; asks before it acts.
    Command { label: String, command: Command },
    OpenLog,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuModel {
    pub tooltip: String,
    /// At least one display confirmed it is showing the current match.
    pub connected: bool,
    pub entries: Vec<Entry>,
}

/// `crossing_in_the_woods_4p` -> `Crossing In The Woods`
pub fn map_title(map: &str) -> String {
    short_map(map)
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            first + chars.as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn faction_label(player: &PlayerInfo) -> &str {
    match Faction::from_log(&player.faction) {
        Some(Faction::American) => "USF",
        Some(Faction::British) => "UKF",
        Some(Faction::Dak) => "DAK",
        Some(Faction::German) => "Wehr",
        None => &player.faction,
    }
}

/// `FairyTale · USF 1215 · 63%`
fn player_label(player: &PlayerInfo) -> String {
    let faction = faction_label(player);
    match &player.stats {
        Some(stats) => match stats.winrate() {
            Some(rate) => format!("{} · {faction} {} · {:.0}%", player.name, stats.rating, rate * 100.0),
            None => format!("{} · {faction} {}", player.name, stats.rating),
        },
        None if player.ai => format!("{} · {faction} · AI", player.name),
        None => format!("{} · {faction} · unrated", player.name),
    }
}

fn ack_label(ack: Option<Ack>) -> &'static str {
    match ack {
        Some(Ack::Shown) => "display updated",
        Some(Ack::NoOled) => "OLED not reachable",
        Some(Ack::Silent) => "not answering",
        None => "connected",
    }
}

/// The lines and actions for the board on USB. `embedded` is the firmware version this app
/// can install, if it carries one.
fn usb_entries(usb: &UsbLink, embedded: Option<&str>, entries: &mut Vec<Entry>) {
    let state = match (usb.installing, &usb.firmware) {
        (Some(percent), _) => format!("installing firmware… {percent} %"),
        (None, FirmwareState::Missing) => "no display firmware".to_string(),
        (None, FirmwareState::Outdated(None)) => "outdated firmware".to_string(),
        (None, FirmwareState::Outdated(Some(version))) => format!("outdated firmware {version}"),
        (None, FirmwareState::Current(_)) => ack_label(usb.ack).to_string(),
    };
    entries.push(Entry::Label(format!("USB {}: {state}", usb.port)));
    if usb.installing.is_some() {
        return;
    }

    let current = matches!(usb.firmware, FirmwareState::Current(_));
    if current && usb.wifi == Some(BoardWifi::Unset) {
        entries.push(Entry::Label("WiFi on this display is not set up".to_string()));
    }
    match (&usb.firmware, embedded) {
        (FirmwareState::Missing, Some(_)) => entries.push(Entry::Command {
            label: "Install display firmware…".to_string(),
            command: Command::InstallFirmware,
        }),
        (FirmwareState::Outdated(_), Some(version)) => entries.push(Entry::Command {
            label: format!("Update display firmware to {version}…"),
            command: Command::InstallFirmware,
        }),
        _ => {}
    }
    if current {
        entries.push(Entry::Command {
            label: "Set up WiFi on this display…".to_string(),
            command: Command::SetupWifi,
        });
    }
}

pub fn build(
    info: Option<&MatchInfo>,
    links: &LinkStatus,
    settings: &Settings,
    autostart: bool,
    embedded: Option<&str>,
) -> MenuModel {
    let mut entries = vec![Entry::Label(format!("CoH3 Display {}", env!("CARGO_PKG_VERSION"))), Entry::Separator];

    let mut tooltip = match info {
        Some(info) => {
            let ended = match info.phase {
                Phase::Live => "",
                Phase::Lingering => " · ended",
                Phase::Over => " · over",
            };
            let title = format!("{} · {}{ended}", info.mode, map_title(&info.map));
            entries.push(Entry::Label(title.clone()));
            for player in shown_players(info, settings.show_own_team) {
                entries.push(Entry::Player { label: player_label(player), profile_id: player.profile_id });
            }
            if let Some(err) = &info.stats_error {
                entries.push(Entry::Label(format!("Stats unavailable: {err}")));
            }
            format!("CoH3 Display – {title}")
        }
        None => {
            entries.push(Entry::Label("No match yet".to_string()));
            "CoH3 Display – no match yet".to_string()
        }
    };
    entries.push(Entry::Separator);

    if let Some(usb) = &links.usb {
        usb_entries(usb, embedded, &mut entries);
        if let Some(percent) = usb.installing {
            tooltip = format!("CoH3 Display – installing firmware {percent} %");
        }
    }
    for wifi in &links.wifi {
        entries.push(Entry::Label(format!("WiFi {}: {}", wifi.addr, ack_label(wifi.ack))));
    }
    if links.usb.is_none() && links.wifi.is_empty() {
        let none = if settings.usb || settings.wifi { "No display found" } else { "Displays switched off" };
        entries.push(Entry::Label(none.to_string()));
    }
    entries.push(Entry::Separator);

    entries.extend([
        Entry::Check { label: "Send over USB", checked: settings.usb, toggle: Toggle::Usb },
        Entry::Check { label: "Send over WiFi", checked: settings.wifi, toggle: Toggle::Wifi },
        Entry::Check { label: "Show own team too", checked: settings.show_own_team, toggle: Toggle::ShowOwnTeam },
        Entry::Check {
            label: "Update display firmware automatically",
            checked: settings.auto_update_firmware,
            toggle: Toggle::AutoUpdateFirmware,
        },
        Entry::Check {
            label: "Keep last match on display",
            checked: settings.keep_last_match,
            toggle: Toggle::KeepLastMatch,
        },
        Entry::Check { label: "Start with Windows", checked: autostart, toggle: Toggle::Autostart },
        Entry::Separator,
        Entry::OpenLog,
        Entry::Quit,
    ]);

    let acks = links.usb.iter().map(|usb| usb.ack).chain(links.wifi.iter().map(|wifi| wifi.ack));
    let connected = acks.into_iter().any(|ack| ack == Some(Ack::Shown));
    MenuModel { tooltip, connected, entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_link::WifiLink;

    fn usb(firmware: FirmwareState, wifi: Option<BoardWifi>, ack: Option<Ack>) -> UsbLink {
        UsbLink { port: "COM6".to_string(), firmware, wifi, ack, installing: None }
    }

    fn current() -> FirmwareState {
        FirmwareState::Current("0.2.0".to_string())
    }
    use crate::model::PlayerStats;

    fn player(slot: u8, team: u8, name: &str, faction: &str, rating: Option<i64>) -> PlayerInfo {
        PlayerInfo {
            slot,
            name: name.to_string(),
            team,
            faction: faction.to_string(),
            ai: false,
            profile_id: Some(100 + u64::from(slot)),
            country: None,
            stats: rating.map(|rating| PlayerStats { rating, wins: 22, losses: 13, streak: -2 }),
        }
    }

    fn santuario() -> MatchInfo {
        MatchInfo {
            map: "santuario_4p".to_string(),
            mode: "2v2".to_string(),
            match_type: None,
            win_condition: None,
            started_at: None,
            players: vec![
                player(0, 0, "ZOCKER748", "afrika_korps", Some(1006)),
                player(1, 1, "FairyTale", "americans", Some(1215)),
                player(2, 0, "Nullgerät", "germans", Some(1369)),
                player(3, 1, "Senjjoy", "americans", None),
            ],
            local_profile_id: Some(102),
            stats_error: None,
            phase: Phase::Live,
        }
    }

    #[test]
    fn marks_a_match_that_is_no_longer_on() {
        let title = |phase| {
            let info = MatchInfo { phase, ..santuario() };
            labels(&build(Some(&info), &LinkStatus::default(), &Settings::default(), false, None))[1].clone()
        };
        assert_eq!(title(Phase::Live), "2v2 · Santuario");
        assert_eq!(title(Phase::Lingering), "2v2 · Santuario · ended");
        assert_eq!(title(Phase::Over), "2v2 · Santuario · over");
    }

    fn labels(model: &MenuModel) -> Vec<String> {
        model
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Label(label) | Entry::Player { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn titles_maps() {
        assert_eq!(map_title("crossing_in_the_woods_4p"), "Crossing In The Woods");
        assert_eq!(map_title("santuario_4p"), "Santuario");
    }

    #[test]
    fn shows_the_match_and_the_displays() {
        let links = LinkStatus {
            usb: Some(usb(current(), Some(BoardWifi::Up(None)), Some(Ack::Shown))),
            wifi: vec![WifiLink { addr: "10.204.28.172".parse().unwrap(), ack: Some(Ack::Silent) }],
            outcome: None,
        };
        let model = build(Some(&santuario()), &links, &Settings::default(), false, Some("0.2.0"));

        assert_eq!(model.tooltip, "CoH3 Display – 2v2 · Santuario");
        assert!(model.connected);
        assert_eq!(
            labels(&model)[1..],
            [
                "2v2 · Santuario",
                "FairyTale · USF 1215 · 63%",
                "Senjjoy · USF · unrated",
                "USB COM6: display updated",
                "WiFi 10.204.28.172: not answering",
            ]
        );
        assert!(model.entries.contains(&Entry::Command {
            label: "Set up WiFi on this display…".to_string(),
            command: Command::SetupWifi
        }));
        assert!(model.entries.contains(&Entry::Player {
            label: "FairyTale · USF 1215 · 63%".to_string(),
            profile_id: Some(101)
        }));
    }

    #[test]
    fn lists_everyone_when_the_own_team_is_shown() {
        let settings = Settings { show_own_team: true, ..Settings::default() };
        let model = build(Some(&santuario()), &LinkStatus::default(), &settings, false, None);
        let players = model.entries.iter().filter(|entry| matches!(entry, Entry::Player { .. })).count();
        assert_eq!(players, 4);
    }

    #[test]
    fn describes_the_empty_states() {
        let model = build(None, &LinkStatus::default(), &Settings::default(), true, None);
        assert!(!model.connected);
        assert_eq!(labels(&model)[1..], ["No match yet", "No display found"]);
        assert!(model.entries.contains(&Entry::Check {
            label: "Start with Windows",
            checked: true,
            toggle: Toggle::Autostart
        }));

        let off = Settings { usb: false, wifi: false, ..Settings::default() };
        let model = build(None, &LinkStatus::default(), &off, false, None);
        assert_eq!(labels(&model)[2], "Displays switched off");
    }

    fn commands(links: &LinkStatus, embedded: Option<&str>) -> Vec<(String, Command)> {
        build(None, links, &Settings::default(), false, embedded)
            .entries
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Command { label, command } => Some((label, command)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn offers_to_install_or_update_firmware() {
        let with = |link: UsbLink| LinkStatus { usb: Some(link), ..LinkStatus::default() };

        let blank = with(usb(FirmwareState::Missing, None, None));
        let model = build(None, &blank, &Settings::default(), false, Some("0.2.0"));
        assert_eq!(labels(&model)[2], "USB COM6: no display firmware");
        assert_eq!(commands(&blank, Some("0.2.0")), [("Install display firmware…".to_string(), Command::InstallFirmware)]);
        // An app built without firmware has nothing to offer.
        assert_eq!(commands(&blank, None), []);

        let old = with(usb(FirmwareState::Outdated(Some("0.1.0".to_string())), Some(BoardWifi::Connecting), None));
        assert_eq!(
            commands(&old, Some("0.2.0")),
            [("Update display firmware to 0.2.0…".to_string(), Command::InstallFirmware)]
        );

        let fresh = with(usb(current(), Some(BoardWifi::Unset), None));
        let model = build(None, &fresh, &Settings::default(), false, Some("0.2.0"));
        assert!(labels(&model).contains(&"WiFi on this display is not set up".to_string()));
        assert_eq!(commands(&fresh, Some("0.2.0")), [("Set up WiFi on this display…".to_string(), Command::SetupWifi)]);
    }

    #[test]
    fn shows_progress_and_nothing_else_while_installing() {
        let mut link = usb(FirmwareState::Missing, None, None);
        link.installing = Some(42);
        let links = LinkStatus { usb: Some(link), ..LinkStatus::default() };
        let model = build(None, &links, &Settings::default(), false, Some("0.2.0"));
        assert_eq!(labels(&model)[2], "USB COM6: installing firmware… 42 %");
        assert_eq!(model.tooltip, "CoH3 Display – installing firmware 42 %");
        assert_eq!(commands(&links, Some("0.2.0")), []);
    }
}
