//! ARM PrimeCell PL031 Real-Time Clock (RTC) driver (`0x0901_0000`).

use core::fmt;
use core::ptr::{read_volatile, write_volatile};

pub const RTC_BASE: usize = 0x0901_0000;

const RTC_DR: *const u32 = (RTC_BASE + 0x000) as *const u32;
const RTC_CR: *mut u32 = (RTC_BASE + 0x00C) as *mut u32;

/// Initialize the PL031 RTC (ensure RTC start bit is set in `RTCCR`).
pub fn init() {
    unsafe {
        write_volatile(RTC_CR, 1);
    }
}

/// Read the 32-bit ARM PrimeCell Peripheral ID (`0x00041031` for PL031).
pub fn peripheral_id() -> u32 {
    unsafe {
        let id0 = read_volatile((RTC_BASE + 0xFE0) as *const u32) & 0xff;
        let id1 = read_volatile((RTC_BASE + 0xFE4) as *const u32) & 0xff;
        let id2 = read_volatile((RTC_BASE + 0xFE8) as *const u32) & 0xff;
        let id3 = read_volatile((RTC_BASE + 0xFEC) as *const u32) & 0xff;
        id0 | (id1 << 8) | (id2 << 16) | (id3 << 24)
    }
}

/// Read the current UNIX epoch timestamp (seconds since 1970-01-01 00:00:00 UTC).
pub fn unix_timestamp() -> u32 {
    unsafe { read_volatile(RTC_DR) }
}

#[derive(Clone, Copy, Debug)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    /// Convert a UNIX timestamp (seconds since 1970-01-01T00:00:00Z) to UTC Gregorian DateTime.
    pub fn from_unix_timestamp(ts: u64) -> Self {
        let second = (ts % 60) as u8;
        let minute = ((ts / 60) % 60) as u8;
        let hour = ((ts / 3600) % 24) as u8;

        // Days since 1970-01-01 (Howard Hinnant civil_from_days algorithm)
        let z = (ts / 86400) as i64 + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u64; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        let year = (y + if m <= 2 { 1 } else { 0 }) as u16;

        Self {
            year,
            month: m as u8,
            day: d as u8,
            hour,
            minute,
            second,
        }
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Read the current UTC date and time from the PL031 RTC.
pub fn now_utc() -> DateTime {
    DateTime::from_unix_timestamp(unix_timestamp() as u64)
}
