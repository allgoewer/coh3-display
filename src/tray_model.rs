//! What the tray menu shows, worked out from the app's state without touching any GUI.

use crate::display_link::{Ack, LinkStatus, UsbLink, shown_players};
use crate::firmware::{BoardWifi, FirmwareState};
use crate::i18n::Lang;
use crate::maps;
use crate::model::{Faction, MatchInfo, PlayerInfo};
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
fn player_label(player: &PlayerInfo, lang: Lang) -> String {
    let faction = faction_label(player);
    match &player.stats {
        Some(stats) => match stats.winrate() {
            Some(rate) => format!("{} · {faction} {} · {:.0}%", player.name, stats.rating, rate * 100.0),
            None => format!("{} · {faction} {}", player.name, stats.rating),
        },
        None if player.ai => format!("{} · {faction} · {}", player.name, lang.ai()),
        None => format!("{} · {faction} · {}", player.name, lang.unrated()),
    }
}

/// The lines and actions for the board on USB. `embedded` is the firmware version this app
/// can install, if it carries one.
fn usb_entries(usb: &UsbLink, embedded: Option<&str>, lang: Lang, entries: &mut Vec<Entry>) {
    let state = match (usb.installing, &usb.firmware) {
        (Some(percent), _) => lang.installing_firmware(percent),
        (None, FirmwareState::Missing) => lang.no_firmware().to_string(),
        (None, FirmwareState::Outdated(version)) => lang.outdated_firmware(version.as_deref()),
        (None, FirmwareState::Current(_)) => lang.ack(usb.ack).to_string(),
    };
    entries.push(Entry::Label(format!("USB {}: {state}", usb.port)));
    if usb.installing.is_some() {
        return;
    }

    let current = matches!(usb.firmware, FirmwareState::Current(_));
    if current && usb.wifi == Some(BoardWifi::Unset) {
        entries.push(Entry::Label(lang.wifi_not_set_up().to_string()));
    }
    match (&usb.firmware, embedded) {
        (FirmwareState::Missing, Some(_)) => entries.push(Entry::Command {
            label: lang.install_firmware().to_string(),
            command: Command::InstallFirmware,
        }),
        (FirmwareState::Outdated(_), Some(version)) => entries.push(Entry::Command {
            label: lang.update_firmware(version),
            command: Command::InstallFirmware,
        }),
        _ => {}
    }
    if current {
        entries.push(Entry::Command { label: lang.set_up_wifi().to_string(), command: Command::SetupWifi });
    }
}

