//! Power-On Self-Test (POST) hardware and firmware diagnostic suite.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, Ordering};
use crate::{arch, dtb, exceptions, fw_cfg, gic, gpio, mem, pci, power, println, rtc, timer, uart, virtio};

static mut BSS_TEST: u64 = 0;
static mut DATA_TEST: u64 = 0x0123_4567_89AB_CDEF;
static INITIAL_MEMORY_CHECK_DONE: AtomicBool = AtomicBool::new(false);

pub struct PostSummary {
    pub total_tests: usize,
    pub passed_tests: usize,
}

/// Run the full 10-stage BIOS Power-On Self-Test (POST) sequence.
pub fn run_post(verbose_sections: bool) -> PostSummary {
    let mut total = 0usize;
    let mut passed = 0usize;

    // 1. Memory Section Integrity (.bss zeroing & .data ROM-to-RAM relocation)
    total += 1;
    let mem_sections_ok = unsafe {
        if !INITIAL_MEMORY_CHECK_DONE.swap(true, Ordering::SeqCst) {
            let bss_init = read_volatile(&raw const BSS_TEST);
            if verbose_sections {
                println!("BSS_TEST initial value (should be 0): {}", bss_init);
            }
            write_volatile(&raw mut BSS_TEST, 42);
            let bss_after = read_volatile(&raw const BSS_TEST);
            if verbose_sections {
                println!("BSS_TEST after assignment (should be 42): {}", bss_after);
            }

            let data_init = read_volatile(&raw const DATA_TEST);
            if verbose_sections {
                println!(
                    "DATA_TEST initial value (should be 0x123456789ABCDEF): 0x{:X}",
                    data_init
                );
            }
            write_volatile(&raw mut DATA_TEST, 0xDEAD_BEEF);
            let data_after = read_volatile(&raw const DATA_TEST);
            if verbose_sections {
                println!(
                    "DATA_TEST after assignment (should be 0xDEADBEEF): 0x{:X}",
                    data_after
                );
            }
            bss_init == 0
                && bss_after == 42
                && data_init == 0x0123_4567_89AB_CDEF
                && data_after == 0xDEAD_BEEF
        } else {
            let bss_cur = read_volatile(&raw const BSS_TEST);
            let data_cur = read_volatile(&raw const DATA_TEST);
            bss_cur == 42 && data_cur == 0xDEAD_BEEF
        }
    };

    println!("\n--- Power-On Self-Test (POST) Diagnostics ---");
    if mem_sections_ok {
        passed += 1;
        println!(
            "  [ OK ] 01. Memory Sections    : .data ({} B) & .bss ({} B) verified in RAM",
            mem::edata() - mem::sdata(),
            mem::ebss() - mem::sbss()
        );
    } else {
        println!("  [FAIL] 01. Memory Sections    : .data / .bss verification failed");
    }

    // 2. CPU, Exception Level, & FP/SIMD Check
    total += 1;
    let cpu = arch::decode_cpu_info();
    let el = arch::current_el();
    let fpen = (arch::cpacr_el1() >> 20) & 0x3;
    if el >= 1 && fpen == 0b11 {
        passed += 1;
        println!(
            "  [ OK ] 02. CPU & Architecture : {} {} (r{}p{}, EL{}, PA={}b, FP/SIMD=ON)",
            cpu.implementer_name, cpu.part_name, cpu.variant, cpu.revision, el, cpu.pa_bits
        );
    } else {
        println!("  [FAIL] 02. CPU & Architecture : Unexpected EL={} or FPEN={}", el, fpen);
    }

    // 3. Heap Allocator Check (`alloc`)
    total += 1;
    let heap_ok = {
        let b = Box::new(0xCAFE_BABE_1234_5678u64);
        let mut v = Vec::with_capacity(16);
        for i in 0..16u64 {
            v.push(i * 3);
        }
        let s = String::from("Hinode BIOS Heap OK");
        *b == 0xCAFE_BABE_1234_5678 && v.len() == 16 && v[15] == 45 && s.len() == 19
    };
    let hstats = mem::heap_stats();
    if heap_ok {
        passed += 1;
        println!(
            "  [ OK ] 03. Heap Allocator     : {} KB arena @ 0x{:08X} (alloc/dealloc verified)",
            hstats.total_bytes / 1024,
            hstats.start
        );
    } else {
        println!("  [FAIL] 03. Heap Allocator     : Allocation self-test failed");
    }

    // 4. Exception Vector Table & Trap Handling (`BRK #0x42`)
    total += 1;
    let vbar = arch::vbar_el1();
    let brk_ok = (vbar & 0x7ff) == 0 && exceptions::trigger_test_brk(false);
    if brk_ok {
        passed += 1;
        println!(
            "  [ OK ] 04. Exception Vectors  : VBAR_EL1=0x{:08X}, BRK #0x42 trap & resume OK",
            vbar
        );
    } else {
        println!("  [FAIL] 04. Exception Vectors  : VBAR_EL1=0x{:08X} trap test failed", vbar);
    }

    // 5. ARMv8 Generic Timer & PL011 UART Check
    total += 1;
    let freq = timer::frequency_hz();
    let t0 = timer::counter_ticks();
    timer::delay_us(50);
    let t1 = timer::counter_ticks();
    let uart_pid = uart::peripheral_id();
    if freq > 0 && t1 > t0 && (uart_pid & 0x000f_ffff) == 0x0004_1011 {
        passed += 1;
        println!(
            "  [ OK ] 05. Timer & UART0      : CNTFRQ={} Hz ({} MHz), PL011 ID=0x{:08X}",
            freq,
            freq / 1_000_000,
            uart_pid
        );
    } else {
        println!("  [FAIL] 05. Timer & UART0      : freq={} t0={} t1={}", freq, t0, t1);
    }

    // 6. PL031 Real-Time Clock & Power Event Controller (PL061 GPIO / ACPI GED) Check
    total += 1;
    let rtc_pid = rtc::peripheral_id();
    let now = rtc::now_utc();
    if (rtc_pid & 0x000f_ffff) == 0x0004_1031 && now.year >= 1970 {
        passed += 1;
        if let Some(gpio_pid) = gpio::peripheral_id() {
            println!(
                "  [ OK ] 06. RTC & GPIO         : PL031 ({}) & PL061 (ID=0x{:08X})",
                now, gpio_pid
            );
        } else {
            let ged = gpio::read_acpi_ged_status();
            println!(
                "  [ OK ] 06. RTC & Power Events : PL031 ({}) & ACPI GED @ 0x09080000 (status=0x{:X})",
                now, ged
            );
        }
    } else {
        println!("  [FAIL] 06. RTC & Power Events : RTC ID=0x{:08X}, Year={}", rtc_pid, now.year);
    }

    // 7. ARM GICv2 Interrupt Controller & SGI Interrupt Self-Test
    total += 1;
    let gic_info = gic::init();
    let sgi_ok = gic::trigger_sgi_self_test(1, false);
    if gic_info.max_irqs >= 32 && sgi_ok {
        passed += 1;
        println!(
            "  [ OK ] 07. GICv2 Interrupts   : {} IRQs, {} CPU(s), SGI #1 IRQ delivery verified",
            gic_info.max_irqs, gic_info.cpu_interfaces
        );
    } else {
        println!("  [FAIL] 07. GICv2 Interrupts   : SGI self-test failed");
    }

    // 8. Flattened Device Tree (FDT / DTB) Check
    total += 1;
    let dtb_info = dtb::parse();
    if dtb_info.valid && dtb_info.ram_size > 0 {
        passed += 1;
        println!(
            "  [ OK ] 08. Device Tree (FDT)  : 0x{:08X} ({} B, {} MB RAM, {} CPU, {} nodes)",
            dtb_info.base_addr,
            dtb_info.total_size,
            dtb_info.ram_size / (1024 * 1024),
            dtb_info.cpu_count,
            dtb_info.node_count
        );
    } else {
        println!("  [FAIL] 08. Device Tree (FDT)  : Invalid DTB at 0x{:08X}", dtb_info.base_addr);
    }

    // 9. QEMU `fw_cfg` & PSCI Power Management Check
    total += 1;
    let (fw_present, fw_dma, _fw_rev) = fw_cfg::probe();
    let (psci_maj, psci_min) = power::version();
    if fw_present && psci_maj >= 1 {
        passed += 1;
        println!(
            "  [ OK ] 09. fw_cfg & PSCI      : QEMU fw_cfg (DMA={}), PSCI v{}.{} ({})",
            if fw_dma { "ON" } else { "OFF" },
            psci_maj,
            psci_min,
            dtb_info.psci_method
        );
    } else {
        println!("  [FAIL] 09. fw_cfg & PSCI      : fw_cfg={} psci={}.{}", fw_present, psci_maj, psci_min);
    }

    // 10. PCIe ECAM & VirtIO Bus Enumeration
    total += 1;
    let ecam = dtb_info.pcie_ecam_base as usize;
    let pci_devs = pci::enumerate_and_allocate(ecam);
    let virtio_mmio = virtio::scan_mmio_devices();
    let blk_devs = virtio::find_block_devices(ecam);
    if !pci_devs.is_empty() {
        passed += 1;
        println!(
            "  [ OK ] 10. PCIe & VirtIO Bus  : {} PCIe dev(s), {} VirtIO-MMIO, {} Block drive(s)",
            pci_devs.len(),
            virtio_mmio.len(),
            blk_devs.len()
        );
    } else {
        println!("  [FAIL] 10. PCIe & VirtIO Bus  : No PCIe Host Bridge found at 0x{:010X}", ecam);
    }

    println!(
        "--- POST Complete: {}/{} checks passed in {} ms ---\n",
        passed,
        total,
        timer::uptime_ms()
    );

    PostSummary {
        total_tests: total,
        passed_tests: passed,
    }
}
