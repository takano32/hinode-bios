//! ARM PrimeCell PL061 GPIO (`0x0903_0000`) & QEMU ACPI Generic Event Device (`0x0908_0000`) driver.

use core::ptr::read_volatile;
use crate::dtb;

pub const GPIO_BASE: usize = 0x0903_0000;
pub const ACPI_GED_BASE: usize = 0x0908_0000;

/// Check whether the PL061 GPIO controller is mapped in the active Device Tree.
/// (On QEMU `virt` with `-bios`, QEMU maps `acpi-ged` at `0x0908_0000` unless `acpi=off`.)
pub fn is_pl061_present() -> bool {
    let d = dtb::parse();
    for i in 0..d.device_count {
        if d.devices[i].name.starts_with("pl061@") || d.devices[i].reg_base == GPIO_BASE as u64 {
            return true;
        }
    }
    false
}

/// Read the 32-bit ARM PrimeCell Peripheral ID (`0x00041061` for PL061) if present.
pub fn peripheral_id() -> Option<u32> {
    if !is_pl061_present() {
        return None;
    }
    unsafe {
        let id0 = read_volatile((GPIO_BASE + 0xFE0) as *const u32) & 0xff;
        let id1 = read_volatile((GPIO_BASE + 0xFE4) as *const u32) & 0xff;
        let id2 = read_volatile((GPIO_BASE + 0xFE8) as *const u32) & 0xff;
        let id3 = read_volatile((GPIO_BASE + 0xFEC) as *const u32) & 0xff;
        Some(id0 | (id1 << 8) | (id2 << 16) | (id3 << 24))
    }
}

/// Read the ACPI GED event status register (`0x0908_0000`) when running in `-bios` ACPI mode.
pub fn read_acpi_ged_status() -> u32 {
    if is_pl061_present() {
        0
    } else {
        unsafe { read_volatile(ACPI_GED_BASE as *const u32) }
    }
}

/// Read all 8 GPIO pin levels (`GPIODATA` at `+0x3FC`) if PL061 is present.
pub fn read_pins() -> u8 {
    if !is_pl061_present() {
        return 0;
    }
    unsafe { (read_volatile((GPIO_BASE + 0x3FC) as *const u32) & 0xff) as u8 }
}

/// Read the GPIO direction register (`GPIODIR` at `+0x400`) if PL061 is present.
pub fn read_direction() -> u8 {
    if !is_pl061_present() {
        return 0;
    }
    unsafe { (read_volatile((GPIO_BASE + 0x400) as *const u32) & 0xff) as u8 }
}
