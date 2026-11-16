//! ELF64 AArch64 executable parser, segment loader, and dispatcher.

use crate::{arch, mem, println};

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_AARCH64: u16 = 183;
const PT_LOAD: u32 = 1;

pub struct ElfLoadedInfo {
    pub entry_point: usize,
    pub segments_loaded: usize,
    pub total_memsz: usize,
    pub min_addr: usize,
    pub max_addr: usize,
}

#[inline]
fn read_le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

#[inline]
fn read_le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[inline]
fn read_le64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

pub fn is_elf64_aarch64(image: &[u8]) -> bool {
    if image.len() < 64 {
        return false;
    }
    image[0..4] == ELF_MAGIC
        && image[4] == ELFCLASS64
        && image[5] == ELFDATA2LSB
        && read_le16(image, 18) == EM_AARCH64
}

/// Parse and load an AArch64 ELF64 binary into memory.
pub fn load_elf64(image: &[u8]) -> Result<ElfLoadedInfo, &'static str> {
    if image.len() < 64 {
        return Err("Image too small for ELF64 header");
    }
    if image[0..4] != ELF_MAGIC {
        return Err("Invalid ELF magic");
    }
    if image[4] != ELFCLASS64 || image[5] != ELFDATA2LSB {
        return Err("Not a 64-bit little-endian ELF image");
    }
    let e_type = read_le16(image, 16);
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err("Unsupported ELF type (expected ET_EXEC or ET_DYN)");
    }
    let e_machine = read_le16(image, 18);
    if e_machine != EM_AARCH64 {
        return Err("ELF image is not for AArch64 (EM_AARCH64 = 183)");
    }

    let e_entry = read_le64(image, 24) as usize;
    let e_phoff = read_le64(image, 32) as usize;
    let e_phentsize = read_le16(image, 54) as usize;
    let e_phnum = read_le16(image, 56) as usize;

    if e_phoff + e_phentsize * e_phnum > image.len() {
        return Err("ELF program header table exceeds image size");
    }

    // Determine if PIE / low-linked image needs relocation into `_payload_start`
    let mut min_paddr = usize::MAX;
    for i in 0..e_phnum {
        let ph = &image[e_phoff + i * e_phentsize..e_phoff + (i + 1) * e_phentsize];
        if read_le32(ph, 0) == PT_LOAD {
            let p_paddr = read_le64(ph, 24) as usize;
            if p_paddr < min_paddr {
                min_paddr = p_paddr;
            }
        }
    }

    if min_paddr == usize::MAX {
        return Err("ELF image contains no PT_LOAD segments");
    }

    let load_bias = if min_paddr < mem::payload_start() {
        mem::payload_start().saturating_sub(min_paddr)
    } else {
        0
    };

    let mut segments_loaded = 0usize;
    let mut total_memsz = 0usize;
    let mut loaded_min = usize::MAX;
    let mut loaded_max = 0usize;

    for i in 0..e_phnum {
        let ph = &image[e_phoff + i * e_phentsize..e_phoff + (i + 1) * e_phentsize];
        let p_type = read_le32(ph, 0);
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = read_le64(ph, 8) as usize;
        let p_paddr = read_le64(ph, 24) as usize;
        let p_filesz = read_le64(ph, 32) as usize;
        let p_memsz = read_le64(ph, 40) as usize;

        if p_offset + p_filesz > image.len() {
            return Err("ELF PT_LOAD segment exceeds file bounds");
        }

        let target_addr = p_paddr + load_bias;
        unsafe {
            let dst = core::slice::from_raw_parts_mut(target_addr as *mut u8, p_memsz);
            dst[..p_filesz].copy_from_slice(&image[p_offset..p_offset + p_filesz]);
            if p_memsz > p_filesz {
                dst[p_filesz..].fill(0);
            }
        }
        arch::sync_icache_range(target_addr, p_memsz);

        segments_loaded += 1;
        total_memsz += p_memsz;
        if target_addr < loaded_min {
            loaded_min = target_addr;
        }
        if target_addr + p_memsz > loaded_max {
            loaded_max = target_addr + p_memsz;
        }
    }

    Ok(ElfLoadedInfo {
        entry_point: e_entry + load_bias,
        segments_loaded,
        total_memsz,
        min_addr: loaded_min,
        max_addr: loaded_max,
    })
}

/// Execute a loaded AArch64 ELF64 binary at `info.entry_point(dtb_addr, 0, 0, 0)`.
pub fn execute_elf64(info: &ElfLoadedInfo) -> u64 {
    println!(
        "  [ELF64] Jumping to entry point 0x{:016X} (x0 = DTB @ 0x{:08X})...",
        info.entry_point,
        mem::dtb_start()
    );
    let entry_fn: extern "C" fn(u64, u64, u64, u64) -> u64 =
        unsafe { core::mem::transmute(info.entry_point) };
    let ret = entry_fn(mem::dtb_start() as u64, 0, 0, 0);
    println!("  [ELF64] Payload returned to Hinode BIOS with status: 0x{:X}", ret);
    ret
}
