//! PE32+ AArch64 UEFI executable (`.efi`) parser, section loader, and base relocator.

use core::ptr::{read_unaligned, write_unaligned};
use crate::{arch, mem};

const MZ_MAGIC: u16 = 0x5A4D; // "MZ"
const PE_MAGIC: u32 = 0x0000_4550; // "PE\0\0"
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;
const PE32_PLUS_MAGIC: u16 = 0x020B;
const IMAGE_REL_BASED_ABSOLUTE: u16 = 0;
const IMAGE_REL_BASED_DIR64: u16 = 10;

pub struct PeLoadedInfo {
    pub image_base: usize,
    pub image_size: usize,
    pub entry_point: usize,
    pub section_count: usize,
    pub relocations_applied: usize,
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

pub fn is_pe32_plus_arm64(image: &[u8]) -> bool {
    if image.len() < 0x80 || read_le16(image, 0) != MZ_MAGIC {
        return false;
    }
    let lfanew = read_le32(image, 0x3C) as usize;
    if lfanew + 26 > image.len() {
        return false;
    }
    read_le32(image, lfanew) == PE_MAGIC
        && read_le16(image, lfanew + 4) == IMAGE_FILE_MACHINE_ARM64
        && read_le16(image, lfanew + 24) == PE32_PLUS_MAGIC
}

/// Load a PE32+ AArch64 UEFI image into physical memory at `_payload_start` and apply `.reloc` fixups.
pub fn load_pe32_plus(image: &[u8]) -> Result<PeLoadedInfo, &'static str> {
    if image.len() < 0x80 || read_le16(image, 0) != MZ_MAGIC {
        return Err("Invalid DOS MZ header");
    }
    let e_lfanew = read_le32(image, 0x3C) as usize;
    if e_lfanew + 24 + 112 > image.len() {
        return Err("PE header offset out of bounds");
    }
    if read_le32(image, e_lfanew) != PE_MAGIC {
        return Err("Invalid PE\\0\\0 signature");
    }

    let coff_off = e_lfanew + 4;
    let machine = read_le16(image, coff_off);
    if machine != IMAGE_FILE_MACHINE_ARM64 {
        return Err("PE image is not for AArch64 (expected 0xAA64)");
    }
    let num_sections = read_le16(image, coff_off + 2) as usize;
    let size_of_opt = read_le16(image, coff_off + 16) as usize;

    let opt_off = coff_off + 20;
    if read_le16(image, opt_off) != PE32_PLUS_MAGIC {
        return Err("Not a PE32+ (64-bit) Optional Header");
    }

    let entry_rva = read_le32(image, opt_off + 16) as usize;
    let preferred_base = read_le64(image, opt_off + 24);
    let size_of_image = read_le32(image, opt_off + 56) as usize;
    let size_of_headers = read_le32(image, opt_off + 60) as usize;
    let num_rva_and_sizes = read_le32(image, opt_off + 108) as usize;

    if size_of_image == 0 || size_of_image > 64 * 1024 * 1024 {
        return Err("Invalid PE32+ SizeOfImage");
    }

    let target_base = (mem::payload_start() + 0xfff) & !0xfff;
    let dst_slice = unsafe { core::slice::from_raw_parts_mut(target_base as *mut u8, size_of_image) };
    dst_slice.fill(0);

    let hdr_copy_len = size_of_headers.min(image.len()).min(size_of_image);
    dst_slice[..hdr_copy_len].copy_from_slice(&image[..hdr_copy_len]);

    let sec_table_off = opt_off + size_of_opt;
    if sec_table_off + num_sections * 40 > image.len() {
        return Err("PE section table exceeds file bounds");
    }

    for i in 0..num_sections {
        let s_off = sec_table_off + i * 40;
        let virt_size = read_le32(image, s_off + 8) as usize;
        let virt_addr = read_le32(image, s_off + 12) as usize;
        let raw_size = read_le32(image, s_off + 16) as usize;
        let raw_ptr = read_le32(image, s_off + 20) as usize;

        if raw_size == 0 || raw_ptr == 0 {
            continue;
        }
        if raw_ptr + raw_size > image.len() {
            return Err("PE section raw data exceeds file bounds");
        }
        let copy_len = if virt_size > 0 {
            raw_size.min(virt_size)
        } else {
            raw_size
        };
        if virt_addr + copy_len > size_of_image {
            return Err("PE section virtual address exceeds SizeOfImage");
        }
        dst_slice[virt_addr..virt_addr + copy_len]
            .copy_from_slice(&image[raw_ptr..raw_ptr + copy_len]);
    }

    // Apply Base Relocations (Data Directory #5)
    let mut relocations_applied = 0usize;
    let delta = (target_base as u64).wrapping_sub(preferred_base);
    if delta != 0 && num_rva_and_sizes > 5 && size_of_opt >= 112 + 6 * 8 {
        let reloc_dir_off = opt_off + 112 + 5 * 8;
        let reloc_rva = read_le32(image, reloc_dir_off) as usize;
        let reloc_size = read_le32(image, reloc_dir_off + 4) as usize;

        if reloc_rva != 0 && reloc_size >= 8 && reloc_rva + reloc_size <= size_of_image {
            let mut b_off = reloc_rva;
            let b_end = reloc_rva + reloc_size;
            while b_off + 8 <= b_end {
                let page_rva = read_le32(dst_slice, b_off) as usize;
                let block_size = read_le32(dst_slice, b_off + 4) as usize;
                if block_size < 8 || b_off + block_size > b_end {
                    break;
                }
                let entries = (block_size - 8) / 2;
                for e in 0..entries {
                    let entry = read_le16(dst_slice, b_off + 8 + e * 2);
                    let rtype = entry >> 12;
                    let roff = (entry & 0x0fff) as usize;
                    match rtype {
                        IMAGE_REL_BASED_ABSOLUTE => {}
                        IMAGE_REL_BASED_DIR64 => {
                            let patch_rva = page_rva + roff;
                            if patch_rva + 8 <= size_of_image {
                                unsafe {
                                    let p = (target_base + patch_rva) as *mut u64;
                                    let cur = read_unaligned(p);
                                    write_unaligned(p, cur.wrapping_add(delta));
                                }
                                relocations_applied += 1;
                            }
                        }
                        _ => {}
                    }
                }
                b_off += block_size;
            }
        }
    }

    arch::sync_icache_range(target_base, size_of_image);

    Ok(PeLoadedInfo {
        image_base: target_base,
        image_size: size_of_image,
        entry_point: target_base + entry_rva,
        section_count: num_sections,
        relocations_applied,
    })
}
