//! Flattened Device Tree (FDT / DTB) binary parser for hardware discovery at `0x4000_0000`.

use crate::{mem, print, println};

pub const FDT_MAGIC: u32 = 0xd00dfeed;

const FDT_BEGIN_NODE: u32 = 0x0000_0001;
const FDT_END_NODE: u32 = 0x0000_0002;
const FDT_PROP: u32 = 0x0000_0003;
const FDT_NOP: u32 = 0x0000_0004;
const FDT_END: u32 = 0x0000_0009;

#[derive(Clone, Copy)]
pub struct DtbDeviceNode {
    pub name: &'static str,
    pub compatible: &'static str,
    pub reg_base: u64,
    pub reg_size: u64,
}

pub struct DtbInfo {
    pub valid: bool,
    pub base_addr: usize,
    pub total_size: u32,
    pub version: u32,
    pub model: &'static str,
    pub compatible: &'static str,
    pub ram_base: u64,
    pub ram_size: u64,
    pub cpu_count: usize,
    pub cpu_compatible: &'static str,
    pub icache_size: u32,
    pub dcache_size: u32,
    pub psci_method: &'static str,
    pub stdout_path: &'static str,
    pub bootargs: &'static str,
    pub initrd_start: u64,
    pub initrd_end: u64,
    pub apb_clock_hz: u32,
    pub pcie_ecam_base: u64,
    pub pcie_ecam_size: u64,
    pub virtio_mmio_count: usize,
    pub node_count: usize,
    pub prop_count: usize,
    pub devices: [DtbDeviceNode; 16],
    pub device_count: usize,
}

#[inline]
fn read_be32(addr: usize) -> u32 {
    let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, 4) };
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[inline]
fn read_be64(addr: usize) -> u64 {
    let hi = read_be32(addr) as u64;
    let lo = read_be32(addr + 4) as u64;
    (hi << 32) | lo
}

#[inline]
const fn align4(val: usize) -> usize {
    (val + 3) & !3
}

unsafe fn cstr_at(addr: usize, max_len: usize) -> &'static str {
    let slice = unsafe { core::slice::from_raw_parts(addr as *const u8, max_len) };
    let mut len = 0;
    while len < max_len && slice[len] != 0 {
        len += 1;
    }
    core::str::from_utf8(&slice[..len]).unwrap_or("<invalid-utf8>")
}

/// Parse the Flattened Device Tree at `dtb_Start` (`0x4000_0000`).
pub fn parse() -> DtbInfo {
    parse_at(mem::dtb_start())
}

