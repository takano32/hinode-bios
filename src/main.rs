#![no_std]
#![no_main]

extern crate alloc;

pub mod arch;
pub mod boot;
pub mod dtb;
pub mod elf;
pub mod exceptions;
pub mod fat;
pub mod flash;
pub mod fw_cfg;
pub mod gic;
pub mod gpio;
pub mod mem;
pub mod pci;
pub mod pe;
pub mod post;
pub mod power;
pub mod rtc;
pub mod shell;
pub mod timer;
pub mod uart;
pub mod uefi;
pub mod virtio;

use core::arch::global_asm;

// Include low-level AArch64 assembly bootstrap and exception vector table
global_asm!(include_str!("entry.S"));

#[unsafe(no_mangle)]
pub extern "C" fn rust_start() -> ! {
    // 1. Initialize core hardware peripherals
    uart::init();
    timer::init();
    rtc::init();

    // 2. Display BIOS startup banner
    println!("--------------------------------------------------");
    println!("     Hinode BIOS (AArch64 Bare-Metal Firmware)    ");
    println!("--------------------------------------------------");
    println!(
        "Booting on CPU {} (Exception Level EL{})...",
        arch::cpu_id(),
        arch::current_el()
    );
    println!("Hello from Rust on AArch64 bare-metal QEMU!");

    // 3. Execute 10-stage Power-On Self-Test (POST)
    let _post_summary = post::run_post(true);

    // 4. If no fw_cfg command script is overriding boot flow, attempt auto-boot of attached media
    if fw_cfg::find_file("opt/hinode/cmd").is_none() {
        let dtb_info = dtb::parse();
        let ecam = dtb_info.pcie_ecam_base as usize;
        if let Ok(status) = boot::boot_auto(ecam) {
            println!("Auto-boot payload completed with status: 0x{:X}\n", status);
        }
    }

    // 5. Enter interactive BIOS monitor shell (or execute fw_cfg command script)
    shell::run_interactive_shell()
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("\n!!! PANIC !!!");
    println!("{}", info);
    if fw_cfg::find_file("opt/hinode/panic_off").is_some() {
        power::system_off();
    }
    loop {
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack));
        }
    }
}
