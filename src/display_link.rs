//! Sends the current match to the ESP32-C6 display, over its USB serial port (this module)
//! and over WiFi ([`udp`]). The wire format is in `PROTOCOL.md`.

pub mod udp;

use std::fmt::Write as _;
use std::io;
use std::net::IpAddr;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{SerialPort, SerialPortType};
use tokio::sync::watch;

use crate::applog::log;
use crate::firmware::{self, BoardWifi, FirmwareState};
use crate::i18n::Lang;
use crate::maps;
use crate::model::{Faction, MatchInfo, Phase, PlayerInfo};
use crate::settings::Settings;

/// USB-Serial-JTAG of Espressif chips.
const USB_ID: (u16, u16) = (0x303A, 0x1001);
const MAX_PLAYERS: usize = 8;
const MAX_TEXT: usize = 24;
/// Map titles get more room: the longest is "Überquerung des Rapido-Flusses".
pub const MAX_MAP: usize = 32;
const RESEND: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_secs(2);
/// How often a link that is switched off checks whether it was switched on again.
const DISABLED_POLL: Duration = Duration::from_millis(500);
/// How often the board on USB is asked what it runs and how its WiFi is doing.
const PROBE_INTERVAL: Duration = Duration::from_secs(10);

/// A board on a USB serial port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbLink {
    pub port: String,
    pub firmware: FirmwareState,
    /// What the board says about its WiFi; `None` when it runs no display firmware.
    pub wifi: Option<BoardWifi>,
    /// `None` until a frame was sent.
    pub ack: Option<Ack>,
    /// Percent done while firmware is being installed.
    pub installing: Option<u8>,
}

/// What the tray menu can ask the USB link to do.
#[derive(Debug)]
pub enum LinkCommand {
    InstallFirmware,
    SetWifi { ssid: String, password: String },
}

/// How a [`LinkCommand`] ended, to be shown to the user once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Increases with every outcome, so a new one can be told from the last.
    pub id: u32,
    pub kind: OutcomeKind,
}

/// What happened; the wording is in [`crate::i18n`]. `error` is the cause, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeKind {
    WifiStored { ssid: String },
    WifiFailed { error: String },
    FirmwareInstalled { port: String },
    FirmwareFailed { port: String, error: String },
}

impl OutcomeKind {
    pub fn ok(&self) -> bool {
        matches!(self, OutcomeKind::WifiStored { .. } | OutcomeKind::FirmwareInstalled { .. })
    }
}

/// A display found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiLink {
    pub addr: IpAddr,
    pub ack: Option<Ack>,
}

/// The displays currently in reach, for the tray menu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkStatus {
    pub usb: Option<UsbLink>,
    pub wifi: Vec<WifiLink>,
    pub outcome: Option<Outcome>,
}

/// What a link thread works with: the match to show, the live settings, and where to
/// report the displays it sees.
#[derive(Clone)]
pub struct LinkContext {
    pub matches: watch::Receiver<Option<MatchInfo>>,
    pub settings: watch::Receiver<Settings>,
    pub status: Arc<watch::Sender<LinkStatus>>,
    /// The language of what the display shows: the map's title.
    pub lang: Lang,
}

/// Why a link's serve loop ended without an error.
enum Stop {
    /// The tracker is shutting down.
    Closed,
    /// The link was switched off in the settings.
    Disabled,
    /// The port must be released so firmware can be installed through it.
    Install,
}

/// Cuts text down to what the display can show: no control characters, Latin-1 only.
fn sanitize(text: &str) -> String {
    sanitize_to(text, MAX_TEXT)
}

fn sanitize_to(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).map(|c| if (c as u32) <= 0xFF { c } else { '?' }).take(max).collect()
}

/// `alamein_4p` -> `alamein`
pub fn short_map(map: &str) -> &str {
    match map.rsplit_once('_') {
        Some((name, suffix))
            if suffix.strip_suffix('p').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            name
        }
        _ => map,
    }
}

fn faction_code(player: &PlayerInfo) -> char {
    match Faction::from_log(&player.faction) {
        Some(Faction::American) => 'A',
        Some(Faction::British) => 'B',
        Some(Faction::Dak) => 'D',
        Some(Faction::German) => 'G',
        None => '?',
    }
}

/// The players worth showing: the opposing team, or everyone when asked to or when the local
/// player is not in the match (a replay).
pub fn shown_players(info: &MatchInfo, show_own_team: bool) -> Vec<&PlayerInfo> {
    let own_team = info
        .local_profile_id
        .filter(|_| !show_own_team)
        .and_then(|id| info.players.iter().find(|p| p.profile_id == Some(id)))
        .map(|me| me.team);
    let mut players: Vec<&PlayerInfo> = info.players.iter().filter(|p| Some(p.team) != own_team).collect();
    players.sort_by_key(|p| (p.team, p.slot));
    players
}

