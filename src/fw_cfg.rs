//! QEMU Firmware Configuration (`fw_cfg`) MMIO and DMA driver (`0x0902_0000`).

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use crate::{arch, mem, println};

pub const FW_CFG_BASE: usize = 0x0902_0000;

const FW_CFG_DATA8: *const u8 = (FW_CFG_BASE + 0x00) as *const u8;
const FW_CFG_DATA32: *const u32 = (FW_CFG_BASE + 0x00) as *const u32;
const FW_CFG_CTL: *mut u16 = (FW_CFG_BASE + 0x08) as *mut u16;
const FW_CFG_DMA: *mut u64 = (FW_CFG_BASE + 0x10) as *mut u64;

const FW_CFG_SIGNATURE: u16 = 0x0000;
const FW_CFG_ID: u16 = 0x0001;
const FW_CFG_FILE_DIR: u16 = 0x0019;

// DMA control bits
const FW_CFG_DMA_CTL_ERROR: u32 = 1 << 0;
const FW_CFG_DMA_CTL_READ: u32 = 1 << 1;
const FW_CFG_DMA_CTL_SELECT: u32 = 1 << 3;

#[repr(C, align(16))]
struct FwCfgDmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

#[derive(Clone)]
pub struct FwCfgFileEntry {
    pub name: [u8; 56],
    pub name_len: usize,
    pub size: u32,
    pub select: u16,
}

impl FwCfgFileEntry {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("<invalid>")
    }
}

#[inline]
fn select(selector: u16) {
    unsafe {
        // QEMU MMIO fw_cfg selector register expects big-endian 16-bit write
        write_volatile(FW_CFG_CTL, selector.to_be());
        arch::dsb_sy();
    }
}

#[inline]
fn read_bytes_pio(selector: u16, buf: &mut [u8]) {
    select(selector);
    for b in buf.iter_mut() {
        *b = unsafe { read_volatile(FW_CFG_DATA8) };
    }
}

/// Perform a high-speed `fw_cfg` DMA read into `buf`.
pub fn read_bytes_dma(selector: u16, buf: &mut [u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    let dma_desc_ptr = mem::dma_start() as *mut FwCfgDmaAccess;
    let ctl = FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ | ((selector as u32) << 16);
    unsafe {
        write_volatile(
            dma_desc_ptr,
            FwCfgDmaAccess {
                control: ctl.to_be(),
                length: (buf.len() as u32).to_be(),
                address: (buf.as_mut_ptr() as u64).to_be(),
            },
        );
        arch::dsb_sy();
        write_volatile(FW_CFG_DMA, (dma_desc_ptr as u64).to_be());
        arch::dsb_sy();

        for _ in 0..1_000_000 {
            let status = u32::from_be(read_volatile(&raw const (*dma_desc_ptr).control));
            if (status & !FW_CFG_DMA_CTL_ERROR) == 0 {
                return (status & FW_CFG_DMA_CTL_ERROR) == 0;
            }
            core::hint::spin_loop();
        }
    }
    false
}

/// Check if QEMU `fw_cfg` is present and return `(present, dma_supported, rev_id)`.
pub fn probe() -> (bool, bool, u32) {
    let mut sig = [0u8; 4];
    read_bytes_pio(FW_CFG_SIGNATURE, &mut sig);
    if &sig != b"QEMU" {
        return (false, false, 0);
    }

    select(FW_CFG_ID);
    let rev = unsafe { u32::from_le(read_volatile(FW_CFG_DATA32)) };
    let dma_reg = unsafe { u64::from_be(read_volatile(FW_CFG_DMA as *const u64)) };
    let dma_supported = dma_reg == 0x51454d5520434647; // "QEMU CFG"

    (true, dma_supported, rev)
}

/// List all files present in the `fw_cfg` directory (`0x0019`).
pub fn list_files() -> Vec<FwCfgFileEntry> {
    let mut result = Vec::new();
    let (present, _, _) = probe();
    if !present {
        return result;
    }

    select(FW_CFG_FILE_DIR);
    let mut count_bytes = [0u8; 4];
    for b in &mut count_bytes {
        *b = unsafe { read_volatile(FW_CFG_DATA8) };
    }
    let count = u32::from_be_bytes(count_bytes) as usize;
    if count > 256 {
        return result;
    }

    for _ in 0..count {
        let mut raw = [0u8; 64];
        for b in &mut raw {
            *b = unsafe { read_volatile(FW_CFG_DATA8) };
        }
        let size = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let select_id = u16::from_be_bytes([raw[4], raw[5]]);
        let mut name = [0u8; 56];
        name.copy_from_slice(&raw[8..64]);
        let mut name_len = 0;
        while name_len < 56 && name[name_len] != 0 {
            name_len += 1;
        }
        result.push(FwCfgFileEntry {
            name,
            name_len,
            size,
            select: select_id,
        });
    }

    result
}

/// Find a file entry in `fw_cfg` by name (e.g. `"opt/hinode/cmd"` or `"etc/boot-fail-wait"`).
pub fn find_file(target_name: &str) -> Option<FwCfgFileEntry> {
    for entry in list_files() {
        if entry.name_str() == target_name {
            return Some(entry);
        }
    }
    None
}

/// Read the contents of a named `fw_cfg` file into a `Vec<u8>`.
pub fn read_file(target_name: &str) -> Option<Vec<u8>> {
    let entry = find_file(target_name)?;
    if entry.size > 16 * 1024 * 1024 {
        return None;
    }
    let mut data = vec![0u8; entry.size as usize];
    if !read_bytes_dma(entry.select, &mut data) {
        read_bytes_pio(entry.select, &mut data);
    }
    Some(data)
}

/// Print the status and file directory of QEMU `fw_cfg`.
pub fn print_info() {
    let (present, dma, rev) = probe();
    if !present {
        println!("QEMU fw_cfg : Not detected at 0x{:08X}", FW_CFG_BASE);
        return;
    }
    println!(
        "QEMU fw_cfg : Detected at 0x{:08X} (Signature: \"QEMU\", Rev: 0x{:X}, DMA: {})",
        FW_CFG_BASE,
        rev,
        if dma { "Enabled (\"QEMU CFG\")" } else { "Disabled" }
    );
    let files = list_files();
    if files.is_empty() {
        println!("  Directory : 0 files in fw_cfg file table");
    } else {
        println!("  Directory : {} file(s):", files.len());
        for f in &files {
            println!(
                "    - [sel=0x{:04X}] {:<36} ({} bytes)",
                f.select,
                f.name_str(),
                f.size
            );
        }
    }
}
