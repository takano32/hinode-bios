//! ARMv8 Generic Timer driver (`CNTFRQ_EL0`, `CNTPCT_EL0`).

use core::arch::asm;

static mut BOOT_TICKS: u64 = 0;

/// Record the initial physical counter tick at boot.
pub fn init() {
    unsafe {
        BOOT_TICKS = counter_ticks();
    }
}

/// Read the system counter frequency in Hz (`CNTFRQ_EL0`).
#[inline(always)]
pub fn frequency_hz() -> u64 {
    let freq: u64;
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack, preserves_flags));
    }
    freq
}

/// Read the current 64-bit physical counter value (`CNTPCT_EL0`).
#[inline(always)]
pub fn counter_ticks() -> u64 {
    let pct: u64;
    unsafe {
        asm!("isb", "mrs {}, cntpct_el0", out(reg) pct, options(nomem, nostack, preserves_flags));
    }
    pct
}

/// Return elapsed ticks since `timer::init()`.
pub fn elapsed_ticks() -> u64 {
    let boot = unsafe { BOOT_TICKS };
    counter_ticks().saturating_sub(boot)
}

/// Return system uptime in microseconds.
pub fn uptime_us() -> u64 {
    let freq = frequency_hz();
    if freq == 0 {
        return 0;
    }
    let ticks = elapsed_ticks() as u128;
    ((ticks * 1_000_000) / (freq as u128)) as u64
}

/// Return system uptime in milliseconds.
pub fn uptime_ms() -> u64 {
    uptime_us() / 1_000
}

/// Busy-wait for `us` microseconds using the hardware counter.
pub fn delay_us(us: u64) {
    let freq = frequency_hz();
    if freq == 0 {
        for _ in 0..(us * 100) {
            core::hint::spin_loop();
        }
        return;
    }
    let start = counter_ticks();
    let delta = ((us as u128 * freq as u128) / 1_000_000) as u64;
    while counter_ticks().wrapping_sub(start) < delta {
        core::hint::spin_loop();
    }
}

/// Busy-wait for `ms` milliseconds.
pub fn delay_ms(ms: u64) {
    delay_us(ms.saturating_mul(1_000));
}
