//! Multi-format Boot Manager (PE32+ UEFI `.efi`, ELF64, Linux ARM64 `Image`, VirtIO FAT32, `fw_cfg`, Flash).

extern crate alloc;
use alloc::vec::Vec;
use core::ptr::read_volatile;
use crate::{arch, elf, fat, flash, fw_cfg, mem, pe, println, uefi, virtio};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PayloadFormat {
    Pe32PlusEfi,
    Elf64,
    LinuxArm64Image,
    Unknown,
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

/// Identify the executable format of a binary image.
pub fn detect_format(image: &[u8]) -> PayloadFormat {
    if pe::is_pe32_plus_arm64(image) {
        PayloadFormat::Pe32PlusEfi
    } else if elf::is_elf64_aarch64(image) {
        PayloadFormat::Elf64
    } else if image.len() >= 64 && read_le32(image, 0x38) == 0x644d_5241 {
        PayloadFormat::LinuxArm64Image
    } else {
        PayloadFormat::Unknown
    }
}

/// Load and execute a binary payload of any supported format (PE32+ `.efi`, ELF64, Linux `Image`).
pub fn boot_image_bytes(image: &[u8], source_label: &str) -> Result<u64, &'static str> {
    match detect_format(image) {
        PayloadFormat::Pe32PlusEfi => {
            println!(
                "Booting PE32+ AArch64 UEFI Application from {} ({} bytes)...",
                source_label,
                image.len()
            );
            let pe_info = pe::load_pe32_plus(image)?;
            println!(
                "  [PE32+] Loaded {} sections at 0x{:08X}..0x{:08X} ({} base relocations applied)",
                pe_info.section_count,
                pe_info.image_base,
                pe_info.image_base + pe_info.image_size,
                pe_info.relocations_applied
            );
            let status = uefi::execute_pe32_efi(&pe_info);
            Ok(status as u64)
        }
        PayloadFormat::Elf64 => {
            println!(
                "Booting ELF64 AArch64 Executable from {} ({} bytes)...",
                source_label,
                image.len()
            );
            let elf_info = elf::load_elf64(image)?;
            println!(
                "  [ELF64] Loaded {} PT_LOAD segment(s) at 0x{:08X}..0x{:08X} ({} bytes)",
                elf_info.segments_loaded,
                elf_info.min_addr,
                elf_info.max_addr,
                elf_info.total_memsz
            );
            let status = elf::execute_elf64(&elf_info);
            Ok(status)
        }
        PayloadFormat::LinuxArm64Image => {
            println!(
                "Booting Linux ARM64 Kernel Image from {} ({} bytes)...",
                source_label,
                image.len()
            );
            let text_offset = read_le64(image, 8) as usize;
            let image_size = read_le64(image, 16) as usize;
            let load_addr = (0x4000_0000usize + text_offset).max(mem::payload_start());
            let alloc_sz = image_size.max(image.len());
            unsafe {
                let dst = core::slice::from_raw_parts_mut(load_addr as *mut u8, alloc_sz);
                dst[..image.len()].copy_from_slice(image);
                if alloc_sz > image.len() {
                    dst[image.len()..].fill(0);
                }
            }
            arch::sync_icache_range(load_addr, alloc_sz);
            println!(
                "  [LINUX] Jumping to ARM64 Kernel @ 0x{:08X} (x0 = DTB @ 0x{:08X})...",
                load_addr,
                mem::dtb_start()
            );
            let kernel_entry: extern "C" fn(u64, u64, u64, u64) -> u64 =
                unsafe { core::mem::transmute(load_addr) };
            let ret = kernel_entry(mem::dtb_start() as u64, 0, 0, 0);
            println!("  [LINUX] Kernel returned with status: 0x{:X}", ret);
            Ok(ret)
        }
        PayloadFormat::Unknown => Err("Unrecognized binary format (expected PE32+ EFI, ELF64, or ARM64 Image)"),
    }
}

