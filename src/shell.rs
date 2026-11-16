//! Hinode BIOS Interactive Monitor Shell and Command Dispatcher.

extern crate alloc;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use crate::{
    arch, boot, dtb, exceptions, fat, flash, fw_cfg, gic, gpio, mem, pci, post, power, print,
    println, rtc, timer, uart, virtio,
};

/// Execute a single command line (which may contain `;`-separated commands).
pub fn execute_command_line(line: &str) {
    for sub in line.split(';') {
        let trimmed = sub.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        execute_single_command(trimmed);
    }
}

fn execute_single_command(cmd_line: &str) {
    let mut parts = cmd_line.split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return,
    };
    let args: Vec<&str> = parts.collect();

    match cmd {
        "help" | "?" => cmd_help(),
        "info" | "sysinfo" => cmd_sysinfo(),
        "post" => {
            let _ = post::run_post(false);
        }
        "dtb" => {
            let d = dtb::parse();
            if args.first().copied() == Some("full") {
                dtb::dump_tree(d.base_addr, 8);
            } else {
                dtb::print_summary(&d);
            }
        }
        "memmap" => {
            let d = dtb::parse();
            mem::print_memory_map(d.ram_base, d.ram_size);
        }
        "memtest" => {
            let kb = args
                .first()
                .and_then(|s| parse_u64(s))
                .unwrap_or(64)
                .clamp(4, 16384) as usize;
            let test_base = 0x4060_0000usize;
            let size = kb * 1024;
            println!(
                "Running 4-pattern RAM test on 0x{:08X} .. 0x{:08X} ({} KB)...",
                test_base,
                test_base + size,
                kb
            );
            let rep = mem::run_memtest(test_base, size);
            match rep.failed_addr {
                None => println!(
                    "  [PASS] Verified {} KB ({} patterns) with 0 errors.",
                    kb, rep.patterns_tested
                ),
                Some(bad) => println!("  [FAIL] Memory error detected at 0x{:08X}!", bad),
            }
        }
        "pci" => {
            let d = dtb::parse();
            pci::print_devices(d.pcie_ecam_base as usize);
        }
        "virtio" => {
            let d = dtb::parse();
            virtio::print_devices(d.pcie_ecam_base as usize);
        }
        "fwcfg" => fw_cfg::print_info(),
        "flash" => flash::print_info(),
        "devices" => {
            let d = dtb::parse();
            pci::print_devices(d.pcie_ecam_base as usize);
            println!();
            virtio::print_devices(d.pcie_ecam_base as usize);
            println!();
            fw_cfg::print_info();
            println!();
            flash::print_info();
        }
        "ls" => {
            let path = args.first().copied().unwrap_or("/");
            cmd_ls(path);
        }
        "cat" => {
            if let Some(&path) = args.first() {
                cmd_cat(path);
            } else {
                println!("Usage: cat <path>  (e.g. cat /STARTUP.NSH)");
            }
        }
        "demo" => {
            let target = args.first().copied().unwrap_or("all");
            match target {
                "elf" => {
                    if let Err(e) = boot::run_demo_elf() {
                        println!("Demo ELF error: {}", e);
                    }
                }
                "efi" | "uefi" => {
                    if let Err(e) = boot::run_demo_efi() {
                        println!("Demo UEFI error: {}", e);
                    }
                }
                _ => {
                    if let Err(e) = boot::run_demo_elf() {
                        println!("Demo ELF error: {}", e);
                    }
                    println!();
                    if let Err(e) = boot::run_demo_efi() {
                        println!("Demo UEFI error: {}", e);
                    }
                }
            }
        }
        "boot" => {
            let d = dtb::parse();
            let ecam = d.pcie_ecam_base as usize;
            match args.first().copied() {
                None | Some("auto") => match boot::boot_auto(ecam) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Auto-boot notice: {}", e);
                        println!("Running built-in ROM UEFI & ELF64 boot verification instead...");
                        let _ = boot::run_demo_elf();
                        let _ = boot::run_demo_efi();
                    }
                },
                Some("elf") => {
                    let _ = boot::run_demo_elf();
                }
                Some("efi") | Some("uefi") => {
                    let _ = boot::run_demo_efi();
                }
                Some(path) => {
                    if let Err(e) = boot::boot_disk_file(ecam, path) {
                        println!("Boot failed for '{}': {}", path, e);
                    }
                }
            }
        }
        "regs" => cmd_regs(),
        "md" | "dump" => {
            if let Some(addr_str) = args.first() {
                if let Some(addr) = parse_u64(addr_str) {
                    let bytes = args
                        .get(1)
                        .and_then(|s| parse_u64(s))
                        .unwrap_or(64)
                        .clamp(16, 4096) as usize;
                    cmd_hexdump(addr as usize, bytes);
                } else {
                    println!("Invalid address: {}", addr_str);
                }
            } else {
                println!("Usage: md <hex_addr> [byte_count]  (e.g. md 0x40000000 64)");
            }
        }
        "mw" => {
            if args.len() >= 2 {
                if let (Some(addr), Some(val)) = (parse_u64(args[0]), parse_u64(args[1])) {
                    unsafe {
                        write_volatile(addr as *mut u64, val);
                    }
                    println!("Wrote 0x{:016X} -> [0x{:012X}]", val, addr);
                } else {
                    println!("Invalid hex arguments.");
                }
            } else {
                println!("Usage: mw <hex_addr> <u64_val>");
            }
        }
        "date" | "time" | "uptime" => {
            let now = rtc::now_utc();
            let up_ms = timer::uptime_ms();
            let up_us = timer::uptime_us();
            println!("RTC Date/Time : {} (epoch: {} s)", now, rtc::unix_timestamp());
            println!(
                "System Uptime : {}.{:03} s ({} us, {} ticks @ {} Hz)",
                up_ms / 1000,
                up_ms % 1000,
                up_us,
                timer::elapsed_ticks(),
                timer::frequency_hz()
            );
        }
        "trap" => {
            println!("Triggering AArch64 BRK #0x42 software breakpoint exception...");
            let ok = exceptions::trigger_test_brk(true);
            println!("Trap handler returned: {}", if ok { "SUCCESS" } else { "FAILED" });
        }
        "irq" => {
            println!("Triggering GICv2 Software Generated Interrupt (SGI #1)...");
            let ok = gic::trigger_sgi_self_test(1, true);
            println!("IRQ handler returned: {}", if ok { "SUCCESS" } else { "FAILED" });
        }
        "panic" => {
            println!("Triggering a test panic to verify panic output...");
            panic!("This is a test panic from Hinode BIOS!");
        }
        "reboot" | "reset" => {
            println!("Rebooting system via PSCI SYSTEM_RESET...");
            power::system_reset();
        }
        "shutdown" | "poweroff" | "halt" | "exit" | "quit" => {
            println!("Powering off system via PSCI SYSTEM_OFF...");
            power::system_off();
        }
        _ => {
            println!("Unknown command: '{}'. Type 'help' for available commands.", cmd);
        }
    }
}

