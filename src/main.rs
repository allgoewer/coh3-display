// No console window when started by double-click or at sign-in.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod win;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::watch;

use coh3_display::applog::{self, log};
use coh3_display::display_link::{self, LinkContext, LinkStatus};
use coh3_display::firmware::{self, BoardWifi, FirmwareState};
use coh3_display::i18n::Lang;
use coh3_display::log_parser::LogEvent;
use coh3_display::model::Phase;
use coh3_display::relic::RelicClient;
use coh3_display::render::render;
use coh3_display::settings::Settings;
use coh3_display::watcher::LogTail;

/// Shows the players of the latest Company of Heroes 3 match with their ELO.
#[derive(Parser)]
struct Args {
    /// Path to warnings.log [default: Documents\My Games\Company of Heroes 3\warnings.log]
    #[arg(long)]
    log: Option<PathBuf>,

    /// Print the latest match and exit instead of following the log
    #[arg(long)]
    once: bool,

    /// Serial port of the display, e.g. COM6 [default: auto-detect]
    #[arg(long)]
    serial: Option<String>,

    /// Address of a WiFi display, for networks where it is not discovered automatically
    #[arg(long)]
    display_addr: Option<IpAddr>,

    /// Do not use the display's USB serial port in this run
    #[arg(long)]
    no_serial: bool,

    /// Do not look for displays on the network in this run
    #[arg(long)]
    no_wifi: bool,

    /// Do not send matches to any display in this run
    #[arg(long)]
    no_display: bool,

    /// Run without the tray icon
    #[arg(long)]
    no_tray: bool,

    /// Language of the tray menu and of map names, en or de [default: the Windows display language]
    #[arg(long, value_parser = parse_lang)]
    lang: Option<Lang>,

    /// Seconds the opponents stay on the display after leaving a match (for testing)
    #[arg(long, hide = true)]
    linger_secs: Option<u64>,

    /// Show what the display on USB is running, then exit
    #[arg(long)]
    board_status: bool,

    /// Install the firmware carried by this app on the display on USB, then exit
    #[arg(long)]
    install_firmware: bool,

    /// With --install-firmware: do not ask before overwriting the board
    #[arg(long)]
    yes: bool,

    /// Store WiFi credentials on the display on USB, then exit; the password is read from standard input
    #[arg(long, value_name = "SSID")]
    set_wifi: Option<String>,
}

fn parse_lang(tag: &str) -> Result<Lang, String> {
    Lang::from_tag(tag).ok_or_else(|| "expected en or de".to_string())
}

