//! ARM Generic Interrupt Controller v2 (GICv2) driver (`0x0800_0000` / `0x0801_0000`).

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use crate::{arch, println};

pub const GICD_BASE: usize = 0x0800_0000;
pub const GICC_BASE: usize = 0x0801_0000;

// Distributor registers
const GICD_CTLR: *mut u32 = (GICD_BASE + 0x000) as *mut u32;
const GICD_TYPER: *const u32 = (GICD_BASE + 0x004) as *const u32;
const GICD_IIDR: *const u32 = (GICD_BASE + 0x008) as *const u32;
const GICD_ISENABLER0: *mut u32 = (GICD_BASE + 0x100) as *mut u32;
const GICD_ICPENDR0: *mut u32 = (GICD_BASE + 0x280) as *mut u32;
const GICD_SGIR: *mut u32 = (GICD_BASE + 0xF00) as *mut u32;

// CPU Interface registers
const GICC_CTLR: *mut u32 = (GICC_BASE + 0x000) as *mut u32;
const GICC_PMR: *mut u32 = (GICC_BASE + 0x004) as *mut u32;
const GICC_IAR: *const u32 = (GICC_BASE + 0x00C) as *const u32;
const GICC_EOIR: *mut u32 = (GICC_BASE + 0x010) as *mut u32;
const GICC_IIDR: *const u32 = (GICC_BASE + 0x0FC) as *const u32;

static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_IRQ_ID: AtomicU32 = AtomicU32::new(1023);
static QUIET_IRQ_TEST: AtomicBool = AtomicBool::new(false);

pub struct GicInfo {
    pub max_irqs: u32,
    pub cpu_interfaces: u32,
    pub implementer: u32,
    pub gicc_iidr: u32,
}

/// Initialize the GICv2 Distributor and CPU Interface.
pub fn init() -> GicInfo {
    unsafe {
        // Disable Distributor and CPU Interface during setup
        write_volatile(GICD_CTLR, 0);
        write_volatile(GICC_CTLR, 0);

        let typer = read_volatile(GICD_TYPER);
        let it_lines = typer & 0x1f;
        let max_irqs = 32 * (it_lines + 1);
        let cpu_interfaces = ((typer >> 5) & 0x7) + 1;
        let implementer = read_volatile(GICD_IIDR);
        let gicc_iidr = read_volatile(GICC_IIDR);

        // Set priority mask to allow all interrupt priorities (0xFF)
        write_volatile(GICC_PMR, 0xff);

        // Enable SGIs (IRQ 0..15)
        write_volatile(GICD_ICPENDR0, 0xffff);
        write_volatile(GICD_ISENABLER0, 0xffff);

        // Enable Distributor (Group 0/1) and CPU Interface
        write_volatile(GICD_CTLR, 1);
        write_volatile(GICC_CTLR, 1);

        GicInfo {
            max_irqs,
            cpu_interfaces,
            implementer,
            gicc_iidr,
        }
    }
}

/// Query GICv2 hardware parameters without resetting state.
pub fn info() -> GicInfo {
    unsafe {
        let typer = read_volatile(GICD_TYPER);
        let it_lines = typer & 0x1f;
        let max_irqs = 32 * (it_lines + 1);
        let cpu_interfaces = ((typer >> 5) & 0x7) + 1;
        let implementer = read_volatile(GICD_IIDR);
        let gicc_iidr = read_volatile(GICC_IIDR);
        GicInfo {
            max_irqs,
            cpu_interfaces,
            implementer,
            gicc_iidr,
        }
    }
}

pub fn irq_count() -> u64 {
    IRQ_COUNT.load(Ordering::SeqCst)
}

pub fn last_irq_id() -> u32 {
    LAST_IRQ_ID.load(Ordering::SeqCst)
}

/// Handle an incoming hardware interrupt from the AArch64 IRQ vector.
pub fn handle_irq() {
    unsafe {
        let iar = read_volatile(GICC_IAR);
        let irq_id = iar & 0x3ff;
        if irq_id < 1020 {
            LAST_IRQ_ID.store(irq_id, Ordering::SeqCst);
            let count = IRQ_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if !QUIET_IRQ_TEST.load(Ordering::SeqCst) {
                println!(
                    "  [IRQ]  Handled GICv2 Interrupt ID {} (IAR=0x{:08X}, total IRQs={})",
                    irq_id, iar, count
                );
            }
            write_volatile(GICC_EOIR, iar);
        }
    }
}

/// Trigger a Software Generated Interrupt (SGI #`sgi_id`, 0..15) to the requesting CPU
/// and verify that the GICv2 + AArch64 IRQ vector delivers and acknowledges it.
pub fn trigger_sgi_self_test(sgi_id: u32, verbose: bool) -> bool {
    let sgi = sgi_id & 0xf;
    let before = irq_count();
    QUIET_IRQ_TEST.store(!verbose, Ordering::SeqCst);

    unsafe {
        // TargetListFilter = 0b10 (send SGI to the requesting CPU interface only)
        let sgir_val = (0b10 << 24) | sgi;
        write_volatile(GICD_SGIR, sgir_val);
        arch::dsb_sy();
        arch::enable_irq();
        arch::isb();
        // Wait briefly for IRQ to fire
        for _ in 0..1000 {
            if irq_count() > before {
                break;
            }
            core::hint::spin_loop();
        }
        arch::disable_irq();
    }

    QUIET_IRQ_TEST.store(false, Ordering::SeqCst);
    irq_count() > before && last_irq_id() == sgi
}