/// Start the interactive BIOS shell over PL011 UART0.
pub fn run_interactive_shell() -> ! {
    // 1. Check if QEMU fw_cfg provides an automated command script (`opt/hinode/cmd`)
    if let Some(script_bytes) = fw_cfg::read_file("opt/hinode/cmd") {
        if let Ok(script) = core::str::from_utf8(&script_bytes) {
            let clean = script.trim_matches(char::from(0)).trim();
            if !clean.is_empty() {
                println!("Executing fw_cfg command script (opt/hinode/cmd): \"{}\"", clean);
                execute_command_line(clean);
            }
        }
    }

    println!("Entering Hinode BIOS Interactive Monitor. Type 'help' for commands.");
    let mut line_buf = [0u8; 256];
    loop {
        print!("hinode> ");
        match uart::read_line(&mut line_buf) {
            Some(len) => {
                if len == 0 {
                    continue;
                }
                if let Ok(line_str) = core::str::from_utf8(&line_buf[..len]) {
                    execute_command_line(line_str);
                }
            }
            None => {
                // EOF (Ctrl+D): cleanly shut down VM
                println!("EOF received. Powering off via PSCI SYSTEM_OFF...");
                power::system_off();
            }
        }
    }
}

fn cmd_help() {
    println!("Hinode BIOS Monitor Commands:");
    println!("  help              Show this command list");
    println!("  info / sysinfo    Display CPU, hardware, memory, RTC, and firmware summary");
    println!("  post              Re-run the 10-stage Power-On Self-Test (POST) suite");
    println!("  dtb [full]        Display Flattened Device Tree summary (or full tree)");
    println!("  memmap            Display physical address space and BIOS section map");
    println!("  memtest [kb]      Run 4-pattern RAM diagnostic test (default: 64 KB)");
    println!("  devices           Scan all buses (PCIe ECAM, VirtIO, fw_cfg, NOR Flash)");
    println!("  pci               Enumerate PCIe ECAM Bus 0 devices and BARs");
    println!("  virtio            Scan VirtIO MMIO transports and Block storage drives");
    println!("  fwcfg             List QEMU fw_cfg firmware configuration files");
    println!("  flash             Inspect CFI NOR Flash Bank 0 & Bank 1");
    println!("  ls [path]         List directory on VirtIO FAT12/16/32 disk (default: /)");
    println!("  cat <path>        Display file contents from VirtIO FAT12/16/32 disk");
    println!("  boot [path|auto]  Boot payload from fw_cfg, VirtIO FAT disk, or Flash");
    println!("  demo [all|elf|efi] Run built-in AArch64 ELF64 and PE32+ UEFI demo payloads");
    println!("  regs              Dump AArch64 architectural and system registers");
    println!("  md <addr> [len]   Hex + ASCII memory dump (e.g. md 0x40000000 64)");
    println!("  mw <addr> <val>   Write 64-bit value to physical memory");
    println!("  date / uptime     Display PL031 RTC UTC date/time and Generic Timer uptime");
    println!("  trap              Trigger recoverable BRK #0x42 exception self-test");
    println!("  irq               Trigger GICv2 SGI #1 interrupt delivery self-test");
    println!("  panic             Trigger test panic handler");
    println!("  reboot / reset    Reboot machine via PSCI SYSTEM_RESET");
    println!("  shutdown / exit   Power off machine cleanly via PSCI SYSTEM_OFF");
}

