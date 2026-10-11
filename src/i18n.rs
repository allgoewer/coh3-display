//! Everything the tray, its dialogs and its message boxes say, in English and German.
//! The log file and the command line stay English.

use crate::display_link::{Ack, OutcomeKind};
use crate::firmware::CredentialError;
use crate::model::Phase;
use crate::settings::Brightness;
use crate::tray_model::Toggle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    De,
}

/// The primary language of a Windows `LANGID` is in its low ten bits.
const LANG_GERMAN: u16 = 0x07;

impl Lang {
    /// From a Windows `LANGID`. Only the primary language counts, so Austria and
    /// Switzerland get German too; every other language gets English.
    pub fn from_langid(langid: u16) -> Self {
        if langid & 0x3ff == LANG_GERMAN { Lang::De } else { Lang::En }
    }

    /// From a language tag as given on the command line: `en`, `de`, `de-AT`, ...
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "de" => Some(Lang::De),
            _ => None,
        }
    }

    pub fn no_match_yet(self) -> &'static str {
        match self {
            Lang::En => "No match yet",
            Lang::De => "Noch kein Match",
        }
    }

    pub fn tooltip_no_match(self) -> &'static str {
        match self {
            Lang::En => "CoH3 Display – no match yet",
            Lang::De => "CoH3 Display – noch kein Match",
        }
    }

    pub fn tooltip_installing(self, percent: u8) -> String {
        match self {
            Lang::En => format!("CoH3 Display – installing firmware {percent} %"),
            Lang::De => format!("CoH3 Display – Firmware wird installiert {percent} %"),
        }
    }

    /// What follows the map in the match title once the match is no longer on.
    pub fn phase_suffix(self, phase: Phase) -> &'static str {
        match (self, phase) {
            (_, Phase::Live) => "",
            (Lang::En, Phase::Lingering) => " · ended",
            (Lang::De, Phase::Lingering) => " · beendet",
            (Lang::En, Phase::Over) => " · over",
            (Lang::De, Phase::Over) => " · vorbei",
        }
    }

    pub fn stats_unavailable(self, error: &str) -> String {
        match self {
            Lang::En => format!("Stats unavailable: {error}"),
            Lang::De => format!("Statistik nicht verfügbar: {error}"),
        }
    }

    pub fn ai(self) -> &'static str {
        match self {
            Lang::En => "AI",
            Lang::De => "KI",
        }
    }

    pub fn unrated(self) -> &'static str {
        match self {
            Lang::En => "unrated",
            Lang::De => "ohne Wertung",
        }
    }

    pub fn ack(self, ack: Option<Ack>) -> &'static str {
        match (self, ack) {
            (Lang::En, Some(Ack::Shown)) => "display updated",
            (Lang::De, Some(Ack::Shown)) => "Display aktualisiert",
            (Lang::En, Some(Ack::NoOled)) => "OLED not reachable",
            (Lang::De, Some(Ack::NoOled)) => "OLED nicht erreichbar",
            (Lang::En, Some(Ack::Silent)) => "not answering",
            (Lang::De, Some(Ack::Silent)) => "antwortet nicht",
            (Lang::En, None) => "connected",
            (Lang::De, None) => "verbunden",
        }
    }

    pub fn installing_firmware(self, percent: u8) -> String {
        match self {
            Lang::En => format!("installing firmware… {percent} %"),
            Lang::De => format!("Firmware wird installiert… {percent} %"),
        }
    }

    pub fn no_firmware(self) -> &'static str {
        match self {
            Lang::En => "no display firmware",
            Lang::De => "keine Display-Firmware",
        }
    }

    pub fn outdated_firmware(self, version: Option<&str>) -> String {
        let outdated = match self {
            Lang::En => "outdated firmware",
            Lang::De => "veraltete Firmware",
        };
        match version {
            Some(version) => format!("{outdated} {version}"),
            None => outdated.to_string(),
        }
    }

    /// The name of the network transport in status lines: `WiFi 10.0.0.7: connected`.
    pub fn wifi(self) -> &'static str {
        match self {
            Lang::En => "WiFi",
            Lang::De => "WLAN",
        }
    }

    pub fn wifi_not_set_up(self) -> &'static str {
        match self {
            Lang::En => "WiFi on this display is not set up",
            Lang::De => "WLAN ist auf diesem Display nicht eingerichtet",
        }
    }

    pub fn install_firmware(self) -> &'static str {
        match self {
            Lang::En => "Install display firmware…",
            Lang::De => "Display-Firmware installieren…",
        }
    }

    pub fn update_firmware(self, version: &str) -> String {
        match self {
            Lang::En => format!("Update display firmware to {version}…"),
            Lang::De => format!("Display-Firmware auf {version} aktualisieren…"),
        }
    }

    pub fn set_up_wifi(self) -> &'static str {
        match self {
            Lang::En => "Set up WiFi on this display…",
            Lang::De => "WLAN auf diesem Display einrichten…",
        }
    }

    pub fn no_display_found(self) -> &'static str {
        match self {
            Lang::En => "No display found",
            Lang::De => "Kein Display gefunden",
        }
    }

    pub fn displays_off(self) -> &'static str {
        match self {
            Lang::En => "Displays switched off",
            Lang::De => "Displays ausgeschaltet",
        }
    }

    pub fn toggle(self, toggle: Toggle) -> &'static str {
        match (self, toggle) {
            (Lang::En, Toggle::Usb) => "Send over USB",
            (Lang::De, Toggle::Usb) => "Über USB senden",
            (Lang::En, Toggle::Wifi) => "Send over WiFi",
            (Lang::De, Toggle::Wifi) => "Über WLAN senden",
            (Lang::En, Toggle::ShowOwnTeam) => "Show own team too",
            (Lang::De, Toggle::ShowOwnTeam) => "Auch eigenes Team anzeigen",
            (Lang::En, Toggle::AutoUpdateFirmware) => "Update display firmware automatically",
            (Lang::De, Toggle::AutoUpdateFirmware) => "Display-Firmware automatisch aktualisieren",
            (Lang::En, Toggle::KeepLastMatch) => "Keep last match on display",
            (Lang::De, Toggle::KeepLastMatch) => "Letztes Match auf dem Display behalten",
            (Lang::En, Toggle::OwnStats) => "Show own ratings in the menu",
            (Lang::De, Toggle::OwnStats) => "Eigene Wertungen im Menü anzeigen",
            (Lang::En, Toggle::Autostart) => "Start with Windows",
            (Lang::De, Toggle::Autostart) => "Mit Windows starten",
        }
    }

    pub fn brightness_menu(self) -> &'static str {
        match self {
            Lang::En => "Display brightness",
            Lang::De => "Display-Helligkeit",
        }
    }

    pub fn brightness(self, brightness: Brightness) -> &'static str {
        match (self, brightness) {
            (Lang::En, Brightness::Low) => "Low",
            (Lang::De, Brightness::Low) => "Niedrig",
            (_, Brightness::Normal) => "Normal",
            (Lang::En, Brightness::High) => "High",
            (Lang::De, Brightness::High) => "Hoch",
        }
    }

    pub fn open_log(self) -> &'static str {
        match self {
            Lang::En => "Open log file",
            Lang::De => "Logdatei öffnen",
        }
    }

    pub fn quit(self) -> &'static str {
        match self {
            Lang::En => "Quit",
            Lang::De => "Beenden",
        }
    }

    pub fn confirm_install(self, version: &str, port: &str) -> String {
        match self {
            Lang::En => format!(
                "Install display firmware {version} on the ESP32 board at {port}?\n\n\
                 Whatever is on that board now will be replaced. Only continue if this is your CoH3 display."
            ),
            Lang::De => format!(
                "Display-Firmware {version} auf dem ESP32-Board an {port} installieren?\n\n\
                 Alles, was jetzt auf diesem Board ist, wird ersetzt. Fahre nur fort, wenn das dein CoH3-Display ist."
            ),
        }
    }

    pub fn wifi_dialog_title(self) -> &'static str {
        match self {
            Lang::En => "Set up WiFi on the display",
            Lang::De => "WLAN auf dem Display einrichten",
        }
    }

    pub fn network_name(self) -> &'static str {
        match self {
            Lang::En => "Network name (SSID):",
            Lang::De => "Netzwerkname (SSID):",
        }
    }

    pub fn password(self) -> &'static str {
        match self {
            Lang::En => "Password (empty for an open network):",
            Lang::De => "Passwort (leer für ein offenes Netzwerk):",
        }
    }

    pub fn cancel(self) -> &'static str {
        match self {
            Lang::En => "Cancel",
            Lang::De => "Abbrechen",
        }
    }

    pub fn outcome(self, outcome: &OutcomeKind) -> String {
        match (self, outcome) {
            (Lang::En, OutcomeKind::WifiStored { ssid }) => {
                format!("WiFi settings stored. The display is joining \"{ssid}\".")
            }
            (Lang::De, OutcomeKind::WifiStored { ssid }) => {
                format!("WLAN-Einstellungen gespeichert. Das Display verbindet sich mit „{ssid}“.")
            }
            (Lang::En, OutcomeKind::WifiFailed { error }) => format!("Could not set up WiFi: {error}"),
            (Lang::De, OutcomeKind::WifiFailed { error }) => {
                format!("WLAN konnte nicht eingerichtet werden: {error}")
            }
            (Lang::En, OutcomeKind::FirmwareInstalled { port }) => format!("Display firmware installed on {port}."),
            (Lang::De, OutcomeKind::FirmwareInstalled { port }) => format!("Display-Firmware auf {port} installiert."),
            (Lang::En, OutcomeKind::FirmwareFailed { port, error }) => {
                format!("Installing the firmware on {port} failed: {error}")
            }
            (Lang::De, OutcomeKind::FirmwareFailed { port, error }) => {
                format!("Die Installation der Firmware auf {port} ist fehlgeschlagen: {error}")
            }
        }
    }

    pub fn credential_error(self, error: CredentialError) -> &'static str {
        match (self, error) {
            (Lang::En, CredentialError::SsidLength) => "the network name must be 1 to 32 bytes long",
            (Lang::De, CredentialError::SsidLength) => "Der Netzwerkname muss 1 bis 32 Bytes lang sein.",
            (Lang::En, CredentialError::PasswordTooLong) => "the password must be at most 63 bytes long",
            (Lang::De, CredentialError::PasswordTooLong) => "Das Passwort darf höchstens 63 Bytes lang sein.",
            (Lang::En, CredentialError::PasswordTooShort) => {
                "a WiFi password has at least 8 characters (leave it empty for an open network)"
            }
            (Lang::De, CredentialError::PasswordTooShort) => {
                "Ein WLAN-Passwort hat mindestens 8 Zeichen (für ein offenes Netzwerk leer lassen)."
            }
            (Lang::En, CredentialError::ControlCharacters) => {
                "network name and password must not contain tabs or other control characters"
            }
            (Lang::De, CredentialError::ControlCharacters) => {
                "Netzwerkname und Passwort dürfen keine Tabulatoren oder anderen Steuerzeichen enthalten."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_windows_gets_german_and_everything_else_english() {
        // de-DE, de-CH, de-AT
        for langid in [0x0407, 0x0807, 0x0C07] {
            assert_eq!(Lang::from_langid(langid), Lang::De);
        }
        // en-US, fr-FR, and "nothing known"
        for langid in [0x0409, 0x040C, 0] {
            assert_eq!(Lang::from_langid(langid), Lang::En);
        }
    }

    #[test]
    fn reads_language_tags() {
        assert_eq!(Lang::from_tag("de"), Some(Lang::De));
        assert_eq!(Lang::from_tag("de-AT"), Some(Lang::De));
        assert_eq!(Lang::from_tag("EN"), Some(Lang::En));
        assert_eq!(Lang::from_tag("fr"), None);
    }
}
