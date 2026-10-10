//! The few Win32 calls a windowed (console-less) program needs.

use std::fs::OpenOptions;
use std::os::windows::io::IntoRawHandle;

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WPARAM,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, DLGTEMPLATE, DS_CENTER, DS_MODALFRAME, DS_SETFONT, DialogBoxIndirectParamW, ES_AUTOHSCROLL,
    ES_PASSWORD, EndDialog, GWLP_USERDATA, GetDlgItemTextW, GetWindowLongPtrW, IDCANCEL, IDOK, IDYES, MB_DEFBUTTON2,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW,
    SetDlgItemTextW, SetForegroundWindow, SetWindowLongPtrW, WM_COMMAND, WM_INITDIALOG, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

fn is_usable(handle: HANDLE) -> bool {
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

/// Makes output show up when the program was started from a terminal. A windowed program
/// starts without standard handles; ones that were redirected to a pipe or file are kept.
pub fn attach_parent_console() {
    // SAFETY: plain Win32 calls on this process's own standard handles.
    unsafe {
        let stdout_missing = !is_usable(GetStdHandle(STD_OUTPUT_HANDLE));
        let stderr_missing = !is_usable(GetStdHandle(STD_ERROR_HANDLE));
        if !(stdout_missing || stderr_missing) || AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let Ok(console) = OpenOptions::new().read(true).write(true).open("CONOUT$") else {
            return;
        };
        // Handed to the process for the rest of its life.
        let handle = console.into_raw_handle() as HANDLE;
        if stdout_missing {
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
        }
        if stderr_missing {
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }
}

/// Returns `false` when another copy of the tracker is already running for this user.
pub fn claim_single_instance() -> bool {
    let name: Vec<u16> = "Local\\coh3-display-tracker\0".encode_utf16().collect();
    // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call. The mutex
    // handle is deliberately never closed: it marks this process as the running instance.
    unsafe {
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        !mutex.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Shows a message and waits for OK.
pub fn message(title: &str, text: &str, is_error: bool) {
    let icon = if is_error { MB_ICONERROR } else { MB_ICONINFORMATION };
    // SAFETY: both strings are NUL-terminated and live until the call returns.
    unsafe {
        MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST);
    }
}

/// Asks a yes/no question; "No" is the default button.
pub fn confirm(title: &str, text: &str) -> bool {
    let style = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST;
    // SAFETY: as in `message`.
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), style) == IDYES }
}

const ID_SSID: i32 = 101;
const ID_PASSWORD: i32 = 102;
// Predefined control classes for dialog templates.
const CLASS_BUTTON: u16 = 0x0080;
const CLASS_EDIT: u16 = 0x0081;
const CLASS_STATIC: u16 = 0x0082;

struct WifiForm {
    ssid: String,
    password: String,
}

/// Appends one control to an in-memory dialog template (a `DLGITEMTEMPLATE`).
fn control(template: &mut Vec<u16>, class: u16, style: u32, rect: [i16; 4], id: i32, text: &str) {
    // Every item starts on a 4-byte boundary.
    if template.len() % 2 != 0 {
        template.push(0);
    }
    let style = style | WS_CHILD | WS_VISIBLE;
    template.extend([style as u16, (style >> 16) as u16, 0, 0]);
    template.extend(rect.map(|value| value as u16));
    template.extend([id as u16, 0xFFFF, class]);
    template.extend(text.encode_utf16().chain(Some(0)));
    template.push(0);
}

unsafe fn dialog_text(dialog: HWND, id: i32) -> String {
    let mut buffer = [0u16; 128];
    // SAFETY: the buffer is as long as the length passed.
    let length = unsafe { GetDlgItemTextW(dialog, id, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length as usize])
}

unsafe extern "system" fn wifi_dialog_proc(dialog: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    // SAFETY: `lparam` on WM_INITDIALOG is the `WifiForm` passed to `DialogBoxIndirectParamW`,
    // which outlives the modal dialog; it is kept in the window's user data for later messages.
    unsafe {
        match message {
            WM_INITDIALOG => {
                SetWindowLongPtrW(dialog, GWLP_USERDATA, lparam);
                let form = &*(lparam as *const WifiForm);
                SetDlgItemTextW(dialog, ID_SSID, wide(&form.ssid).as_ptr());
                SetForegroundWindow(dialog);
                1
            }
            WM_COMMAND => {
                match (wparam & 0xFFFF) as i32 {
                    IDOK => {
                        let form = &mut *(GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut WifiForm);
                        form.ssid = dialog_text(dialog, ID_SSID);
                        form.password = dialog_text(dialog, ID_PASSWORD);
                        EndDialog(dialog, 1);
                    }
                    IDCANCEL => {
                        EndDialog(dialog, 0);
                    }
                    _ => {}
                }
                1
            }
            _ => 0,
        }
    }
}

/// Asks for a WiFi network name and password. `None` if the dialog was cancelled.
pub fn ask_wifi(ssid: &str) -> Option<(String, String)> {
    let frame = (DS_MODALFRAME | DS_CENTER | DS_SETFONT) as u32 | WS_POPUP | WS_CAPTION | WS_SYSMENU;
    // DLGTEMPLATE: style, extended style, item count, position and size in dialog units,
    // no menu, default class, title, then the font because of DS_SETFONT.
    let mut template: Vec<u16> = vec![frame as u16, (frame >> 16) as u16, 0, 0, 6, 0, 0, 220, 96, 0, 0];
    template.extend("Set up WiFi on the display".encode_utf16().chain(Some(0)));
    template.push(9);
    template.extend("Segoe UI".encode_utf16().chain(Some(0)));

    let edit = (ES_AUTOHSCROLL as u32) | WS_BORDER | WS_TABSTOP;
    control(&mut template, CLASS_STATIC, 0, [8, 8, 204, 8], -1, "Network name (SSID):");
    control(&mut template, CLASS_EDIT, edit, [8, 19, 204, 13], ID_SSID, "");
    control(&mut template, CLASS_STATIC, 0, [8, 38, 204, 8], -1, "Password (empty for an open network):");
    control(&mut template, CLASS_EDIT, edit | ES_PASSWORD as u32, [8, 49, 204, 13], ID_PASSWORD, "");
    control(&mut template, CLASS_BUTTON, BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, [108, 74, 50, 14], IDOK, "OK");
    control(&mut template, CLASS_BUTTON, WS_TABSTOP, [162, 74, 50, 14], IDCANCEL, "Cancel");

    // The template must start on a 4-byte boundary, which a `Vec<u16>` does not promise.
    let mut aligned = vec![0u32; template.len().div_ceil(2)];
    let mut form = WifiForm { ssid: ssid.to_string(), password: String::new() };
    // SAFETY: `aligned` holds at least `template.len()` u16 values and is a valid dialog
    // template; `form` outlives the modal call.
    let accepted = unsafe {
        std::ptr::copy_nonoverlapping(template.as_ptr(), aligned.as_mut_ptr().cast::<u16>(), template.len());
        DialogBoxIndirectParamW(
            null_mut(),
            aligned.as_ptr().cast::<DLGTEMPLATE>(),
            null_mut(),
            Some(wifi_dialog_proc),
            &mut form as *mut WifiForm as LPARAM,
        ) == 1
    };
    accepted.then_some((form.ssid, form.password))
}
