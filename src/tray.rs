//! The notification-area icon and its menu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, mpsc};

use anyhow::Result;
use tokio::sync::watch;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

use coh3_display::applog::{self, log};
use coh3_display::autostart;
use coh3_display::display_link::{LinkCommand, LinkStatus};
use coh3_display::firmware::{self, BoardWifi, FirmwareState};
use coh3_display::i18n::Lang;
use coh3_display::icon;
use coh3_display::model::MatchInfo;
use coh3_display::settings::{Brightness, Settings};
use coh3_display::tray_model::{self, Command as BoardCommand, Entry, MenuModel, Toggle};
use coh3_display::update;

use crate::win;

const TITLE: &str = "CoH3 Display";

pub struct TrayContext {
    pub matches: watch::Receiver<Option<MatchInfo>>,
    pub links: watch::Receiver<LinkStatus>,
    /// The settings the link threads follow.
    pub settings: Arc<watch::Sender<Settings>>,
    /// What is on disk: the same, minus this run's command-line overrides.
    pub saved: Settings,
    pub settings_path: Option<PathBuf>,
    /// Requests to the thread that owns the display's USB port.
    pub commands: mpsc::Sender<LinkCommand>,
    /// The language of the menu and its dialogs.
    pub lang: Lang,
    /// A newer version of the app, once the check at startup has found one.
    pub update: watch::Receiver<Option<String>>,
}

enum UserEvent {
    Menu(MenuEvent),
    /// Something the menu shows has changed.
    Refresh,
}

enum Action {
    Toggle(Toggle),
    Brightness(Brightness),
    Board(BoardCommand),
    OpenProfile(u64),
    OpenDownload,
    OpenLog,
    Quit,
}

/// The app's icon, lit when a display is showing the match and grey otherwise. Drawn at the
/// size the notification area uses, so Windows does not have to shrink it.
fn icon(connected: bool) -> Option<Icon> {
    let size = win::small_icon_size();
    Icon::from_rgba(icon::render(size, connected), size, size).ok()
}

/// Opens a file or URL with its default program, without flashing a console window.
fn open(target: &str) {
    if let Err(err) = Command::new("explorer.exe").arg(target).spawn() {
        log(format!("Could not open {target}: {err}"));
    }
}

struct App {
    ctx: TrayContext,
    tray: Option<TrayIcon>,
    actions: HashMap<MenuId, Action>,
    shown: Option<MenuModel>,
    /// The last command result that was shown, so each is shown once.
    last_outcome: u32,
    /// Firmware was just installed from here; offer WiFi setup once the board is back.
    offer_wifi: bool,
}

impl App {
    fn build_menu(&mut self, model: &MenuModel) -> Menu {
        let menu = Menu::new();
        self.actions.clear();
        for (index, entry) in model.entries.iter().enumerate() {
            let id = MenuId::new(index.to_string());
            // A single `&` would be taken as a keyboard mnemonic.
            let escape = |label: &str| label.replace('&', "&&");
            let _ = match entry {
                Entry::Separator => menu.append(&PredefinedMenuItem::separator()),
                Entry::Label(label) => menu.append(&MenuItem::with_id(id, escape(label), false, None)),
                Entry::Player { label, profile_id } => {
                    if let Some(profile_id) = profile_id {
                        self.actions.insert(id.clone(), Action::OpenProfile(*profile_id));
                    }
                    menu.append(&MenuItem::with_id(id, escape(label), profile_id.is_some(), None))
                }
                Entry::Check { label, checked, toggle } => {
                    self.actions.insert(id.clone(), Action::Toggle(*toggle));
                    menu.append(&CheckMenuItem::with_id(id, label, true, *checked, None))
                }
                Entry::Group { label, checks } => {
                    let submenu = Submenu::new(label, true);
                    for (option, (label, checked, toggle)) in checks.iter().enumerate() {
                        let id = MenuId::new(format!("{index}.{option}"));
                        self.actions.insert(id.clone(), Action::Toggle(*toggle));
                        let _ = submenu.append(&CheckMenuItem::with_id(id, label, true, *checked, None));
                    }
                    menu.append(&submenu)
                }
                Entry::Brightness { label, options, current } => {
                    let submenu = Submenu::new(label, true);
                    for (option, (label, brightness)) in options.iter().enumerate() {
                        let id = MenuId::new(format!("{index}.{option}"));
                        self.actions.insert(id.clone(), Action::Brightness(*brightness));
                        let _ = submenu.append(&CheckMenuItem::with_id(id, label, true, brightness == current, None));
                    }
                    menu.append(&submenu)
                }
                Entry::Command { label, command } => {
                    self.actions.insert(id.clone(), Action::Board(*command));
                    menu.append(&MenuItem::with_id(id, escape(label), true, None))
                }
                Entry::Update { label } => {
                    self.actions.insert(id.clone(), Action::OpenDownload);
                    menu.append(&MenuItem::with_id(id, escape(label), true, None))
                }
                Entry::OpenLog => {
                    self.actions.insert(id.clone(), Action::OpenLog);
                    menu.append(&MenuItem::with_id(id, self.ctx.lang.open_log(), true, None))
                }
                Entry::Quit => {
                    self.actions.insert(id.clone(), Action::Quit);
                    menu.append(&MenuItem::with_id(id, self.ctx.lang.quit(), true, None))
                }
            };
        }
        menu
    }

