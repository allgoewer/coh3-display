//! Whether the game itself is running. The log tells when it was started and closed, but a
//! game that crashed leaves no closing line.

#[cfg(windows)]
const EXE: &str = "RelicCoH3.exe";

/// Looks for the game among the running processes.
#[cfg(windows)]
pub fn is_running() -> bool {
    process_exists(EXE)
}

#[cfg(windows)]
fn process_exists(exe: &str) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };

    // SAFETY: the snapshot handle is checked before use and closed afterwards; `entry` is a
    // plain struct with its size filled in, as the two calls that write to it require.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more && !found {
            let length = entry.szExeFile.iter().position(|&unit| unit == 0).unwrap_or(entry.szExeFile.len());
            found = String::from_utf16_lossy(&entry.szExeFile[..length]).eq_ignore_ascii_case(exe);
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
pub fn is_running() -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn finds_a_process_by_the_name_of_its_exe() {
        let this = std::env::current_exe().unwrap();
        let name = this.file_name().unwrap().to_str().unwrap();
        assert!(process_exists(name));
        assert!(process_exists(&name.to_uppercase()));
        assert!(!process_exists("no-such-program-coh3-display.exe"));
    }
}
