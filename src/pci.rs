//! Generic PCIe ECAM Bus Scanner and BAR Resource Allocator for QEMU `virt`.

extern crate alloc;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use crate::{arch, println};

pub const DEFAULT_ECAM_BASE: usize = 0x40_1000_0000;
pub const PCIE_PIO_CPU_BASE: usize = 0x3eff_0000;
pub const PCIE_MMIO32_BASE: u32 = 0x1000_0000;
pub const PCIE_MMIO64_BASE: u64 = 0x80_0000_0000;

#[derive(Clone, Copy, Debug)]
pub struct PciBar {
    pub index: u8,
    pub is_io: bool,
    pub is_64bit: bool,
    pub addr: u64,
    pub cpu_addr: usize,
    pub size: u64,
}

#[derive(Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub subsys_vendor: u16,
    pub subsys_id: u16,
    pub bars: [Option<PciBar>; 6],
}

#[inline]
fn ecam_addr(ecam_base: usize, bus: u8, dev: u8, func: u8, offset: u16) -> usize {
    ecam_base
        + ((bus as usize) << 20)
        + ((dev as usize) << 15)
        + ((func as usize) << 12)
        + ((offset as usize) & 0xffc)
}

#[inline]
pub fn config_read32(ecam_base: usize, bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    unsafe { read_volatile(ecam_addr(ecam_base, bus, dev, func, offset) as *const u32) }
}

#[inline]
pub fn config_write32(ecam_base: usize, bus: u8, dev: u8, func: u8, offset: u16, val: u32) {
    unsafe {
        write_volatile(ecam_addr(ecam_base, bus, dev, func, offset) as *mut u32, val);
        arch::dsb_sy();
    }
}

#[inline]
const fn align_up_u64(val: u64, align: u64) -> u64 {
    (val + align - 1) & !(align - 1)
}

