//! AArch64 Exception Vector Table handler, ESR_EL1 decoder, and trap dispatcher.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::{println, gic};

#[repr(C)]
pub struct ExceptionContext {
    pub gpr: [u64; 31], // x0..x30
    pub sp: u64,        // original stack pointer before exception frame
    pub elr: u64,       // Exception Link Register
    pub spsr: u64,      // Saved Program Status Register
    pub esr: u64,       // Exception Syndrome Register
    pub far: u64,       // Fault Address Register
}

static TRAP_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_BRK_IMM: AtomicU64 = AtomicU64::new(0);
static QUIET_TRAP_TEST: AtomicBool = AtomicBool::new(false);

pub fn trap_count() -> u64 {
    TRAP_COUNT.load(Ordering::SeqCst)
}

pub fn last_brk_imm() -> u64 {
    LAST_BRK_IMM.load(Ordering::SeqCst)
}

/// Trigger a recoverable software breakpoint (`BRK #0x42`) to test exception vectors.
/// If `verbose` is true, the exception handler prints details of the caught trap.
pub fn trigger_test_brk(verbose: bool) -> bool {
    let before = trap_count();
    QUIET_TRAP_TEST.store(!verbose, Ordering::SeqCst);
    unsafe {
        asm!("brk #0x42", options(nomem, nostack));
    }
    QUIET_TRAP_TEST.store(false, Ordering::SeqCst);
    let after = trap_count();
    after == before + 1 && last_brk_imm() == 0x42
}

fn vector_name(vector_id: u64) -> &'static str {
    match vector_id {
        0 => "Current EL with SP_EL0: Synchronous",
        1 => "Current EL with SP_EL0: IRQ",
        2 => "Current EL with SP_EL0: FIQ",
        3 => "Current EL with SP_EL0: SError",
        4 => "Current EL with SP_ELx: Synchronous",
        5 => "Current EL with SP_ELx: IRQ",
        6 => "Current EL with SP_ELx: FIQ",
        7 => "Current EL with SP_ELx: SError",
        8 => "Lower EL (AArch64): Synchronous",
        9 => "Lower EL (AArch64): IRQ",
        10 => "Lower EL (AArch64): FIQ",
        11 => "Lower EL (AArch64): SError",
        12 => "Lower EL (AArch32): Synchronous",
        13 => "Lower EL (AArch32): IRQ",
        14 => "Lower EL (AArch32): FIQ",
        15 => "Lower EL (AArch32): SError",
        _ => "Unknown Vector",
    }
}

fn decode_exception_class(ec: u64) -> &'static str {
    match ec {
        0x00 => "Unknown reason",
        0x01 => "Trapped WFI or WFE instruction",
        0x07 => "Access to SVE, Advanced SIMD, or FP functionality trapped",
        0x0E => "Illegal Execution state",
        0x11 => "SVC instruction execution in AArch32 state",
        0x15 => "SVC instruction execution in AArch64 state",
        0x16 => "HVC instruction execution in AArch64 state",
        0x17 => "SMC instruction execution in AArch64 state",
        0x18 => "Trapped MSR, MRS or System instruction execution in AArch64 state",
        0x19 => "Access to SVE functionality trapped",
        0x20 => "Instruction Abort from a lower Exception level",
        0x21 => "Instruction Abort taken without a change in Exception level",
        0x22 => "PC alignment fault",
        0x24 => "Data Abort from a lower Exception level",
        0x25 => "Data Abort taken without a change in Exception level",
        0x26 => "SP alignment fault",
        0x2C => "Trapped floating-point exception (AArch64)",
        0x2F => "SError interrupt",
        0x30 => "Breakpoint exception from a lower Exception level",
        0x31 => "Breakpoint exception taken without a change in Exception level",
        0x32 => "Software Step exception from a lower Exception level",
        0x33 => "Software Step exception taken without a change in Exception level",
        0x34 => "Watchpoint exception from a lower Exception level",
        0x35 => "Watchpoint exception taken without a change in Exception level",
        0x3C => "BRK instruction execution in AArch64 state",
        _ => "Reserved / Other Exception Class",
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_exception_handler(ctx: &mut ExceptionContext, vector_id: u64) {
    let is_irq = (vector_id % 4) == 1;
    if is_irq {
        gic::handle_irq();
        return;
    }

    let ec = (ctx.esr >> 26) & 0x3f;
    let iss = ctx.esr & 0x01ff_ffff;

    // Handle recoverable software breakpoint (BRK #imm16)
    if ec == 0x3C {
        let imm16 = iss & 0xffff;
        LAST_BRK_IMM.store(imm16, Ordering::SeqCst);
        let count = TRAP_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        if !QUIET_TRAP_TEST.load(Ordering::SeqCst) {
            println!("  [TRAP] Caught AArch64 BRK #0x{:X} at ELR=0x{:016X} (ESR=0x{:08X}, trap #{})",
                imm16, ctx.elr, ctx.esr, count);
            println!("  [TRAP] Advancing ELR by +4 bytes (0x{:016X}) and resuming execution.", ctx.elr + 4);
        }
        ctx.elr = ctx.elr.wrapping_add(4);
        return;
    }

    // Handle recoverable Supervisor Call (SVC #imm16)
    if ec == 0x15 {
        let imm16 = iss & 0xffff;
        let count = TRAP_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        if !QUIET_TRAP_TEST.load(Ordering::SeqCst) {
            println!("  [SVC]  Caught AArch64 SVC #0x{:X} at ELR=0x{:016X} (trap #{})",
                imm16, ctx.elr, count);
        }
        return;
    }

    // Unhandled / fatal exception: print full architectural dump
    println!("\n======================================================================");
    println!("               !!! UNHANDLED AARCH64 CPU EXCEPTION !!!                ");
    println!("======================================================================");
    println!("Vector   : [{}] {}", vector_id, vector_name(vector_id));
    println!("Class    : 0x{:02X} ({})", ec, decode_exception_class(ec));
    println!("ESR_ELx  : 0x{:016X} (ISS = 0x{:07X})", ctx.esr, iss);
    println!("ELR_ELx  : 0x{:016X}", ctx.elr);
    println!("FAR_ELx  : 0x{:016X}", ctx.far);
    println!("SPSR_ELx : 0x{:016X}", ctx.spsr);
    println!("SP       : 0x{:016X}", ctx.sp);
    println!("----------------------------------------------------------------------");
    for i in (0..30).step_by(2) {
        println!(
            "X{:02} : 0x{:016X}    X{:02} : 0x{:016X}",
            i, ctx.gpr[i], i + 1, ctx.gpr[i + 1]
        );
    }
    println!("X30 : 0x{:016X} (LR)", ctx.gpr[30]);
    println!("======================================================================");
    println!("System halted.");
    loop {
        unsafe {
            asm!("wfe", options(nomem, nostack));
        }
    }
}