    /// Brings icon, tooltip and menu in line with the current state.
    fn refresh(&mut self) {
        let model = tray_model::build(
            self.ctx.matches.borrow().as_ref(),
            &self.ctx.links.borrow(),
            &self.ctx.settings.borrow(),
            autostart::is_enabled(),
            firmware::embedded().map(|firmware| firmware.version),
            self.ctx.update.borrow().as_deref(),
            self.ctx.lang,
        );
        if self.shown.as_ref() == Some(&model) {
            return;
        }
        let menu = self.build_menu(&model);

        match &self.tray {
            Some(tray) => {
                tray.set_menu(Some(Box::new(menu)));
                let _ = tray.set_tooltip(Some(&model.tooltip));
                if self.shown.as_ref().map(|shown| shown.connected) != Some(model.connected) {
                    let _ = tray.set_icon(icon(model.connected));
                }
            }
            None => {
                let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(&model.tooltip);
                if let Some(icon) = icon(model.connected) {
                    builder = builder.with_icon(icon);
                }
                match builder.build() {
                    Ok(tray) => self.tray = Some(tray),
                    Err(err) => log(format!("Could not create the tray icon: {err}")),
                }
            }
        }
        self.shown = Some(model);
    }

    /// Shows the result of the last board command, and offers WiFi setup on a board that
    /// just got its firmware. Both open modal windows, so no channel may be borrowed here.
    fn follow_up(&mut self) {
        let (outcome, usb) = {
            let links = self.ctx.links.borrow();
            (links.outcome.clone(), links.usb.clone())
        };
        if let Some(outcome) = outcome.filter(|outcome| outcome.id != self.last_outcome) {
            self.last_outcome = outcome.id;
            let ok = outcome.kind.ok();
            win::message(TITLE, &self.ctx.lang.outcome(&outcome.kind), !ok);
            if !ok {
                self.offer_wifi = false;
            }
        }
        let fresh = usb.is_some_and(|usb| {
            usb.installing.is_none()
                && matches!(usb.firmware, FirmwareState::Current(_))
                && usb.wifi == Some(BoardWifi::Unset)
        });
        if self.offer_wifi && fresh {
            self.offer_wifi = false;
            self.setup_wifi();
        }
    }

    fn install_firmware(&mut self) {
        let Some(firmware) = firmware::embedded() else {
            return;
        };
        let Some(port) = self.ctx.links.borrow().usb.as_ref().map(|usb| usb.port.clone()) else {
            return;
        };
        let question = self.ctx.lang.confirm_install(firmware.version, &port);
        if win::confirm(TITLE, &question) && self.ctx.commands.send(LinkCommand::InstallFirmware).is_ok() {
            self.offer_wifi = true;
        }
    }

    fn setup_wifi(&mut self) {
        let mut ssid = String::new();
        loop {
            let Some((entered_ssid, password)) = win::ask_wifi(&ssid, self.ctx.lang) else {
                return;
            };
            ssid = entered_ssid.trim().to_string();
            match firmware::check_credentials(&ssid, &password) {
                Ok(()) => {
                    let _ = self.ctx.commands.send(LinkCommand::SetWifi { ssid, password });
                    return;
                }
                Err(err) => win::message(TITLE, self.ctx.lang.credential_error(err), true),
            }
        }
    }