pub fn build(
    info: Option<&MatchInfo>,
    links: &LinkStatus,
    settings: &Settings,
    autostart: bool,
    embedded: Option<&str>,
    lang: Lang,
) -> MenuModel {
    let mut entries = vec![Entry::Label(format!("CoH3 Display {}", env!("CARGO_PKG_VERSION"))), Entry::Separator];

    let mut tooltip = match info {
        Some(info) => {
            let map = maps::title(&info.map, lang);
            let title = format!("{} · {map}{}", info.mode, lang.phase_suffix(info.phase));
            entries.push(Entry::Label(title.clone()));
            for player in shown_players(info, settings.show_own_team) {
                entries.push(Entry::Player { label: player_label(player, lang), profile_id: player.profile_id });
            }
            if let Some(err) = &info.stats_error {
                entries.push(Entry::Label(lang.stats_unavailable(err)));
            }
            format!("CoH3 Display – {title}")
        }
        None => {
            entries.push(Entry::Label(lang.no_match_yet().to_string()));
            lang.tooltip_no_match().to_string()
        }
    };
    entries.push(Entry::Separator);

    if let Some(usb) = &links.usb {
        usb_entries(usb, embedded, lang, &mut entries);
        if let Some(percent) = usb.installing {
            tooltip = lang.tooltip_installing(percent);
        }
    }
    for wifi in &links.wifi {
        entries.push(Entry::Label(format!("{} {}: {}", lang.wifi(), wifi.addr, lang.ack(wifi.ack))));
    }
    if links.usb.is_none() && links.wifi.is_empty() {
        let none = if settings.usb || settings.wifi { lang.no_display_found() } else { lang.displays_off() };
        entries.push(Entry::Label(none.to_string()));
    }
    entries.push(Entry::Separator);

    let check = |toggle, checked| Entry::Check { label: lang.toggle(toggle), checked, toggle };
    entries.extend([
        check(Toggle::Usb, settings.usb),
        check(Toggle::Wifi, settings.wifi),
        check(Toggle::ShowOwnTeam, settings.show_own_team),
        check(Toggle::AutoUpdateFirmware, settings.auto_update_firmware),
        check(Toggle::KeepLastMatch, settings.keep_last_match),
        check(Toggle::Autostart, autostart),
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
    use crate::model::{Phase, PlayerStats};

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
            labels(&build(Some(&info), &LinkStatus::default(), &Settings::default(), false, None, Lang::En))[1].clone()
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
    fn shows_the_match_and_the_displays() {
        let links = LinkStatus {
            usb: Some(usb(current(), Some(BoardWifi::Up(None)), Some(Ack::Shown))),
            wifi: vec![WifiLink { addr: "10.204.28.172".parse().unwrap(), ack: Some(Ack::Silent) }],
            outcome: None,
        };
        let model = build(Some(&santuario()), &links, &Settings::default(), false, Some("0.2.0"), Lang::En);

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
        let model = build(Some(&santuario()), &LinkStatus::default(), &settings, false, None, Lang::En);
        let players = model.entries.iter().filter(|entry| matches!(entry, Entry::Player { .. })).count();
        assert_eq!(players, 4);
    }

    #[test]
    fn describes_the_empty_states() {
        let model = build(None, &LinkStatus::default(), &Settings::default(), true, None, Lang::En);
        assert!(!model.connected);
        assert_eq!(labels(&model)[1..], ["No match yet", "No display found"]);
        assert!(model.entries.contains(&Entry::Check {
            label: "Start with Windows",
            checked: true,
            toggle: Toggle::Autostart
        }));

        let off = Settings { usb: false, wifi: false, ..Settings::default() };
        let model = build(None, &LinkStatus::default(), &off, false, None, Lang::En);
        assert_eq!(labels(&model)[2], "Displays switched off");
    }

    #[test]
    fn speaks_german() {
        let links = LinkStatus {
            usb: Some(usb(current(), Some(BoardWifi::Unset), Some(Ack::Shown))),
            wifi: vec![WifiLink { addr: "10.204.28.172".parse().unwrap(), ack: Some(Ack::Silent) }],
            outcome: None,
        };
        let info = MatchInfo { map: "rural_town_4p".to_string(), phase: Phase::Lingering, ..santuario() };
        let model = build(Some(&info), &links, &Settings::default(), false, Some("0.2.0"), Lang::De);

        assert_eq!(model.tooltip, "CoH3 Display – 2v2 · Pachino-Ackerland · beendet");
        assert_eq!(
            labels(&model)[1..],
            [
                "2v2 · Pachino-Ackerland · beendet",
                "FairyTale · USF 1215 · 63%",
                "Senjjoy · USF · ohne Wertung",
                "USB COM6: Display aktualisiert",
                "WLAN ist auf diesem Display nicht eingerichtet",
                "WLAN 10.204.28.172: antwortet nicht",
            ]
        );
        assert!(model.entries.contains(&Entry::Check {
            label: "Mit Windows starten",
            checked: false,
            toggle: Toggle::Autostart
        }));

        let model = build(None, &LinkStatus::default(), &Settings::default(), false, None, Lang::De);
        assert_eq!(model.tooltip, "CoH3 Display – noch kein Match");
        assert_eq!(labels(&model)[1..], ["Noch kein Match", "Kein Display gefunden"]);
    }

    fn commands(links: &LinkStatus, embedded: Option<&str>) -> Vec<(String, Command)> {
        build(None, links, &Settings::default(), false, embedded, Lang::En)
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
        let model = build(None, &blank, &Settings::default(), false, Some("0.2.0"), Lang::En);
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
        let model = build(None, &fresh, &Settings::default(), false, Some("0.2.0"), Lang::En);
        assert!(labels(&model).contains(&"WiFi on this display is not set up".to_string()));
        assert_eq!(commands(&fresh, Some("0.2.0")), [("Set up WiFi on this display…".to_string(), Command::SetupWifi)]);
    }

    #[test]
    fn shows_progress_and_nothing_else_while_installing() {
        let mut link = usb(FirmwareState::Missing, None, None);
        link.installing = Some(42);
        let links = LinkStatus { usb: Some(link), ..LinkStatus::default() };
        let model = build(None, &links, &Settings::default(), false, Some("0.2.0"), Lang::En);
        assert_eq!(labels(&model)[2], "USB COM6: installing firmware… 42 %");
        assert_eq!(model.tooltip, "CoH3 Display – installing firmware 42 %");
        assert_eq!(commands(&links, Some("0.2.0")), []);
    }
}