fn cmd_sysinfo() {
    let cpu = arch::decode_cpu_info();
    let d = dtb::parse();
    let gic_info = gic::info();
    let hstats = mem::heap_stats();
    let (psci_maj, psci_min) = power::version();
    let now = rtc::now_utc();
    let up_ms = timer::uptime_ms();

    println!("======================================================================");
    println!("                  Hinode BIOS System Information                      ");
    println!("======================================================================");
    println!("Firmware      : Hinode BIOS v0.2.0 (AArch64 Bare-Metal Rust Firmware)");
    println!("Machine Model : {} ({})", d.model, d.compatible);
    println!(
        "Processor     : {} {} (r{}p{}, {} core(s))",
        cpu.implementer_name, cpu.part_name, cpu.variant, cpu.revision, d.cpu_count
    );
    println!(
        "Execution     : Exception Level EL{}, Physical Address Width {} bits",
        arch::current_el(),
        cpu.pa_bits
    );
    println!(
        "Caches        : L1 I-Cache {} KB, L1 D-Cache {} KB",
        d.icache_size / 1024,
        d.dcache_size / 1024
    );
    println!(
        "System RAM    : {} MB (0x{:08X} .. 0x{:08X})",
        d.ram_size / (1024 * 1024),
        d.ram_base,
        d.ram_base + d.ram_size
    );
    println!(
        "BIOS Heap     : {} B used / {} KB total ({} active allocations)",
        hstats.used_bytes,
        hstats.total_bytes / 1024,
        hstats.active_allocs
    );
    println!(
        "Interrupts    : ARM GICv2 ({} IRQs, {} handled, traps caught: {})",
        gic_info.max_irqs,
        gic::irq_count(),
        exceptions::trap_count()
    );
    println!(
        "System Timer  : {} Hz ({} MHz), Uptime: {}.{:03} s",
        timer::frequency_hz(),
        timer::frequency_hz() / 1_000_000,
        up_ms / 1000,
        up_ms % 1000
    );
    println!("RTC Clock     : {} (PL031 @ 0x{:08X})", now, rtc::RTC_BASE);
    if gpio::is_pl061_present() {
        println!(
            "GPIO Status   : PL061 @ 0x{:08X} (Pins=0x{:02X}, Dir=0x{:02X})",
            gpio::GPIO_BASE,
            gpio::read_pins(),
            gpio::read_direction()
        );
    } else {
        println!(
            "Power Events  : ACPI GED @ 0x{:08X} (EventStatus=0x{:08X})",
            gpio::ACPI_GED_BASE,
            gpio::read_acpi_ged_status()
        );
    }
    println!("Power (PSCI)  : v{}.{} via {}", psci_maj, psci_min, d.psci_method);
    println!("======================================================================");
}