pub fn parse_at(base: usize) -> DtbInfo {
    let empty_node = DtbDeviceNode {
        name: "",
        compatible: "",
        reg_base: 0,
        reg_size: 0,
    };
    let mut info = DtbInfo {
        valid: false,
        base_addr: base,
        total_size: 0,
        version: 0,
        model: "Unknown",
        compatible: "Unknown",
        ram_base: 0x4000_0000,
        ram_size: 128 * 1024 * 1024,
        cpu_count: 0,
        cpu_compatible: "arm,cortex-a57",
        icache_size: 0,
        dcache_size: 0,
        psci_method: "hvc",
        stdout_path: "/pl011@9000000",
        bootargs: "",
        initrd_start: 0,
        initrd_end: 0,
        apb_clock_hz: 24_000_000,
        pcie_ecam_base: 0x40_1000_0000,
        pcie_ecam_size: 0x1000_0000,
        virtio_mmio_count: 0,
        node_count: 0,
        prop_count: 0,
        devices: [empty_node; 16],
        device_count: 0,
    };

    let magic = read_be32(base);
    if magic != FDT_MAGIC {
        return info;
    }

    let totalsize = read_be32(base + 4);
    let off_dt_struct = read_be32(base + 8) as usize;
    let off_dt_strings = read_be32(base + 12) as usize;
    let version = read_be32(base + 20);

    if totalsize < 40 || totalsize > 2 * 1024 * 1024 {
        return info;
    }

    info.valid = true;
    info.total_size = totalsize;
    info.version = version;

    let struct_start = base + off_dt_struct;
    let struct_end = base + (totalsize as usize);
    let strings_base = base + off_dt_strings;

    let mut ptr = struct_start;
    let mut depth = 0usize;
    let mut current_node: &'static str = "";
    let mut cur_compat: &'static str = "";
    let mut cur_reg_base: u64 = 0;
    let mut cur_reg_size: u64 = 0;
    let mut cur_has_reg = false;

    while ptr + 4 <= struct_end {
        let token = read_be32(ptr);
        ptr += 4;

        match token {
            FDT_BEGIN_NODE => {
                // Save previous node if it was a top-level hardware peripheral
                if depth == 2 && cur_has_reg && !current_node.starts_with("virtio_mmio@") {
                    if info.device_count < info.devices.len() {
                        info.devices[info.device_count] = DtbDeviceNode {
                            name: current_node,
                            compatible: cur_compat,
                            reg_base: cur_reg_base,
                            reg_size: cur_reg_size,
                        };
                        info.device_count += 1;
                    }
                }

                let name = unsafe { cstr_at(ptr, 128) };
                let name_bytes = name.len() + 1;
                ptr += align4(name_bytes);

                depth += 1;
                info.node_count += 1;
                current_node = name;
                cur_compat = "";
                cur_reg_base = 0;
                cur_reg_size = 0;
                cur_has_reg = false;

                if name.starts_with("cpu@") {
                    info.cpu_count += 1;
                } else if name.starts_with("virtio_mmio@") {
                    info.virtio_mmio_count += 1;
                }
            }
            FDT_END_NODE => {
                if depth == 2 && cur_has_reg && !current_node.starts_with("virtio_mmio@") {
                    if info.device_count < info.devices.len() {
                        info.devices[info.device_count] = DtbDeviceNode {
                            name: current_node,
                            compatible: cur_compat,
                            reg_base: cur_reg_base,
                            reg_size: cur_reg_size,
                        };
                        info.device_count += 1;
                    }
                    cur_has_reg = false;
                }
                depth = depth.saturating_sub(1);
            }
            FDT_PROP => {
                if ptr + 8 > struct_end {
                    break;
                }
                let len = read_be32(ptr) as usize;
                let nameoff = read_be32(ptr + 4) as usize;
                ptr += 8;

                let prop_name = unsafe { cstr_at(strings_base + nameoff, 64) };
                let val_ptr = ptr;
                ptr += align4(len);
                info.prop_count += 1;

                if depth == 1 {
                    if prop_name == "model" && len > 0 {
                        info.model = unsafe { cstr_at(val_ptr, len) };
                    } else if prop_name == "compatible" && len > 0 {
                        info.compatible = unsafe { cstr_at(val_ptr, len) };
                    }
                } else {
                    if prop_name == "compatible" && len > 0 {
                        cur_compat = unsafe { cstr_at(val_ptr, len) };
                        if current_node.starts_with("cpu@") {
                            info.cpu_compatible = cur_compat;
                        }
                    } else if prop_name == "reg" && len >= 16 {
                        cur_reg_base = read_be64(val_ptr);
                        cur_reg_size = read_be64(val_ptr + 8);
                        cur_has_reg = true;
                        if current_node.starts_with("memory@") || current_node == "memory" {
                            info.ram_base = cur_reg_base;
                            info.ram_size = cur_reg_size;
                        } else if current_node.starts_with("pcie@") {
                            info.pcie_ecam_base = cur_reg_base;
                            info.pcie_ecam_size = cur_reg_size;
                        }
                    } else if current_node.starts_with("cpu@") {
                        if prop_name == "i-cache-size" && len == 4 {
                            info.icache_size = read_be32(val_ptr);
                        } else if prop_name == "d-cache-size" && len == 4 {
                            info.dcache_size = read_be32(val_ptr);
                        }
                    } else if current_node == "psci" && prop_name == "method" && len > 0 {
                        info.psci_method = unsafe { cstr_at(val_ptr, len) };
                    } else if current_node == "chosen" {
                        if prop_name == "stdout-path" && len > 0 {
                            info.stdout_path = unsafe { cstr_at(val_ptr, len) };
                        } else if prop_name == "bootargs" && len > 0 {
                            info.bootargs = unsafe { cstr_at(val_ptr, len) };
                        } else if prop_name == "linux,initrd-start" {
                            info.initrd_start = if len == 8 {
                                read_be64(val_ptr)
                            } else {
                                read_be32(val_ptr) as u64
                            };
                        } else if prop_name == "linux,initrd-end" {
                            info.initrd_end = if len == 8 {
                                read_be64(val_ptr)
                            } else {
                                read_be32(val_ptr) as u64
                            };
                        }
                    } else if current_node == "apb-pclk" && prop_name == "clock-frequency" && len == 4 {
                        info.apb_clock_hz = read_be32(val_ptr);
                    }
                }
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => break,
        }
    }

    if info.cpu_count == 0 {
        info.cpu_count = 1;
    }

    info
}

/// Print a structured hardware summary extracted from the Flattened Device Tree.
pub fn print_summary(info: &DtbInfo) {
    if !info.valid {
        println!("DTB Status   : Not found or invalid magic at 0x{:08X}", info.base_addr);
        return;
    }

    println!("Flattened Device Tree (FDT) Summary:");
    println!(
        "  DTB Address  : 0x{:08X} (size: {} bytes, FDT v{}, {} nodes, {} properties)",
        info.base_addr, info.total_size, info.version, info.node_count, info.prop_count
    );
    println!("  Machine      : {} (compatible: {})", info.model, info.compatible);
    println!(
        "  CPUs         : {}x {} (I-Cache: {} KB, D-Cache: {} KB)",
        info.cpu_count,
        info.cpu_compatible,
        info.icache_size / 1024,
        info.dcache_size / 1024
    );
    println!(
        "  System RAM   : 0x{:08X} .. 0x{:08X} ({} MB)",
        info.ram_base,
        info.ram_base + info.ram_size,
        info.ram_size / (1024 * 1024)
    );
    println!("  PSCI Method  : {}", info.psci_method);
    println!("  Stdout Path  : {}", info.stdout_path);
    println!("  APB Clock    : {} MHz", info.apb_clock_hz / 1_000_000);
    println!(
        "  PCIe ECAM    : 0x{:010X} (size: {} MB)",
        info.pcie_ecam_base,
        info.pcie_ecam_size / (1024 * 1024)
    );
    println!("  VirtIO MMIO  : {} transport slots (0x0A000000 .. 0x0A003E00)", info.virtio_mmio_count);
    if !info.bootargs.is_empty() {
        println!("  Bootargs     : {}", info.bootargs);
    }
    if info.initrd_start != 0 {
        println!("  Initrd       : 0x{:08X} .. 0x{:08X}", info.initrd_start, info.initrd_end);
    }
    println!("  Discovered MMIO Nodes:");
    for i in 0..info.device_count {
        let dev = &info.devices[i];
        println!(
            "    - {:<18} @ 0x{:010X} (size: 0x{:08X}) [{}]",
            dev.name, dev.reg_base, dev.reg_size, dev.compatible
        );
    }
}

/// Dump the full hierarchical structure of the Flattened Device Tree.
pub fn dump_tree(base: usize, max_depth: usize) {
    if read_be32(base) != FDT_MAGIC {
        println!("Invalid FDT magic at 0x{:08X}", base);
        return;
    }
    let totalsize = read_be32(base + 4) as usize;
    let off_dt_struct = read_be32(base + 8) as usize;
    let off_dt_strings = read_be32(base + 12) as usize;

    let struct_start = base + off_dt_struct;
    let struct_end = base + totalsize;
    let strings_base = base + off_dt_strings;

    let mut ptr = struct_start;
    let mut depth = 0usize;

    while ptr + 4 <= struct_end {
        let token = read_be32(ptr);
        ptr += 4;
        match token {
            FDT_BEGIN_NODE => {
                let name = unsafe { cstr_at(ptr, 128) };
                ptr += align4(name.len() + 1);
                if depth <= max_depth {
                    for _ in 0..depth {
                        print!("  ");
                    }
                    let display_name = if name.is_empty() { "/" } else { name };
                    println!("{} {{", display_name);
                }
                depth += 1;
            }
            FDT_END_NODE => {
                depth = depth.saturating_sub(1);
                if depth <= max_depth {
                    for _ in 0..depth {
                        print!("  ");
                    }
                    println!("}};");
                }
            }
            FDT_PROP => {
                if ptr + 8 > struct_end {
                    break;
                }
                let len = read_be32(ptr) as usize;
                let nameoff = read_be32(ptr + 4) as usize;
                ptr += 8;
                let prop_name = unsafe { cstr_at(strings_base + nameoff, 64) };
                let val_ptr = ptr;
                ptr += align4(len);

                if depth <= max_depth {
                    for _ in 0..depth {
                        print!("  ");
                    }
                    if len == 0 {
                        println!("{};", prop_name);
                    } else if is_printable_str_prop(prop_name, val_ptr, len) {
                        let s = unsafe { cstr_at(val_ptr, len) };
                        println!("{} = \"{}\";", prop_name, s);
                    } else if len == 4 {
                        println!("{} = <0x{:08X}>;", prop_name, read_be32(val_ptr));
                    } else if len == 8 {
                        println!(
                            "{} = <0x{:08X} 0x{:08X}>;",
                            prop_name,
                            read_be32(val_ptr),
                            read_be32(val_ptr + 4)
                        );
                    } else {
                        println!("{} = <{} bytes>;", prop_name, len);
                    }
                }
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => break,
        }
    }
}

fn is_printable_str_prop(prop_name: &str, val_ptr: usize, len: usize) -> bool {
    if len < 2 {
        return false;
    }
    if prop_name == "compatible"
        || prop_name == "model"
        || prop_name == "device_type"
        || prop_name == "method"
        || prop_name == "stdout-path"
        || prop_name == "bootargs"
        || prop_name == "clock-names"
        || prop_name == "clock-output-names"
        || prop_name == "label"
        || prop_name == "serial0"
    {
        let bytes = unsafe { core::slice::from_raw_parts(val_ptr as *const u8, len) };
        bytes[len - 1] == 0 && bytes[0].is_ascii_graphic()
    } else {
        false
    }
}