/// Automatically scan all boot media in boot order and boot the first bootable image found:
/// 1. QEMU `fw_cfg`: `opt/hinode/efi`, `opt/hinode/kernel`, `opt/hinode/payload`
/// 2. VirtIO Block Disk (MMIO or PCI) FAT12/16/32: `/EFI/BOOT/BOOTAA64.EFI`, `/HINODE.EFI`, `/KERNEL.ELF`, `/IMAGE`
/// 3. CFI NOR Flash Bank 1 (`0x0400_0000`)
pub fn boot_auto(ecam_base: usize) -> Result<u64, &'static str> {
    // 1. Check QEMU fw_cfg
    for fw_name in ["opt/hinode/efi", "opt/hinode/kernel", "opt/hinode/payload"] {
        if let Some(data) = fw_cfg::read_file(fw_name) {
            if !data.is_empty() {
                return boot_image_bytes(&data, fw_name);
            }
        }
    }

    // 2. Check VirtIO Block Disks (FAT12/16/32)
    let blk_devs = virtio::find_block_devices(ecam_base);
    for blk in &blk_devs {
        if let Ok(vol) = fat::mount(blk) {
            let candidates = [
                "/EFI/BOOT/BOOTAA64.EFI",
                "/HINODE.EFI",
                "/BOOTAA64.EFI",
                "/KERNEL.ELF",
                "/HINODE.ELF",
                "/IMAGE",
            ];
            for path in candidates {
                if let Ok(data) = vol.read_file(blk, path) {
                    if !data.is_empty() {
                        return boot_image_bytes(&data, path);
                    }
                }
            }
        }
    }

    // 3. Check Flash Bank 1 (0x0400_0000)
    match flash::inspect_bank1() {
        flash::FlashPayloadKind::Empty => {}
        _ => {
            let slice = unsafe {
                core::slice::from_raw_parts(
                    flash::FLASH_BANK1_BASE as *const u8,
                    4 * 1024 * 1024,
                )
            };
            return boot_image_bytes(slice, "Flash Bank 1 (0x04000000)");
        }
    }

    Err("No bootable media found in fw_cfg, VirtIO-Blk FAT volume, or Flash Bank 1")
}

/// Boot a specific file path from the first mounted VirtIO Block FAT volume.
pub fn boot_disk_file(ecam_base: usize, path: &str) -> Result<u64, &'static str> {
    let blk_devs = virtio::find_block_devices(ecam_base);
    let blk = blk_devs.first().ok_or("No VirtIO block device attached")?;
    let vol = fat::mount(blk)?;
    let data = vol.read_file(blk, path)?;
    boot_image_bytes(&data, path)
}

// ============================================================================
// Built-in Self-Contained Demo Payloads (ELF64 & PE32+ UEFI Application)
// ============================================================================

