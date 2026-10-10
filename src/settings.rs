//! Settings the tray menu can change, kept in `%APPDATA%\coh3-display\settings.toml`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How bright the display shows a match. Idle is always as dim as the panel goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brightness {
    Low,
    #[default]
    Normal,
    High,
}

impl Brightness {
    pub const ALL: [Brightness; 3] = [Brightness::Low, Brightness::Normal, Brightness::High];

    /// The level as the display protocol numbers it.
    pub fn level(self) -> u8 {
        match self {
            Brightness::Low => 1,
            Brightness::Normal => 2,
            Brightness::High => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Send matches to a display on a USB serial port.
    pub usb: bool,
    /// Look for displays on the network and send matches to them.
    pub wifi: bool,
    /// Show every player instead of only the opposing team.
    pub show_own_team: bool,
    /// Update a display that runs older firmware without asking. A board with no display
    /// firmware at all is never flashed without a confirmation.
    pub auto_update_firmware: bool,
    /// Keep the last match on the display after it is over. For trying things out without
    /// a game running; costs OLED lifetime.
    pub keep_last_match: bool,
    /// How bright the display shows a match.
    pub brightness: Brightness,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            usb: true,
            wifi: true,
            show_own_team: false,
            auto_update_firmware: true,
            keep_last_match: false,
            brightness: Brightness::Normal,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("coh3-display").join("settings.toml"))
    }

    /// Loads the settings; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("coh3-display-{}-{name}", std::process::id())).join("settings.toml")
    }

    #[test]
    fn round_trips_through_the_file() {
        let path = temp_file("roundtrip");
        let settings = Settings {
            usb: false,
            wifi: true,
            show_own_team: true,
            auto_update_firmware: false,
            keep_last_match: true,
            brightness: Brightness::Low,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_or_broken_files_give_defaults() {
        assert_eq!(Settings::load(&temp_file("missing")), Settings::default());

        let path = temp_file("broken");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "usb = \"maybe\"").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unknown_and_missing_keys_are_tolerated() {
        let settings: Settings = toml::from_str("wifi = false\nfuture_option = 3\n").unwrap();
        // A settings file from before a setting existed gets that setting's default.
        assert_eq!(settings, Settings { wifi: false, ..Settings::default() });
    }
}
