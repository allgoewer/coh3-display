//! "Start with Windows": an entry for this exe under the current user's Run key.
//! The installer (`installer/coh3-display.iss`) writes and removes the same value.

use std::io;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "coh3-display";

/// The command line registered for this exe.
fn command() -> io::Result<String> {
    Ok(format!("\"{}\"", std::env::current_exe()?.display()))
}

/// Whether this exe starts at sign-in. An entry left behind by another copy (say, one in a
/// build folder) does not count, so that ticking the menu item repoints it to this one.
pub fn is_enabled() -> bool {
    let registered: io::Result<String> =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).and_then(|key| key.get_value(VALUE));
    match (registered, command()) {
        (Ok(registered), Ok(command)) => registered.eq_ignore_ascii_case(&command),
        _ => false,
    }
}

/// Registers the running exe to start at sign-in, or removes the entry.
pub fn set_enabled(enabled: bool) -> io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
    if enabled {
        key.set_value(VALUE, &command()?)
    } else {
        match key.delete_value(VALUE) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}
