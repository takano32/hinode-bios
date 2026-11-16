//! ARM Power State Coordination Interface (PSCI) driver for system shutdown & reboot.
//! Automatically selects `HVC #0` or `SMC #0` based on the `/psci` node in the Device Tree.

use core::arch::asm;
use crate::dtb;

const PSCI_0_2_FN_PSCI_VERSION: u64 = 0x8400_0000;
const PSCI_0_2_FN_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_0_2_FN_SYSTEM_RESET: u64 = 0x8400_0009;

#[inline(always)]
fn psci_hvc(func_id: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "hvc #0",
            inout("x0") func_id => ret,
            in("x1") arg0,
            in("x2") arg1,
            in("x3") arg2,
            options(nomem, nostack)
        );
    }
    ret
}

#[inline(always)]
fn psci_smc(func_id: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "smc #0",
            inout("x0") func_id => ret,
            in("x1") arg0,
            in("x2") arg1,
            in("x3") arg2,
            options(nomem, nostack)
        );
    }
    ret
}

#[inline]
fn psci_call(func_id: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let d = dtb::parse();
    if d.psci_method == "smc" {
        psci_smc(func_id, arg0, arg1, arg2)
    } else {
        psci_hvc(func_id, arg0, arg1, arg2)
    }
}

/// Query the PSCI implementation version `(major, minor)`.
pub fn version() -> (u16, u16) {
    let v = psci_call(PSCI_0_2_FN_PSCI_VERSION, 0, 0, 0) as u32;
    let major = ((v >> 16) & 0xffff) as u16;
    let minor = (v & 0xffff) as u16;
    (major, minor)
}

/// Power off the system cleanly via PSCI `SYSTEM_OFF` (terminates QEMU with exit code 0).
pub fn system_off() -> ! {
    let _ = psci_call(PSCI_0_2_FN_SYSTEM_OFF, 0, 0, 0);
    loop {
        unsafe {
            asm!("wfe", options(nomem, nostack));
        }
    }
}

/// Reboot the system via PSCI `SYSTEM_RESET`.
pub fn system_reset() -> ! {
    let _ = psci_call(PSCI_0_2_FN_SYSTEM_RESET, 0, 0, 0);
    loop {
        unsafe {
            asm!("wfe", options(nomem, nostack));
        }
    }
}
