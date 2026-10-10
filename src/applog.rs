//! Status messages: to stderr when there is a terminal, and to a log file the tray menu can open.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// `%LOCALAPPDATA%\coh3-display\coh3-display.log`
pub fn path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("coh3-display").join("coh3-display.log"))
}

/// Starts a fresh log file. Without one, messages still go to stderr.
pub fn init() {
    let Some(path) = path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(file) = File::create(path) {
        *FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(file);
    }
}

pub fn log(message: impl AsRef<str>) {
    let message = message.as_ref();
    // A windowed process has no stderr; a failed write there is not worth a panic.
    let _ = writeln!(std::io::stderr(), "{message}");

    if let Some(file) = FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_mut() {
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs()) % 86_400;
        let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        for line in message.lines() {
            let _ = writeln!(file, "{h:02}:{m:02}:{s:02}Z {line}");
        }
        let _ = file.flush();
    }
}