fn cmd_regs() {
    println!("AArch64 System Registers:");
    println!("  CurrentEL         : EL{} (raw=0x{:X})", arch::current_el(), (arch::current_el() as u64) << 2);
    println!("  MPIDR_EL1         : 0x{:016X} (CPU ID = {})", arch::mpidr_el1(), arch::cpu_id());
    println!("  MIDR_EL1          : 0x{:016X}", arch::midr_el1());
    println!("  SCTLR_EL1         : 0x{:016X}", arch::sctlr_el1());
    println!("  VBAR_EL1          : 0x{:016X}", arch::vbar_el1());
    println!("  CPACR_EL1         : 0x{:016X}", arch::cpacr_el1());
    println!("  DAIF              : 0x{:016X}", arch::daif());
    println!("  SP                : 0x{:016X}", arch::sp());
    println!("  CNTFRQ_EL0        : 0x{:016X} ({} Hz)", timer::frequency_hz(), timer::frequency_hz());
    println!("  CNTPCT_EL0        : 0x{:016X}", timer::counter_ticks());
    println!("  ID_AA64PFR0_EL1   : 0x{:016X}", arch::id_aa64pfr0_el1());
    println!("  ID_AA64MMFR0_EL1  : 0x{:016X}", arch::id_aa64mmfr0_el1());
    println!("  ID_AA64ISAR0_EL1  : 0x{:016X}", arch::id_aa64isar0_el1());
}

fn cmd_hexdump(base: usize, len: usize) {
    let aligned_len = (len + 15) & !15;
    for row in (0..aligned_len).step_by(16) {
        let addr = base + row;
        print!("  {:012X}: ", addr);
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = unsafe { read_volatile((addr + i) as *const u8) };
            print!("{:02X} ", bytes[i]);
            if i == 7 {
                print!(" ");
            }
        }
        print!(" |");
        for &b in &bytes {
            let ch = if (0x20..=0x7e).contains(&b) {
                b as char
            } else {
                '.'
            };
            print!("{}", ch);
        }
        println!("|");
    }
}

fn cmd_ls(path: &str) {
    let d = dtb::parse();
    let blk_devs = virtio::find_block_devices(d.pcie_ecam_base as usize);
    let Some(blk) = blk_devs.first() else {
        println!("No VirtIO block device attached.");
        return;
    };
    let vol = match fat::mount(blk) {
        Ok(v) => v,
        Err(e) => {
            println!("FAT mount error: {}", e);
            return;
        }
    };
    println!(
        "Volume {:?} ({:?}, part_lba={}) — Directory listing of '{}':",
        vol.label_str(),
        vol.fat_type,
        vol.part_lba,
        path
    );
    match vol.list_dir(blk, path) {
        Ok(entries) => {
            if entries.is_empty() {
                println!("  (empty directory)");
            } else {
                for ent in &entries {
                    if ent.is_dir {
                        println!("  <DIR>          {}/", ent.name);
                    } else {
                        println!("  {:>10} B   {}", ent.size, ent.name);
                    }
                }
            }
        }
        Err(e) => println!("  Error listing '{}': {}", path, e),
    }
}

fn cmd_cat(path: &str) {
    let d = dtb::parse();
    let blk_devs = virtio::find_block_devices(d.pcie_ecam_base as usize);
    let Some(blk) = blk_devs.first() else {
        println!("No VirtIO block device attached.");
        return;
    };
    let vol = match fat::mount(blk) {
        Ok(v) => v,
        Err(e) => {
            println!("FAT mount error: {}", e);
            return;
        }
    };
    match vol.read_file(blk, path) {
        Ok(data) => {
            if let Ok(text) = core::str::from_utf8(&data) {
                println!("{}", text);
            } else {
                println!(
                    "Binary file '{}' ({} bytes), showing first 128 bytes:",
                    path,
                    data.len()
                );
                cmd_hexdump(data.as_ptr() as usize, data.len().min(128));
            }
        }
        Err(e) => println!("Error reading '{}': {}", path, e),
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if trimmed.chars().any(|c| ('a'..='f').contains(&c) || ('A'..='F').contains(&c)) {
        u64::from_str_radix(trimmed, 16).ok()
    } else {
        trimmed.parse::<u64>().ok()
    }
}