/// Enumerate PCIe Bus 0 and allocate BARs (I/O, 32-bit MMIO, 64-bit MMIO) for all discovered endpoints.
pub fn enumerate_and_allocate(ecam_base: usize) -> Vec<PciDevice> {
    let mut devices = Vec::new();
    let mut next_pio: u64 = 0x1000;
    let mut next_mmio32: u64 = PCIE_MMIO32_BASE as u64;
    let mut next_mmio64: u64 = PCIE_MMIO64_BASE;

    for dev in 0..32u8 {
        for func in 0..8u8 {
            let id_reg = config_read32(ecam_base, 0, dev, func, 0x00);
            let vendor_id = (id_reg & 0xffff) as u16;
            let device_id = ((id_reg >> 16) & 0xffff) as u16;
            if vendor_id == 0xffff || vendor_id == 0x0000 {
                if func == 0 {
                    break;
                }
                continue;
            }

            let class_reg = config_read32(ecam_base, 0, dev, func, 0x08);
            let revision = (class_reg & 0xff) as u8;
            let prog_if = ((class_reg >> 8) & 0xff) as u8;
            let subclass = ((class_reg >> 16) & 0xff) as u8;
            let class_code = ((class_reg >> 24) & 0xff) as u8;

            let hdr_reg = config_read32(ecam_base, 0, dev, func, 0x0C);
            let header_type = ((hdr_reg >> 16) & 0xff) as u8;

            let mut subsys_vendor = 0u16;
            let mut subsys_id = 0u16;
            let mut bars: [Option<PciBar>; 6] = [None; 6];

            if (header_type & 0x7f) == 0x00 {
                let sub_reg = config_read32(ecam_base, 0, dev, func, 0x2C);
                subsys_vendor = (sub_reg & 0xffff) as u16;
                subsys_id = ((sub_reg >> 16) & 0xffff) as u16;

                // Disable IO and Memory decoding while sizing BARs
                let orig_cmd = config_read32(ecam_base, 0, dev, func, 0x04);
                config_write32(ecam_base, 0, dev, func, 0x04, orig_cmd & !0x03);

                let mut bar_idx = 0usize;
                while bar_idx < 6 {
                    let bar_off = 0x10 + (bar_idx as u16) * 4;
                    let orig_lo = config_read32(ecam_base, 0, dev, func, bar_off);
                    let is_io = (orig_lo & 1) != 0;
                    let is_64 = !is_io && ((orig_lo >> 1) & 0x3) == 0x2;

                    if is_io {
                        config_write32(ecam_base, 0, dev, func, bar_off, 0xffff_ffff);
                        let mask = config_read32(ecam_base, 0, dev, func, bar_off) & 0xffff_fffc;
                        if mask != 0 && mask != 0xffff_fffc {
                            let size = (!mask).wrapping_add(1) as u64;
                            next_pio = align_up_u64(next_pio, size.max(16));
                            let assigned = next_pio;
                            next_pio += size;
                            config_write32(
                                ecam_base,
                                0,
                                dev,
                                func,
                                bar_off,
                                (assigned as u32) | 1,
                            );
                            bars[bar_idx] = Some(PciBar {
                                index: bar_idx as u8,
                                is_io: true,
                                is_64bit: false,
                                addr: assigned,
                                cpu_addr: PCIE_PIO_CPU_BASE + (assigned as usize),
                                size,
                            });
                        } else {
                            config_write32(ecam_base, 0, dev, func, bar_off, orig_lo);
                        }
                        bar_idx += 1;
                    } else if is_64 && bar_idx + 1 < 6 {
                        let orig_hi = config_read32(ecam_base, 0, dev, func, bar_off + 4);
                        config_write32(ecam_base, 0, dev, func, bar_off, 0xffff_ffff);
                        config_write32(ecam_base, 0, dev, func, bar_off + 4, 0xffff_ffff);
                        let mask_lo = config_read32(ecam_base, 0, dev, func, bar_off) & 0xffff_fff0;
                        let mask_hi = config_read32(ecam_base, 0, dev, func, bar_off + 4);
                        let mask64 = ((mask_hi as u64) << 32) | (mask_lo as u64);
                        if mask64 != 0 && mask64 != !0xfu64 {
                            let size = (!mask64).wrapping_add(1);
                            next_mmio64 = align_up_u64(next_mmio64, size.max(4096));
                            let assigned = next_mmio64;
                            next_mmio64 += size;
                            config_write32(
                                ecam_base,
                                0,
                                dev,
                                func,
                                bar_off,
                                (assigned as u32) | (orig_lo & 0xf),
                            );
                            config_write32(
                                ecam_base,
                                0,
                                dev,
                                func,
                                bar_off + 4,
                                (assigned >> 32) as u32,
                            );
                            bars[bar_idx] = Some(PciBar {
                                index: bar_idx as u8,
                                is_io: false,
                                is_64bit: true,
                                addr: assigned,
                                cpu_addr: assigned as usize,
                                size,
                            });
                        } else {
                            config_write32(ecam_base, 0, dev, func, bar_off, orig_lo);
                            config_write32(ecam_base, 0, dev, func, bar_off + 4, orig_hi);
                        }
                        bar_idx += 2;
                    } else {
                        config_write32(ecam_base, 0, dev, func, bar_off, 0xffff_ffff);
                        let mask = config_read32(ecam_base, 0, dev, func, bar_off) & 0xffff_fff0;
                        if mask != 0 && mask != 0xffff_fff0 {
                            let size = (!mask).wrapping_add(1) as u64;
                            next_mmio32 = align_up_u64(next_mmio32, size.max(4096));
                            let assigned = next_mmio32;
                            next_mmio32 += size;
                            config_write32(
                                ecam_base,
                                0,
                                dev,
                                func,
                                bar_off,
                                (assigned as u32) | (orig_lo & 0xf),
                            );
                            bars[bar_idx] = Some(PciBar {
                                index: bar_idx as u8,
                                is_io: false,
                                is_64bit: false,
                                addr: assigned,
                                cpu_addr: assigned as usize,
                                size,
                            });
                        } else {
                            config_write32(ecam_base, 0, dev, func, bar_off, orig_lo);
                        }
                        bar_idx += 1;
                    }
                }

                // Enable I/O Space (bit 0), Memory Space (bit 1), and Bus Master (bit 2)
                config_write32(ecam_base, 0, dev, func, 0x04, orig_cmd | 0x07);
            }

            devices.push(PciDevice {
                bus: 0,
                device: dev,
                function: func,
                vendor_id,
                device_id,
                class_code,
                subclass,
                prog_if,
                revision,
                header_type,
                subsys_vendor,
                subsys_id,
                bars,
            });

            if func == 0 && (header_type & 0x80) == 0 {
                break;
            }
        }
    }

    devices
}

