#![no_std]

pub mod credentials;
pub mod net;
pub mod protocol;
pub mod screen;
pub mod serial;
pub mod storage;

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

/// Latest frame received from the host, handed from the serial and UDP tasks to the screen task.
pub static FRAME: Signal<CriticalSectionRawMutex, protocol::Frame> = Signal::new();

/// Set once the OLED answered on I2C, so the host can be told when it did not.
pub static DISPLAY_READY: AtomicBool = AtomicBool::new(false);

/// The answer to a received frame, the same on every transport.
pub fn frame_reply() -> &'static [u8] {
    if DISPLAY_READY.load(Ordering::Relaxed) { b"OK\n" } else { b"ERR display\n" }
}
