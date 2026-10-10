//! The WiFi credentials record kept in flash.
//!
//! Only `core` and `heapless`, so the tracker's tests can include this file directly.

use heapless::String;

pub const RECORD_SIZE: usize = 128;
const MAGIC: &[u8; 4] = b"C3W1";
const SSID_MAX: usize = 32;
const PASSWORD_MAX: usize = 64;
/// Magic, two lengths, both fields; the CRC follows.
const BODY: usize = 4 + 2 + SSID_MAX + PASSWORD_MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub ssid: String<SSID_MAX>,
    /// Empty for an open network.
    pub password: String<PASSWORD_MAX>,
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

impl Credentials {
    /// Builds credentials from the fields of a `W` line. `None` if the SSID is empty or
    /// either value is too long.
    pub fn new(ssid: &str, password: &str) -> Option<Self> {
        if ssid.is_empty() {
            return None;
        }
        let mut credentials = Self { ssid: String::new(), password: String::new() };
        credentials.ssid.push_str(ssid).ok()?;
        credentials.password.push_str(password).ok()?;
        Some(credentials)
    }

    pub fn encode(&self) -> [u8; RECORD_SIZE] {
        let mut record = [0u8; RECORD_SIZE];
        record[..4].copy_from_slice(MAGIC);
        record[4] = self.ssid.len() as u8;
        record[5] = self.password.len() as u8;
        record[6..6 + self.ssid.len()].copy_from_slice(self.ssid.as_bytes());
        let password_at = 6 + SSID_MAX;
        record[password_at..password_at + self.password.len()].copy_from_slice(self.password.as_bytes());
        let crc = crc32(&record[..BODY]);
        record[BODY..BODY + 4].copy_from_slice(&crc.to_le_bytes());
        record
    }

    /// Reads a record back. Erased flash, a damaged record or anything else gives `None`.
    pub fn decode(record: &[u8]) -> Option<Self> {
        if record.len() < BODY + 4 || &record[..4] != MAGIC {
            return None;
        }
        let stored_crc = u32::from_le_bytes(record[BODY..BODY + 4].try_into().ok()?);
        if stored_crc != crc32(&record[..BODY]) {
            return None;
        }
        let (ssid_len, password_len) = (usize::from(record[4]), usize::from(record[5]));
        let password_at = 6 + SSID_MAX;
        let ssid = core::str::from_utf8(record.get(6..6 + ssid_len)?).ok()?;
        let password = core::str::from_utf8(record.get(password_at..password_at + password_len)?).ok()?;
        if ssid_len > SSID_MAX || password_len > PASSWORD_MAX {
            return None;
        }
        Self::new(ssid, password)
    }
}
