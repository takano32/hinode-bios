//! AArch64 architectural register access, CPU identification, and cache maintenance.

use core::arch::asm;

#[inline(always)]
pub fn current_el() -> u8 {
    let el: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) el, options(nomem, nostack, preserves_flags));
    }
    ((el >> 2) & 0x3) as u8
}

#[inline(always)]
pub fn mpidr_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, mpidr_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn cpu_id() -> u64 {
    mpidr_el1() & 0x00ff_ffff
}

#[inline(always)]
pub fn midr_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, midr_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn sctlr_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, sctlr_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn vbar_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, vbar_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn cpacr_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, cpacr_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn daif() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, daif", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn sp() -> u64 {
    let val: u64;
    unsafe {
        asm!("mov {}, sp", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn id_aa64mmfr0_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, id_aa64mmfr0_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn id_aa64pfr0_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, id_aa64pfr0_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn id_aa64isar0_el1() -> u64 {
    let val: u64;
    unsafe {
        asm!("mrs {}, id_aa64isar0_el1", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn enable_irq() {
    unsafe {
        asm!("msr daifclr, #2", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn disable_irq() {
    unsafe {
        asm!("msr daifset, #2", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn dsb_sy() {
    unsafe {
        asm!("dsb sy", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn isb() {
    unsafe {
        asm!("isb", options(nostack, preserves_flags));
    }
}

/// Clean data cache and invalidate instruction cache for a loaded code region.
pub fn sync_icache_range(start: usize, len: usize) {
    let line_size = 64usize;
    let mut addr = start & !(line_size - 1);
    let end = start.saturating_add(len);
    unsafe {
        while addr < end {
            asm!("dc cvau, {}", in(reg) addr, options(nostack, preserves_flags));
            addr += line_size;
        }
        asm!("dsb ish", options(nostack, preserves_flags));
        addr = start & !(line_size - 1);
        while addr < end {
            asm!("ic ivau, {}", in(reg) addr, options(nostack, preserves_flags));
            addr += line_size;
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

pub struct CpuInfo {
    pub implementer: u8,
    pub variant: u8,
    pub architecture: u8,
    pub part_num: u16,
    pub revision: u8,
    pub implementer_name: &'static str,
    pub part_name: &'static str,
    pub pa_bits: u8,
}

pub fn decode_cpu_info() -> CpuInfo {
    let midr = midr_el1();
    let implementer = ((midr >> 24) & 0xff) as u8;
    let variant = ((midr >> 20) & 0xf) as u8;
    let architecture = ((midr >> 16) & 0xf) as u8;
    let part_num = ((midr >> 4) & 0xfff) as u16;
    let revision = (midr & 0xf) as u8;

    let implementer_name = match implementer {
        0x41 => "ARM Limited",
        0x42 => "Broadcom",
        0x43 => "Cavium",
        0x44 => "DEC",
        0x46 => "Fujitsu",
        0x48 => "HiSilicon",
        0x4E => "NVIDIA",
        0x50 => "APM",
        0x51 => "Qualcomm",
        0x53 => "Samsung",
        0x56 => "Marvell",
        0x61 => "Apple",
        0x69 => "Intel",
        0xC0 => "Ampere",
        0x00 => "Software / QEMU",
        _ => "Unknown Implementer",
    };

    let part_name = match (implementer, part_num) {
        (0x41, 0xD03) => "Cortex-A53",
        (0x41, 0xD04) => "Cortex-A35",
        (0x41, 0xD05) => "Cortex-A55",
        (0x41, 0xD07) => "Cortex-A57",
        (0x41, 0xD08) => "Cortex-A72",
        (0x41, 0xD09) => "Cortex-A73",
        (0x41, 0xD0A) => "Cortex-A75",
        (0x41, 0xD0B) => "Cortex-A76",
        (0x41, 0xD0C) => "Neoverse-N1",
        (0x41, 0xD40) => "Neoverse-V1",
        (0x41, 0xD49) => "Neoverse-N2",
        (0x41, 0xD4F) => "Neoverse-V2",
        _ => "Generic ARMv8-A",
    };

    let mmfr0 = id_aa64mmfr0_el1();
    let pa_bits = match mmfr0 & 0xf {
        0b0000 => 32,
        0b0001 => 36,
        0b0010 => 40,
        0b0011 => 42,
        0b0100 => 44,
        0b0101 => 48,
        0b0110 => 52,
        _ => 48,
    };

    CpuInfo {
        implementer,
        variant,
        architecture,
        part_num,
        revision,
        implementer_name,
        part_name,
        pa_bits,
    }
}
