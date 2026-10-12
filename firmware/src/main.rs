#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_storage::FlashStorage;
use firmware::screen::screen_task;
use firmware::serial::serial_task;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.4.0
    // generator parameters: -o esp32c6 -o embassy -o unstable-hal -o esp-backtrace --toolchain stable

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // The WiFi driver allocates.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // XIAO ESP32-C6 antenna switch: GPIO3 low enables it, GPIO14 low selects the on-board antenna.
    let _antenna_switch = Output::new(peripherals.GPIO3, Level::Low, OutputConfig::default());
    let _antenna_select = Output::new(peripherals.GPIO14, Level::Low, OutputConfig::default());

    // XIAO ESP32-C6: D4 = GPIO22 (SDA), D5 = GPIO23 (SCL)
    let i2c = I2c::new(peripherals.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(400)))
        .expect("valid I2C config")
        .with_sda(peripherals.GPIO22)
        .with_scl(peripherals.GPIO23)
        .into_async();
    let usb = UsbSerialJtag::new(peripherals.USB_DEVICE).into_async();

    spawner.spawn(screen_task(i2c).expect("screen task"));
    let mut flash = FlashStorage::new(peripherals.FLASH);
    let credentials = firmware::storage::load(&mut flash);
    spawner.spawn(serial_task(usb, flash).expect("serial task"));
    firmware::net::start(spawner, peripherals.WIFI, credentials);

    core::future::pending().await
}
