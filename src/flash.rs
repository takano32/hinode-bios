//! CFI NOR Flash Bank 0 (`0x0000_0000`) and Bank 1 (`0x0400_0000`) inspection.

use core::ptr::read_volatile;
use crate::{mem, println};

pub const FLASH_BANK0_BASE: usize = 0x0000_0000;
pub const FLASH_BANK0_SIZE: usize = 0x0400_0000; // 64 MB
pub const FLASH_BANK1_BASE: usize = 0x0400_0000;
pub const FLASH_BANK1_SIZE: usize = 0x0400_0000; // 64 MB

#[derive(Clone, Copy)]
pub enum FlashPayloadKind {
    Empty,
    Elf64,
    Pe32PlusEfi,
    LinuxArm64Image,
    RawData(u32),
}

/// Inspect the beginning of Flash Bank 1 (`0x0400_0000`) to see if a payload image is flashed there.
pub fn inspect_bank1() -> FlashPayloadKind {
    let base = FLASH_BANK1_BASE;
    let w0 = unsafe { read_volatile(base as *const u32) };
    if w0 == 0 || w0 == 0xffff_ffff {
        return FlashPayloadKind::Empty;
    }
    // Check ELF magic: 0x7F 'E' 'L' 'F' (little-endian: 0x464C457F)
    if w0 == 0x464c_457f {
        return FlashPayloadKind::Elf64;
    }
    // Check MZ magic for PE32+ EFI: 'M' 'Z' (0x5A4D)
    if (w0 & 0xffff) == 0x5a4d {
        return FlashPayloadKind::Pe32PlusEfi;
    }
    // Check Linux ARM64 Image magic at +0x38: "ARM\x64" (0x644D5241)
    let arm64_magic = unsafe { read_volatile((base + 0x38) as *const u32) };
    if arm64_magic == 0x644d_5241 {
        return FlashPayloadKind::LinuxArm64Image;
    }
    FlashPayloadKind::RawData(w0)
}

pub fn print_info() {
    let rom_bytes = (mem::rom_end() - mem::rom_start()) + (mem::edata() - mem::sdata());
    println!("CFI NOR Flash Controller:");
    println!(
        "  Bank 0 @ 0x{:08X} .. 0x{:08X} (64 MB): Hinode BIOS ROM ({} bytes used)",
        FLASH_BANK0_BASE,
        FLASH_BANK0_BASE + FLASH_BANK0_SIZE,
        rom_bytes
    );
    let bank1_status = match inspect_bank1() {
        FlashPayloadKind::Empty => "Empty (unprogrammed / 0x00000000)",
        FlashPayloadKind::Elf64 => "ELF64 AArch64 Executable Present",
        FlashPayloadKind::Pe32PlusEfi => "PE32+ AArch64 UEFI Application Present",
        FlashPayloadKind::LinuxArm64Image => "Linux ARM64 Kernel Image Present",
        FlashPayloadKind::RawData(_) => "Raw Binary Data Present",
    };
    println!(
        "  Bank 1 @ 0x{:08X} .. 0x{:08X} (64 MB): {}",
        FLASH_BANK1_BASE,
        FLASH_BANK1_BASE + FLASH_BANK1_SIZE,
        bank1_status
    );
}
