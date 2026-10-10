//! Where the WiFi credentials live in flash.

use esp_storage::FlashStorage;

use crate::credentials::{Credentials, RECORD_SIZE};

/// Start of the `nvs` partition in the default partition table. Nothing else uses it, and
/// installing new firmware does not write there, so the credentials survive updates.
const OFFSET: u32 = 0x9000;

pub fn load(flash: &mut FlashStorage<'_>) -> Option<Credentials> {
    let mut record = [0u8; RECORD_SIZE];
    flash.read(OFFSET, &mut record).ok()?;
    Credentials::decode(&record)
}

/// Stores the credentials, or wipes them with `None`. Returns whether the write succeeded.
pub fn store(flash: &mut FlashStorage<'_>, credentials: Option<&Credentials>) -> bool {
    let record = credentials.map_or([0xFF; RECORD_SIZE], Credentials::encode);
    flash.write(OFFSET, &record).is_ok()
}