/// Encodes a match as one protocol frame.
pub fn encode(info: &MatchInfo, show_own_team: bool, lang: Lang) -> String {
    let map = sanitize_to(&maps::title(&info.map, lang), MAX_MAP);
    let mut frame = format!("M\t{}\t{map}\n", sanitize(&info.mode));

    for player in shown_players(info, show_own_team).into_iter().take(MAX_PLAYERS) {
        let stats = match &player.stats {
            Some(stats) => format!("{}\t{}\t{}\t{}", stats.rating, stats.wins, stats.losses, stats.streak),
            None => "-\t-\t-\t-".to_string(),
        };
        let country = player.country.as_deref().map(sanitize).filter(|c| !c.is_empty());
        let _ = writeln!(
            frame,
            "P\t{}\t{stats}\t{}\t{}",
            faction_code(player),
            country.as_deref().unwrap_or("-"),
            sanitize(&player.name)
        );
    }
    frame.push_str("E\n");
    frame
}

/// The line that sends the display into its idle mode.
pub const IDLE_FRAME: &str = "I\n";

/// What the display should be told right now: the match while it is worth showing,
/// otherwise to go idle.
pub fn frame_for(info: Option<&MatchInfo>, settings: &Settings, lang: Lang) -> String {
    match info {
        Some(info) if info.phase != Phase::Over || settings.keep_last_match => {
            encode(info, settings.show_own_team, lang)
        }
        _ => IDLE_FRAME.to_string(),
    }
}

/// The first connected ESP USB serial port, if any.
pub fn detect_port() -> Option<String> {
    serialport::available_ports().ok()?.into_iter().find_map(|port| match port.port_type {
        SerialPortType::UsbPort(usb) if (usb.vid, usb.pid) == USB_ID => Some(port.port_name),
        _ => None,
    })
}

/// Opens a display's serial port.
pub fn open(port_name: &str) -> serialport::Result<Box<dyn SerialPort>> {
    // DTR/RTS stay low: toggling them resets the chip through the USB-Serial-JTAG bridge.
    serialport::new(port_name, 115_200)
        .timeout(Duration::from_millis(100))
        .dtr_on_open(false)
        .open()
}

/// What the device answered to a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    /// Frame received and shown.
    Shown,
    /// Frame received, but the board could not reach its OLED over I2C.
    NoOled,
    /// No answer at all.
    Silent,
}

impl Ack {
    pub fn describe(self) -> &'static str {
        match self {
            Ack::Shown => "display updated",
            Ack::NoOled => "board answered, but it cannot reach its OLED (check wiring on D4/D5)",
            Ack::Silent => "board is not answering",
        }
    }
}

/// Writes a frame and waits briefly for the device's answer.
fn send_frame(port: &mut dyn SerialPort, frame: &str) -> io::Result<Ack> {
    port.write_all(frame.as_bytes())?;
    port.flush()?;

    let deadline = Instant::now() + Duration::from_millis(800);
    let mut reply = Vec::new();
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(n) => {
                reply.extend_from_slice(&buf[..n]);
                if reply.windows(3).any(|w| w == b"OK\n") {
                    return Ok(Ack::Shown);
                }
                if reply.windows(4).any(|w| w == b"ERR ") {
                    return Ok(Ack::NoOled);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err),
        }
    }
    Ok(Ack::Silent)
}

/// Sends one match to the display and reports what it answered.
pub fn send_once(
    port_name: Option<&str>,
    info: &MatchInfo,
    show_own_team: bool,
    lang: Lang,
) -> anyhow::Result<Ack> {
    let port_name = match port_name {
        Some(name) => name.to_string(),
        None => detect_port().ok_or_else(|| anyhow::anyhow!("none found"))?,
    };
    let mut port = open(&port_name)?;
    Ok(send_frame(port.as_mut(), &encode(info, show_own_team, lang))?)
}

fn publish_usb(ctx: &LinkContext, link: Option<UsbLink>) {
    ctx.status.send_if_modified(|status| {
        let changed = status.usb != link;
        if changed {
            status.usb = link;
        }
        changed
    });
}

/// Tells the user, through the tray, how a command ended.
fn publish_outcome(ctx: &LinkContext, kind: OutcomeKind) {
    log(Lang::En.outcome(&kind));
    ctx.status.send_modify(|status| {
        let id = status.outcome.as_ref().map_or(1, |last| last.id + 1);
        status.outcome = Some(Outcome { id, kind });
    });
}

