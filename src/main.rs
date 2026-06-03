#![no_std]
#![no_main]

use core::arch::global_asm;

// Include the assembly entry point
global_asm!(include_str!("entry.S"));

struct Uart;

impl core::fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        const UART_DR: *mut u32 = 0x0900_0000 as *mut u32;
        const UART_FR: *const u32 = 0x0900_0018 as *const u32;
        const TXFF: u32 = 1 << 5;

        for byte in s.bytes() {
            if byte == b'\n' {
                unsafe {
                    while (core::ptr::read_volatile(UART_FR) & TXFF) != 0 {}
                    core::ptr::write_volatile(UART_DR, b'\r' as u32);
                }
            }
            unsafe {
                while (core::ptr::read_volatile(UART_FR) & TXFF) != 0 {}
                core::ptr::write_volatile(UART_DR, byte as u32);
            }
        }
        Ok(())
    }
}

pub fn print_fmt(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let mut uart = Uart;
    let _ = uart.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print_fmt(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_start() -> ! {
    println!("--------------------------------------------------");
    println!("     Hinode BIOS (AArch64 Bare-Metal Firmware)    ");
    println!("--------------------------------------------------");
    println!("Booting on CPU 0...");
    println!("Hello from Rust on AArch64 bare-metal QEMU!");

    // Let's test that BSS clearing works
    static mut BSS_TEST: u64 = 0;
    unsafe {
        let bss_init = BSS_TEST;
        println!("BSS_TEST initial value (should be 0): {}", bss_init);
        BSS_TEST = 42;
        let bss_after = BSS_TEST;
        println!("BSS_TEST after assignment (should be 42): {}", bss_after);
    }

    // Let's test that DATA copying works
    static mut DATA_TEST: u64 = 0x123456789ABCDEF;
    unsafe {
        let data_init = DATA_TEST;
        println!("DATA_TEST initial value (should be 0x123456789ABCDEF): 0x{:X}", data_init);
        DATA_TEST = 0xDEADBEEF;
        let data_after = DATA_TEST;
        println!("DATA_TEST after assignment (should be 0xDEADBEEF): 0x{:X}", data_after);
    }

    // Trigger a panic to test our panic handler
    println!("Triggering a test panic to verify panic output...");
    panic!("This is a test panic from Hinode BIOS!");
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("\n!!! PANIC !!!");
    println!("{}", info);
    loop {}
}