    fn toggle(&mut self, toggle: Toggle) {
        let flip = |settings: &mut Settings| match toggle {
            Toggle::Usb => settings.usb = !settings.usb,
            Toggle::Wifi => settings.wifi = !settings.wifi,
            Toggle::ShowOwnTeam => settings.show_own_team = !settings.show_own_team,
            Toggle::AutoUpdateFirmware => settings.auto_update_firmware = !settings.auto_update_firmware,
            Toggle::KeepLastMatch => settings.keep_last_match = !settings.keep_last_match,
            Toggle::OwnRating(mode) => settings.own_ratings.toggle(mode),
            Toggle::Autostart => {}
        };
        if toggle == Toggle::Autostart {
            if let Err(err) = autostart::set_enabled(!autostart::is_enabled()) {
                log(format!("Could not change the autostart entry: {err}"));
            }
            return;
        }

        self.ctx.settings.send_modify(flip);
        // The click decides the saved value too, whatever a command-line flag set for this run.
        let effective = *self.ctx.settings.borrow();
        match toggle {
            Toggle::Usb => self.ctx.saved.usb = effective.usb,
            Toggle::Wifi => self.ctx.saved.wifi = effective.wifi,
            Toggle::ShowOwnTeam => self.ctx.saved.show_own_team = effective.show_own_team,
            Toggle::AutoUpdateFirmware => self.ctx.saved.auto_update_firmware = effective.auto_update_firmware,
            Toggle::KeepLastMatch => self.ctx.saved.keep_last_match = effective.keep_last_match,
            Toggle::OwnRating(_) => self.ctx.saved.own_ratings = effective.own_ratings,
            Toggle::Autostart => {}
        }
        self.save_settings();
    }

    fn set_brightness(&mut self, brightness: Brightness) {
        self.ctx.settings.send_modify(|settings| settings.brightness = brightness);
        self.ctx.saved.brightness = brightness;
        self.save_settings();
    }

    fn save_settings(&self) {
        if let Some(path) = &self.ctx.settings_path {
            if let Err(err) = self.ctx.saved.save(path) {
                log(format!("Could not save {}: {err}", path.display()));
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        self.refresh();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if let UserEvent::Menu(event) = event {
            match self.actions.get(&event.id) {
                Some(Action::Toggle(toggle)) => {
                    let toggle = *toggle;
                    self.toggle(toggle);
                }
                Some(Action::Brightness(brightness)) => {
                    let brightness = *brightness;
                    self.set_brightness(brightness);
                }
                Some(Action::Board(BoardCommand::InstallFirmware)) => self.install_firmware(),
                Some(Action::Board(BoardCommand::SetupWifi)) => self.setup_wifi(),
                Some(Action::OpenProfile(profile_id)) => open(&format!("https://coh3stats.com/players/{profile_id}")),
                Some(Action::OpenDownload) => open(update::DOWNLOAD_URL),
                Some(Action::OpenLog) => match applog::path() {
                    Some(path) => open(&path.display().to_string()),
                    None => log("There is no log file"),
                },
                Some(Action::Quit) => {
                    event_loop.exit();
                    return;
                }
                None => {}
            }
            // A clicked check item has flipped on screen; rebuild it from the real state.
            self.shown = None;
        }
        self.refresh();
        self.follow_up();
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window: WindowId, _event: WindowEvent) {}
}

/// Shows the tray icon and blocks until "Quit" is chosen.
pub fn run(ctx: TrayContext, runtime: &tokio::runtime::Handle) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    // Wake the event loop whenever something the menu shows has changed.
    let proxy = event_loop.create_proxy();
    let (mut matches, mut links, mut settings) = (ctx.matches.clone(), ctx.links.clone(), ctx.settings.subscribe());
    runtime.spawn(async move {
        loop {
            let alive = tokio::select! {
                changed = matches.changed() => changed.is_ok(),
                changed = links.changed() => changed.is_ok(),
                changed = settings.changed() => changed.is_ok(),
            };
            if !alive || proxy.send_event(UserEvent::Refresh).is_err() {
                break;
            }
        }
    });

    // The update check answers once, if at all; after that its channel closes for good.
    let proxy = event_loop.create_proxy();
    let mut update = ctx.update.clone();
    runtime.spawn(async move {
        if update.changed().await.is_ok() {
            let _ = proxy.send_event(UserEvent::Refresh);
        }
    });

    let mut app = App { ctx, tray: None, actions: HashMap::new(), shown: None, last_outcome: 0, offer_wifi: false };
    event_loop.run_app(&mut app)?;
    Ok(())
}