/// Opens the board and asks what it runs, giving a freshly reset one time to come back.
fn probe_board(port_name: &str) -> Result<Option<firmware::BoardStatus>> {
    let mut last_error = None;
    for _ in 0..10 {
        match display_link::open(port_name) {
            Ok(mut port) => return Ok(firmware::probe_present(port.as_mut(), port_name)?),
            Err(err) => last_error = Some(err),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(last_error.map_or_else(|| anyhow::anyhow!("could not open {port_name}"), Into::into))
}

fn describe_board(port_name: &str, status: Option<&firmware::BoardStatus>) -> String {
    let embedded = firmware::embedded().map(|firmware| firmware.version);
    let firmware = match firmware::classify(status, embedded) {
        FirmwareState::Missing => "no display firmware".to_string(),
        FirmwareState::Outdated(None) => "firmware older than 0.2.0".to_string(),
        FirmwareState::Outdated(Some(version)) => format!("firmware {version} (outdated)"),
        FirmwareState::Current(version) => format!("firmware {version}"),
    };
    let wifi = match status.map(|status| &status.wifi) {
        None => String::new(),
        Some(BoardWifi::Unset) => ", WiFi not set up".to_string(),
        Some(BoardWifi::Off) => ", WiFi off".to_string(),
        Some(BoardWifi::Connecting) => ", WiFi connecting".to_string(),
        Some(BoardWifi::Up(Some(ip))) => format!(", WiFi up at {ip}"),
        Some(BoardWifi::Up(None)) => ", WiFi up".to_string(),
    };
    format!("{port_name}: {firmware}{wifi}")
}

/// The one-shot commands that work on the board itself. Returns whether one was run.
fn run_board_command(args: &Args) -> Result<bool> {
    if !(args.board_status || args.install_firmware || args.set_wifi.is_some()) {
        return Ok(false);
    }
    let port_name = args.serial.clone().or_else(display_link::detect_port).context("no display found on USB")?;

    if args.install_firmware {
        let embedded = firmware::embedded().context("this build of the app carries no firmware")?;
        if !args.yes {
            eprint!("Install display firmware {} on {port_name}, replacing what is on it? [y/N] ", embedded.version);
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !answer.trim().eq_ignore_ascii_case("y") {
                eprintln!("Nothing installed.");
                return Ok(true);
            }
        }
        let mut shown = 0;
        firmware::install(&port_name, &mut |percent| {
            if percent >= shown + 10 || percent == 100 && shown != 100 {
                shown = percent;
                eprintln!("Installing firmware… {percent} %");
            }
        })?;
        eprintln!("Firmware {} installed.", embedded.version);
    }

    if let Some(ssid) = &args.set_wifi {
        let mut password = String::new();
        std::io::stdin().read_line(&mut password)?;
        let password = password.trim_end_matches(['\r', '\n']);
        // The board may still be starting after an install.
        probe_board(&port_name)?;
        firmware::set_wifi(display_link::open(&port_name)?.as_mut(), ssid, password)?;
        eprintln!("WiFi settings stored; the display is restarting.");
        // Joining the network takes a few seconds.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_secs(1));
            if let Ok(Some(status)) = probe_board(&port_name) {
                if matches!(status.wifi, BoardWifi::Up(_) | BoardWifi::Off) {
                    break;
                }
            }
        }
    }

    println!("{}", describe_board(&port_name, probe_board(&port_name)?.as_ref()));
    Ok(true)
}

fn default_log_path() -> Result<PathBuf> {
    let documents = dirs::document_dir().context("could not locate the Documents folder, pass --log")?;
    Ok(documents.join("My Games").join("Company of Heroes 3").join("warnings.log"))
}

/// Prints the latest match, sends it to the displays once and returns.
async fn run_once(args: &Args, mut tail: LogTail, client: RelicClient, settings: Settings, lang: Lang) -> Result<()> {
    let latest = tail
        .poll()
        .with_context(|| format!("could not read {}", tail.path().display()))?
        .into_iter()
        .rev()
        .find_map(|event| match event {
            LogEvent::Started(logged) => Some(logged),
            _ => None,
        })
        .with_context(|| format!("no match found in {}", tail.path().display()))?;
    let info = client.enrich(&latest).await;
    print!("{}", render(&info));

    if settings.usb {
        let (own_team, brightness) = (settings.show_own_team, settings.brightness);
        match display_link::send_once(args.serial.as_deref(), &info, own_team, lang, brightness) {
            Ok(ack) => eprintln!("Display on USB: {}", ack.describe()),
            Err(err) => eprintln!("Display on USB: {err:#}"),
        }
    }
    if settings.wifi {
        let wait = Duration::from_secs(5);
        let (own_team, brightness) = (settings.show_own_team, settings.brightness);
        match display_link::udp::send_once(args.display_addr, &info, own_team, lang, brightness, wait) {
            Ok(answers) if answers.is_empty() => eprintln!("Display on WiFi: none found"),
            Ok(answers) => {
                for (addr, ack) in answers {
                    eprintln!("Display at {addr}: {}", ack.describe());
                }
            }
            Err(err) => eprintln!("Display on WiFi: {err}"),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    #[cfg(windows)]
    win::attach_parent_console();

    let args = Args::parse();
    if run_board_command(&args)? {
        return Ok(());
    }
    let path = match &args.log {
        Some(path) => path.clone(),
        None => default_log_path()?,
    };
    let client = RelicClient::new()?;
    let tail = LogTail::new(&path);
    #[cfg(windows)]
    let lang = args.lang.unwrap_or_else(win::ui_language);
    #[cfg(not(windows))]
    let lang = args.lang.unwrap_or_default();

    let settings_path = Settings::path();
    let saved = settings_path.as_deref().map(Settings::load).unwrap_or_default();
    // Flags switch a transport off for this run only; they are never written back.
    let settings = Settings {
        usb: saved.usb && !args.no_serial && !args.no_display,
        wifi: saved.wifi && !args.no_wifi && !args.no_display,
        ..saved
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    if args.once {
        return runtime.block_on(run_once(&args, tail, client, settings, lang));
    }

    #[cfg(windows)]
    if !win::claim_single_instance() {
        eprintln!("coh3-display is already running (look for its icon in the notification area)");
        return Ok(());
    }

    applog::init();
    log(format!("Watching {}", path.display()));

    let (match_tx, matches) = watch::channel(None);
    let (own_tx, own) = watch::channel(None);
    let (settings_tx, settings_rx) = watch::channel(settings);
    let (status_tx, links) = watch::channel(LinkStatus::default());
    let settings_tx = Arc::new(settings_tx);
    let ctx = LinkContext { matches: matches.clone(), own, settings: settings_rx, status: Arc::new(status_tx), lang };
    let (commands, command_rx) = mpsc::channel();
    display_link::spawn(args.serial.clone(), ctx.clone(), command_rx);
    display_link::udp::spawn(args.display_addr, ctx);

    let linger = args.linger_secs.map_or(coh3_display::LINGER, Duration::from_secs);
    runtime.spawn(coh3_display::track(tail, client, match_tx, own_tx, Duration::from_secs(1), linger));
    let mut printed = matches.clone();
    runtime.spawn(async move {
        let mut last_match = None;
        while printed.changed().await.is_ok() {
            if let Some(info) = printed.borrow_and_update().as_ref() {
                // The same match comes round again when its phase changes.
                let identity = (info.map.clone(), info.started_at.clone());
                if last_match.as_ref() != Some(&identity) {
                    last_match = Some(identity);
                    log(render(info));
                }
                log(match info.phase {
                    Phase::Live => "Match is on",
                    Phase::Lingering => "Match was left; the display keeps it for a while",
                    Phase::Over => "Match is over; the display goes idle",
                });
            }
        }
    });

    #[cfg(windows)]
    if !args.no_tray {
        let ctx = tray::TrayContext { matches, links, settings: settings_tx, saved, settings_path, commands, lang };
        return tray::run(ctx, runtime.handle());
    }

    // Headless: keep the channels alive and run until killed.
    let _keep = (matches, links, settings_tx, commands);
    runtime.block_on(std::future::pending::<()>());
    Ok(())
}