pub fn device_description(vendor_id: u16, device_id: u16, class_code: u8, subclass: u8) -> &'static str {
    match (vendor_id, device_id) {
        (0x1b36, 0x0008) => "Red Hat QEMU PCIe Host Bridge",
        (0x1b36, 0x0010) => "Red Hat QEMU NVMe Controller",
        (0x1b36, 0x000d) => "Red Hat QEMU XHCI USB Controller",
        (0x1af4, 0x1000) | (0x1af4, 0x1041) => "VirtIO Network Device (PCI)",
        (0x1af4, 0x1001) | (0x1af4, 0x1042) => "VirtIO Block Storage Device (PCI)",
        (0x1af4, 0x1003) | (0x1af4, 0x1043) => "VirtIO Console Device (PCI)",
        (0x1af4, 0x1005) | (0x1af4, 0x1044) => "VirtIO RNG Entropy Device (PCI)",
        (0x1af4, 0x1050) => "VirtIO GPU Device (PCI)",
        (0x1af4, 0x1052) => "VirtIO Input Device (PCI)",
        (0x8086, 0x10d3) => "Intel 82574L Gigabit Network (e1000e)",
        (0x1234, 0x1111) => "QEMU Standard VGA Display",
        _ => match (class_code, subclass) {
            (0x06, 0x00) => "PCI Host Bridge",
            (0x06, 0x04) => "PCI-to-PCI Bridge",
            (0x01, 0x00) => "SCSI Storage Controller",
            (0x01, 0x06) => "SATA AHCI Controller",
            (0x01, 0x08) => "NVMe Non-Volatile Memory Controller",
            (0x01, _) => "Mass Storage Controller",
            (0x02, 0x00) => "Ethernet Network Controller",
            (0x03, _) => "Display Controller",
            (0x0C, 0x03) => "USB Host Controller",
            _ => "Generic PCIe Device",
        },
    }
}

pub fn print_devices(ecam_base: usize) {
    let devs = enumerate_and_allocate(ecam_base);
    println!(
        "PCIe ECAM Bus 0 Enumeration (@ 0x{:010X}): {} device(s) found",
        ecam_base,
        devs.len()
    );
    for d in &devs {
        println!(
            "  {:02X}:{:02X}.{} [{:04X}:{:04X}] Class {:02X}{:02X} (rev {:02X}) - {}",
            d.bus,
            d.device,
            d.function,
            d.vendor_id,
            d.device_id,
            d.class_code,
            d.subclass,
            d.revision,
            device_description(d.vendor_id, d.device_id, d.class_code, d.subclass)
        );
        for bar_opt in &d.bars {
            if let Some(bar) = bar_opt {
                println!(
                    "    └─ BAR{}: {:<8} PCI=0x{:010X} -> CPU=0x{:010X} (size: {} B)",
                    bar.index,
                    if bar.is_io {
                        "I/O"
                    } else if bar.is_64bit {
                        "MMIO-64"
                    } else {
                        "MMIO-32"
                    },
                    bar.addr,
                    bar.cpu_addr,
                    bar.size
                );
            }
        }
    }
}