/// Talks to the board on an open port until the link is closed, switched off, or has to
/// let go of the port. Only a board with current firmware is sent matches.
fn serve(
    port: &mut dyn SerialPort,
    port_name: &str,
    ctx: &mut LinkContext,
    commands: &mpsc::Receiver<LinkCommand>,
    auto_update_tried: &mut bool,
) -> io::Result<Stop> {
    let embedded = firmware::embedded().map(|firmware| firmware.version);
    let mut link = UsbLink {
        port: port_name.to_string(),
        firmware: FirmwareState::Missing,
        wifi: None,
        ack: None,
        installing: None,
    };
    let mut last_sent: Option<Instant> = None;
    let mut last_probe: Option<Instant> = None;
    loop {
        let Ok(match_changed) = ctx.matches.has_changed() else {
            return Ok(Stop::Closed);
        };
        let settings_changed = ctx.settings.has_changed().unwrap_or(false);
        let settings = *ctx.settings.borrow_and_update();
        if !settings.usb {
            return Ok(Stop::Disabled);
        }

        if last_probe.is_none_or(|at| at.elapsed() >= PROBE_INTERVAL) {
            let status = firmware::probe_present(port, port_name)?;
            let state = firmware::classify(status.as_ref(), embedded);
            if state != link.firmware || last_probe.is_none() {
                log(format!("Board on {port_name}: {state:?}"));
            }
            link.firmware = state;
            link.wifi = status.map(|status| status.wifi);
            publish_usb(ctx, Some(link.clone()));
            last_probe = Some(Instant::now());

            match &link.firmware {
                FirmwareState::Current(_) => *auto_update_tried = false,
                // One attempt per outdated board, so a failing update does not repeat forever.
                // A board without display firmware is left for the user to confirm.
                FirmwareState::Outdated(_)
                    if settings.auto_update_firmware && embedded.is_some() && !*auto_update_tried =>
                {
                    *auto_update_tried = true;
                    log(format!("Updating the display firmware on {port_name} automatically"));
                    return Ok(Stop::Install);
                }
                _ => {}
            }
        }

        match commands.try_recv() {
            Ok(LinkCommand::InstallFirmware) => return Ok(Stop::Install),
            Ok(LinkCommand::SetWifi { ssid, password }) => {
                match firmware::set_wifi(port, &ssid, &password) {
                    Ok(()) => publish_outcome(ctx, OutcomeKind::WifiStored { ssid }),
                    Err(err) => publish_outcome(ctx, OutcomeKind::WifiFailed { error: format!("{err:#}") }),
                }
                // The board restarts; ask again soon what it is doing.
                last_probe = Instant::now().checked_sub(PROBE_INTERVAL - Duration::from_secs(3));
            }
            Err(_) => {}
        }

        let is_display = matches!(link.firmware, FirmwareState::Current(_));
        if is_display && (match_changed || settings_changed || last_sent.is_none_or(|at| at.elapsed() >= RESEND)) {
            let frame = frame_for(ctx.matches.borrow_and_update().as_ref(), &settings, ctx.lang);
            let ack = send_frame(port, &frame)?;
            if link.ack != Some(ack) {
                log(format!("Display on {port_name}: {}", ack.describe()));
                link.ack = Some(ack);
            }
            publish_usb(ctx, Some(link.clone()));
            last_sent = Some(Instant::now());
            if ack == Ack::NoOled {
                // Usually a board that has only just started; ask again in a second.
                last_sent = Instant::now().checked_sub(RESEND - Duration::from_secs(1));
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Installs the embedded firmware through `port_name`, which must be closed, and reports
/// progress and the result to the tray.
fn install_firmware(port_name: &str, ctx: &LinkContext) {
    let mut link = UsbLink {
        port: port_name.to_string(),
        firmware: FirmwareState::Missing,
        wifi: None,
        ack: None,
        installing: Some(0),
    };
    publish_usb(ctx, Some(link.clone()));
    log(format!("Installing firmware on {port_name}"));
    let result = firmware::install(port_name, &mut |percent| {
        if link.installing != Some(percent) {
            link.installing = Some(percent);
            publish_usb(ctx, Some(link.clone()));
        }
    });
    match result {
        Ok(()) => publish_outcome(ctx, OutcomeKind::FirmwareInstalled { port: port_name.to_string() }),
        Err(err) => publish_outcome(
            ctx,
            OutcomeKind::FirmwareFailed { port: port_name.to_string(), error: format!("{err:#}") },
        ),
    }
}

/// Starts the background thread for the board on USB. With `port_name` unset the port is
/// auto-detected, also again after the board was unplugged.
pub fn spawn(
    port_name: Option<String>,
    mut ctx: LinkContext,
    commands: mpsc::Receiver<LinkCommand>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_problem = String::new();
        let mut report = |problem: String| {
            if problem != last_problem {
                log(&problem);
                last_problem = problem;
            }
        };
        let mut auto_update_tried = false;
        loop {
            if ctx.matches.has_changed().is_err() {
                return;
            }
            if !ctx.settings.borrow().usb {
                // Commands for a link that is switched off are dropped, not kept for later.
                while commands.try_recv().is_ok() {}
                thread::sleep(DISABLED_POLL);
                continue;
            }
            let mut retry_now = false;
            match port_name.clone().or_else(detect_port) {
                None => {
                    while commands.try_recv().is_ok() {}
                    report("No USB display found, will keep looking".to_string());
                }
                Some(name) => match open(&name) {
                    Ok(mut port) => match serve(port.as_mut(), &name, &mut ctx, &commands, &mut auto_update_tried) {
                        Ok(Stop::Closed) => return,
                        Ok(Stop::Disabled) => report(format!("USB display on {name} switched off")),
                        Ok(Stop::Install) => {
                            drop(port);
                            install_firmware(&name, &ctx);
                            retry_now = true;
                        }
                        Err(err) => report(format!("Lost display on {name}: {err}")),
                    },
                    Err(err) => report(format!("Could not open display on {name}: {err}")),
                },
            }
            publish_usb(&ctx, None);
            if ctx.settings.borrow().usb && !retry_now {
                thread::sleep(RETRY);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PlayerStats;

    fn player(slot: u8, team: u8, name: &str, faction: &str, stats: Option<(i64, u32, u32, i32)>) -> PlayerInfo {
        PlayerInfo {
            slot,
            name: name.to_string(),
            team,
            faction: faction.to_string(),
            ai: false,
            profile_id: Some(100 + u64::from(slot)),
            country: stats.map(|_| "de".to_string()),
            stats: stats.map(|(rating, wins, losses, streak)| PlayerStats { rating, wins, losses, streak }),
        }
    }

    fn info(players: Vec<PlayerInfo>) -> MatchInfo {
        MatchInfo {
            map: "alamein_4p".to_string(),
            mode: "2v2".to_string(),
            match_type: None,
            win_condition: None,
            started_at: None,
            players,
            local_profile_id: None,
            stats_error: None,
            phase: Phase::Live,
        }
    }

    #[test]
    fn sends_idle_instead_of_a_match_that_is_over() {
        let settings = Settings::default();
        let mut info = two_v_two();
        assert_eq!(frame_for(None, &settings, Lang::En), "I\n");
        assert!(frame_for(Some(&info), &settings, Lang::En).starts_with("M\t2v2\tEl Alamein\n"));
        info.phase = Phase::Lingering;
        assert!(frame_for(Some(&info), &settings, Lang::En).starts_with("M\t"));
        info.phase = Phase::Over;
        assert_eq!(frame_for(Some(&info), &settings, Lang::En), "I\n");

        let keep = Settings { keep_last_match: true, ..settings };
        assert!(frame_for(Some(&info), &keep, Lang::En).starts_with("M\t"));
    }

    fn two_v_two() -> MatchInfo {
        info(vec![
            player(0, 0, "Nullgerät", "germans", Some((1369, 44, 28, -1))),
            player(1, 1, "Von_Dolofon", "americans", Some((1284, 164, 135, 3))),
            player(2, 0, "ZOCKER748", "afrika_korps", None),
            player(3, 1, "Saulek", "british_africa", None),
        ])
    }

    #[test]
    fn sends_only_the_opposing_team() {
        let mut info = two_v_two();
        info.local_profile_id = Some(100);
        assert_eq!(
            encode(&info, false, Lang::En),
            "M\t2v2\tEl Alamein\n\
             P\tA\t1284\t164\t135\t3\tde\tVon_Dolofon\n\
             P\tB\t-\t-\t-\t-\t-\tSaulek\n\
             E\n"
        );

        info.local_profile_id = Some(101);
        let names: Vec<_> = shown_players(&info, false).iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Nullgerät", "ZOCKER748"]);
    }

    #[test]
    fn sends_everyone_when_asked_to_show_the_own_team() {
        let mut info = two_v_two();
        info.local_profile_id = Some(100);
        assert_eq!(shown_players(&info, true).len(), 4);
        assert_eq!(encode(&info, true, Lang::En).lines().filter(|l| l.starts_with('P')).count(), 4);
    }

    #[test]
    fn sends_everyone_when_the_local_player_is_not_in_the_match() {
        let mut info = two_v_two();
        assert_eq!(shown_players(&info, false).len(), 4);
        info.local_profile_id = Some(999);
        let names: Vec<_> = shown_players(&info, false).iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Nullgerät", "ZOCKER748", "Von_Dolofon", "Saulek"]);
    }

    #[test]
    fn names_are_made_safe_for_the_display() {
        let players = vec![player(0, 0, "Tab\there Paweł and a very long tail", "frontend", None)];
        let frame = encode(&info(players), false, Lang::En);
        assert_eq!(frame.lines().nth(1).unwrap(), "P\t?\t-\t-\t-\t-\t-\tTabhere Pawe? and a very");
    }

    #[test]
    fn sends_the_maps_title_in_the_apps_language() {
        let rapido = MatchInfo { map: "rapido_river_8p".to_string(), ..two_v_two() };
        assert!(encode(&rapido, false, Lang::En).starts_with("M\t2v2\tRapido River Crossing\n"));
        let frame = encode(&rapido, false, Lang::De);
        assert!(frame.starts_with("M\t2v2\tÜberquerung des Rapido-Flusses\n"));

        // The longest title there is, with a two-byte character, fits the firmware's field.
        let mut parser = device::Parser::new();
        let parsed = frame.lines().find_map(|line| parser.feed_line(line));
        let Some(device::Frame::Match(parsed)) = parsed else {
            panic!("frame ends with E, got {parsed:?}");
        };
        assert_eq!(parsed.map.as_str(), "Überquerung des Rapido-Flusses");

        let unknown = MatchInfo { map: "some_new_map_6p".to_string(), ..two_v_two() };
        assert!(encode(&unknown, false, Lang::De).starts_with("M\t2v2\tSome New Map\n"));
    }

    #[test]
    fn only_strips_player_count_suffixes_from_maps() {
        assert_eq!(short_map("crossing_in_the_woods_4p"), "crossing_in_the_woods");
        assert_eq!(short_map("pachino_farmlands_mkii_8p"), "pachino_farmlands_mkii");
        assert_eq!(short_map("twin_beach"), "twin_beach");
        assert_eq!(short_map("map_p"), "map_p");
    }

    /// The firmware's parser, compiled into the host tests to keep both ends in step.
    #[path = "../../../firmware/src/protocol.rs"]
    #[allow(dead_code)]
    mod device;

    #[test]
    fn firmware_parser_understands_encoded_frames() {
        let frame = encode(&two_v_two(), false, Lang::En);

        let mut parser = device::Parser::new();
        let mut parsed = None;
        for line in frame.lines() {
            assert!(line.len() <= device::MAX_LINE);
            parsed = parser.feed_line(line);
        }
        let Some(device::Frame::Match(parsed)) = parsed else {
            panic!("frame ends with E, got {parsed:?}");
        };

        // The idle line is a frame of its own, and it drops a half-sent match.
        let mut parser = device::Parser::new();
        assert_eq!(parser.feed_line("M\t2v2\talamein"), None);
        assert_eq!(parser.feed_line(IDLE_FRAME.trim_end()), Some(device::Frame::Idle));
        assert_eq!(parser.feed_line("E"), None);
        assert_eq!(device::parse_datagram(IDLE_FRAME.as_bytes()), Some(device::Frame::Idle));

        assert_eq!((parsed.mode.as_str(), parsed.map.as_str()), ("2v2", "El Alamein"));
        assert_eq!(parsed.players.len(), 4);
        let rated = &parsed.players[0];
        assert_eq!((rated.faction, rated.elo, rated.name.as_str()), ('G', Some(1369), "Nullgerät"));
        assert_eq!((rated.wins, rated.losses, rated.streak), (Some(44), Some(28), Some(-1)));
        assert_eq!((rated.winrate(), rated.country.as_str()), (Some(61), "de"));
        let unrated = &parsed.players[1];
        assert_eq!((unrated.elo, unrated.winrate(), unrated.country.as_str()), (None, None, ""));
        assert_eq!(unrated.name.as_str(), "ZOCKER748");
    }

    #[test]
    fn sends_at_most_eight_players() {
        let players = (0..10).map(|i| player(i, i % 2, "x", "germans", None)).collect();
        assert_eq!(encode(&info(players), false, Lang::En).lines().filter(|l| l.starts_with('P')).count(), 8);
    }
}