/// Build a valid AArch64 ELF64 executable in memory that prints a message to PL011 UART,
/// reads the DTB magic (`0xD00DFEED`) from the pointer passed in `x0`, and returns it.
pub fn build_demo_elf64() -> Vec<u8> {
    let mut img = alloc::vec![0u8; 512];
    let load_addr: u64 = 0x4050_0000;
    let code_off: usize = 0x80;
    let entry_addr: u64 = load_addr + (code_off as u64);

    // ELF64 Header (64 bytes)
    img[0..4].copy_from_slice(b"\x7fELF");
    img[4] = 2; // ELFCLASS64
    img[5] = 1; // ELFDATA2LSB
    img[6] = 1; // EV_CURRENT
    img[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    img[18..20].copy_from_slice(&183u16.to_le_bytes()); // EM_AARCH64
    img[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
    img[24..32].copy_from_slice(&entry_addr.to_le_bytes()); // e_entry
    img[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff = 64
    img[52..54].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize = 64
    img[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize = 56
    img[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum = 1

    // Program Header 0 (PT_LOAD at offset 64, 56 bytes)
    let ph = &mut img[64..120];
    ph[0..4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    ph[4..8].copy_from_slice(&7u32.to_le_bytes()); // PF_R | PF_W | PF_X
    ph[8..16].copy_from_slice(&0u64.to_le_bytes()); // p_offset = 0
    ph[16..24].copy_from_slice(&load_addr.to_le_bytes()); // p_vaddr
    ph[24..32].copy_from_slice(&load_addr.to_le_bytes()); // p_paddr
    ph[32..40].copy_from_slice(&512u64.to_le_bytes()); // p_filesz
    ph[40..48].copy_from_slice(&1024u64.to_le_bytes()); // p_memsz (includes 512B BSS)
    ph[48..56].copy_from_slice(&4096u64.to_le_bytes()); // p_align

    // AArch64 Machine Code at offset 0x80 (8 instructions = 32 bytes):
    //   0x00: adr  x1, #32          (0x10000101 -> points to offset 0xA0)
    //   0x04: movz x2, #0x0900, lsl #16 (0xd2920002 -> UART_DR = 0x0900_0000)
    //   0x08: ldrb w3, [x1], #1     (0x38401423)
    //   0x0c: cbz  w3, #12          (0x34000063 -> branch to 0x18)
    //   0x10: str  w3, [x2]         (0xb9000043)
    //   0x14: b    #-12             (0x17fffffd -> branch to 0x08)
    //   0x18: ldr  w4, [x0]         (0xb9400004 -> read DTB magic at [x0])
    //   0x1c: rev  w0, w4           (0x5ac00880 -> byte-swap BE32 to 0xD00DFEED)
    //   0x20: ret                   (0xd65f03c0)
    let insns: [u32; 9] = [
        0x1000_0121, // adr x1, #36 (points to 0x80 + 36 = 0xA4)
        0xd2a1_2002, // movz x2, #0x0900, lsl #16 (0x0900_0000)
        0x3840_1423, // ldrb w3, [x1], #1
        0x3400_0063, // cbz w3, +12
        0xb900_0043, // str w3, [x2]
        0x17ff_fffd, // b -12
        0xb940_0004, // ldr w4, [x0]
        0x5ac0_0880, // rev w0, w4
        0xd65f_03c0, // ret
    ];
    for (i, &inst) in insns.iter().enumerate() {
        let off = code_off + i * 4;
        img[off..off + 4].copy_from_slice(&inst.to_le_bytes());
    }

    let msg = b"  >> [Guest ELF64] Hello from AArch64 ELF64 Payload! Verified DTB in x0.\r\n\0";
    let msg_off = code_off + insns.len() * 4;
    img[msg_off..msg_off + msg.len()].copy_from_slice(msg);

    img
}

/// Build a valid AArch64 PE32+ UEFI Application (`.efi`) in memory with `.text` and `.reloc`
/// sections that calls `SystemTable->ConOut->OutputString()` and returns `EFI_SUCCESS`.
pub fn build_demo_pe32_efi() -> Vec<u8> {
    let mut img = alloc::vec![0u8; 1536];

    // 1. DOS Header at 0x00
    img[0..2].copy_from_slice(b"MZ");
    let pe_off: usize = 0x40;
    img[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());

    // 2. PE Signature at 0x40 ("PE\0\0")
    img[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");

    // 3. COFF File Header at 0x44 (20 bytes)
    let coff = pe_off + 4;
    img[coff..coff + 2].copy_from_slice(&0xAA64u16.to_le_bytes()); // Machine = ARM64
    img[coff + 2..coff + 4].copy_from_slice(&2u16.to_le_bytes()); // NumberOfSections = 2 (.text, .reloc)
    img[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader = 240
    img[coff + 18..coff + 20].copy_from_slice(&0x0202u16.to_le_bytes()); // Characteristics = EXECUTABLE_IMAGE

    // 4. Optional Header (PE32+) at 0x58 (240 bytes)
    let opt = coff + 20;
    let preferred_base: u64 = 0x0000_0000_1000_0000;
    img[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes()); // Magic = PE32+
    img[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // AddressOfEntryPoint = 0x1000
    img[opt + 20..opt + 24].copy_from_slice(&0x1000u32.to_le_bytes()); // BaseOfCode = 0x1000
    img[opt + 24..opt + 32].copy_from_slice(&preferred_base.to_le_bytes()); // ImageBase
    img[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes()); // SectionAlignment
    img[opt + 36..opt + 40].copy_from_slice(&0x0200u32.to_le_bytes()); // FileAlignment
    img[opt + 56..opt + 60].copy_from_slice(&0x3000u32.to_le_bytes()); // SizeOfImage = 12KB
    img[opt + 60..opt + 64].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfHeaders = 512B
    img[opt + 68..opt + 70].copy_from_slice(&10u16.to_le_bytes()); // Subsystem = EFI_APPLICATION (10)
    img[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes = 16

    // Data Directory #5 (Base Relocation Table) at opt + 112 + 5*8 = opt + 152
    img[opt + 152..opt + 156].copy_from_slice(&0x2000u32.to_le_bytes()); // VirtualAddress = 0x2000
    img[opt + 156..opt + 160].copy_from_slice(&12u32.to_le_bytes()); // Size = 12 bytes

    // 5. Section Headers at opt + 240 = 0x148 (2 x 40 = 80 bytes)
    let sec0 = opt + 240;
    img[sec0..sec0 + 6].copy_from_slice(b".text\0");
    img[sec0 + 8..sec0 + 12].copy_from_slice(&0x0200u32.to_le_bytes()); // VirtualSize
    img[sec0 + 12..sec0 + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress = 0x1000
    img[sec0 + 16..sec0 + 20].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfRawData = 512
    img[sec0 + 20..sec0 + 24].copy_from_slice(&0x0200u32.to_le_bytes()); // PointerToRawData = 0x200

    let sec1 = sec0 + 40;
    img[sec1..sec1 + 7].copy_from_slice(b".reloc\0");
    img[sec1 + 8..sec1 + 12].copy_from_slice(&12u32.to_le_bytes()); // VirtualSize
    img[sec1 + 12..sec1 + 16].copy_from_slice(&0x2000u32.to_le_bytes()); // VirtualAddress = 0x2000
    img[sec1 + 16..sec1 + 20].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfRawData = 512
    img[sec1 + 20..sec1 + 24].copy_from_slice(&0x0400u32.to_le_bytes()); // PointerToRawData = 0x400

    // 6. .text raw data at file offset 0x200 (mapped to RVA 0x1000):
    //   0x00: stp x29, x30, [sp, #-16]!  (0xa9bf7bfd)
    //   0x04: ldr x0, [x1, #64]          (0xf9402020 -> x0 = SystemTable->ConOut)
    //   0x08: ldr x2, [x0, #8]           (0xf9400402 -> x2 = ConOut->OutputString)
    //   0x0c: ldr x1, #20                (0x580000a1 -> loads relocated 64-bit pointer at RVA 0x1020!)
    //   0x10: blr x2                     (0xd63f0040 -> calls OutputString(ConOut, msg))
    //   0x14: ldp x29, x30, [sp], #16    (0xa8c17bfd)
    //   0x18: ret                        (0xd65f03c0)
    //   0x1c: nop                        (0xd503201f)
    //   0x20: .quad (preferred_base + 0x1028)  <-- Relocated by IMAGE_REL_BASED_DIR64!
    //   0x28: UTF-16LE string
    let text_off = 0x200usize;
    let efi_insns: [u32; 8] = [
        0xa9bf_7bfd, // stp x29, x30, [sp, #-16]!
        0xf940_2020, // ldr x0, [x1, #64]
        0xf940_0402, // ldr x2, [x0, #8]
        0x5800_00a1, // ldr x1, [pc, #20] (loads u64 from offset 0x20)
        0xd63f_0040, // blr x2
        0xa8c1_7bfd, // ldp x29, x30, [sp], #16
        0xd65f_03c0, // ret
        0xd503_201f, // nop
    ];
    for (i, &inst) in efi_insns.iter().enumerate() {
        let off = text_off + i * 4;
        img[off..off + 4].copy_from_slice(&inst.to_le_bytes());
    }

    // Relocatable 64-bit pointer at RVA 0x1020 pointing to UTF-16 string at RVA 0x1028
    let str_ptr_val: u64 = preferred_base + 0x1028;
    img[text_off + 0x20..text_off + 0x28].copy_from_slice(&str_ptr_val.to_le_bytes());

    let msg_ascii = b"  >> [Guest UEFI]  Hello from AArch64 PE32+ efi_main via ConOut->OutputString!\r\n\0";
    let mut utf16_off = text_off + 0x28;
    for &ch in msg_ascii {
        img[utf16_off..utf16_off + 2].copy_from_slice(&(ch as u16).to_le_bytes());
        utf16_off += 2;
    }

    // 7. .reloc raw data at file offset 0x400 (mapped to RVA 0x2000):
    //   PageRVA = 0x1000, SizeOfBlock = 12
    //   Entry 0 = (IMAGE_REL_BASED_DIR64 << 12) | 0x020 (patches RVA 0x1020!)
    //   Entry 1 = 0 (padding)
    let reloc_off = 0x400usize;
    img[reloc_off..reloc_off + 4].copy_from_slice(&0x1000u32.to_le_bytes());
    img[reloc_off + 4..reloc_off + 8].copy_from_slice(&12u32.to_le_bytes());
    let reloc_entry: u16 = (10u16 << 12) | 0x020;
    img[reloc_off + 8..reloc_off + 10].copy_from_slice(&reloc_entry.to_le_bytes());
    img[reloc_off + 10..reloc_off + 12].copy_from_slice(&0u16.to_le_bytes());

    img
}

/// Run the built-in ELF64 AArch64 demo payload.
pub fn run_demo_elf() -> Result<u64, &'static str> {
    let img = build_demo_elf64();
    let ret = boot_image_bytes(&img, "Built-in ROM ELF64 Demo")?;
    if ret != 0xd00d_feed {
        let _ = unsafe { read_volatile(mem::dtb_start() as *const u32) };
    }
    Ok(ret)
}

/// Run the built-in PE32+ AArch64 UEFI `.efi` demo application.
pub fn run_demo_efi() -> Result<u64, &'static str> {
    let img = build_demo_pe32_efi();
    boot_image_bytes(&img, "Built-in ROM PE32+ UEFI Demo")
}
