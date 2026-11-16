//! ARM PrimeCell PL011 UART driver for QEMU `virt` (`0x0900_0000`).

use core::fmt::{self, Write};
use core::ptr::{read_volatile, write_volatile};

pub const UART_BASE: usize = 0x0900_0000;

const UART_DR: *mut u32 = (UART_BASE + 0x000) as *mut u32;
const UART_ECR: *mut u32 = (UART_BASE + 0x004) as *mut u32;
const UART_FR: *const u32 = (UART_BASE + 0x018) as *const u32;
const UART_IBRD: *mut u32 = (UART_BASE + 0x024) as *mut u32;
const UART_FBRD: *mut u32 = (UART_BASE + 0x028) as *mut u32;
const UART_LCR_H: *mut u32 = (UART_BASE + 0x02C) as *mut u32;
const UART_CR: *mut u32 = (UART_BASE + 0x030) as *mut u32;
const UART_IMSC: *mut u32 = (UART_BASE + 0x038) as *mut u32;
const UART_ICR: *mut u32 = (UART_BASE + 0x044) as *mut u32;

// Flag Register bits
const FR_BUSY: u32 = 1 << 3;
const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;

// Line Control Register bits
const LCR_H_FEN: u32 = 1 << 4;
const LCR_H_WLEN_8: u32 = 0b11 << 5;

// Control Register bits
const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

static mut EARLY_RX_BUF: [u8; 64] = [0; 64];
static mut EARLY_RX_HEAD: usize = 0;
static mut EARLY_RX_TAIL: usize = 0;

/// Initialize the PL011 UART controller for 115200 8N1 with TX/RX enabled.
pub fn init() {
    unsafe {
        // Clear receive errors and pending interrupts without flushing RX holding register
        write_volatile(UART_ECR, 0);
        write_volatile(UART_ICR, 0x7ff);
        write_volatile(UART_IMSC, 0);

        // Configure 115200 baud divisor
        write_volatile(UART_IBRD, 13);
        write_volatile(UART_FBRD, 1);

        // Only enable 16-byte FIFO mode (FEN) if the 1-byte RX holding register is currently empty;
        // in QEMU's PL011 model, toggling FEN while a byte is already queued in RX resets read_count.
        if (read_volatile(UART_FR) & FR_RXFE) != 0 {
            write_volatile(UART_LCR_H, LCR_H_WLEN_8 | LCR_H_FEN);
        } else {
            write_volatile(UART_LCR_H, LCR_H_WLEN_8);
        }

        // Enable UART, Transmit, and Receive
        let _ = FR_BUSY;
        write_volatile(UART_CR, CR_UARTEN | TXE_OR_RXE);
    }
}

const TXE_OR_RXE: u32 = CR_TXE | CR_RXE;

/// Read the 32-bit ARM PrimeCell Peripheral ID (`0x00041011` for PL011).
pub fn peripheral_id() -> u32 {
    unsafe {
        let id0 = read_volatile((UART_BASE + 0xFE0) as *const u32) & 0xff;
        let id1 = read_volatile((UART_BASE + 0xFE4) as *const u32) & 0xff;
        let id2 = read_volatile((UART_BASE + 0xFE8) as *const u32) & 0xff;
        let id3 = read_volatile((UART_BASE + 0xFEC) as *const u32) & 0xff;
        id0 | (id1 << 8) | (id2 << 16) | (id3 << 24)
    }
}

/// Read the 32-bit ARM PrimeCell Cell ID (`0xB105F00D`).
pub fn primecell_id() -> u32 {
    unsafe {
        let c0 = read_volatile((UART_BASE + 0xFF0) as *const u32) & 0xff;
        let c1 = read_volatile((UART_BASE + 0xFF4) as *const u32) & 0xff;
        let c2 = read_volatile((UART_BASE + 0xFF8) as *const u32) & 0xff;
        let c3 = read_volatile((UART_BASE + 0xFFC) as *const u32) & 0xff;
        c0 | (c1 << 8) | (c2 << 16) | (c3 << 24)
    }
}

/// Write a single raw byte to the UART transmit FIFO (blocking until space is available).
#[inline]
pub fn putc_raw(byte: u8) {
    unsafe {
        while (read_volatile(UART_FR) & FR_TXFF) != 0 {}
        write_volatile(UART_DR, byte as u32);
    }
}

/// Write a character to the UART, converting `\n` into `\r\n`.
#[inline]
pub fn putc(byte: u8) {
    if byte == b'\n' {
        putc_raw(b'\r');
    }
    putc_raw(byte);
}

/// Write a string slice to the UART.
pub fn puts(s: &str) {
    for byte in s.bytes() {
        putc(byte);
    }
}

/// Try to read a single byte from the UART receive FIFO without blocking.
#[inline]
pub fn try_getc() -> Option<u8> {
    unsafe {
        if EARLY_RX_HEAD < EARLY_RX_TAIL {
            let b = EARLY_RX_BUF[EARLY_RX_HEAD];
            EARLY_RX_HEAD += 1;
            return Some(b);
        }
        if (read_volatile(UART_FR) & FR_RXFE) == 0 {
            Some((read_volatile(UART_DR) & 0xff) as u8)
        } else {
            None
        }
    }
}

/// Check whether a received byte is waiting in the UART receive FIFO.
#[inline]
pub fn has_rx_data() -> bool {
    unsafe { EARLY_RX_HEAD < EARLY_RX_TAIL || (read_volatile(UART_FR) & FR_RXFE) == 0 }
}

/// Read a single byte from the UART receive FIFO (blocking).
pub fn getc() -> u8 {
    loop {
        if let Some(b) = try_getc() {
            return b;
        }
        core::hint::spin_loop();
    }
}

/// Read an interactive line from the UART into `buf`, echoing characters and handling Backspace.
/// Returns `Some(len)` when Enter is pressed, or `None` if Ctrl+D (EOF) is received on an empty line.
pub fn read_line(buf: &mut [u8]) -> Option<usize> {
    let mut len = 0usize;
    loop {
        let b = getc();
        match b {
            b'\r' | b'\n' => {
                puts("\n");
                return Some(len);
            }
            0x04 => {
                // Ctrl+D (EOF)
                if len == 0 {
                    puts("\n");
                    return None;
                }
            }
            0x03 => {
                // Ctrl+C: cancel current line
                puts("^C\n");
                return Some(0);
            }
            0x15 => {
                // Ctrl+U: clear current line
                while len > 0 {
                    puts("\x08 \x08");
                    len -= 1;
                }
            }
            0x08 | 0x7f => {
                // Backspace / Delete
                if len > 0 {
                    len -= 1;
                    puts("\x08 \x08");
                }
            }
            0x1b => {
                // Consume ANSI escape sequences (e.g. arrow keys: ESC [ A/B/C/D)
                if let Some(b'[') = wait_short_rx() {
                    let _ = wait_short_rx();
                }
            }
            0x20..=0x7e => {
                if len < buf.len() {
                    buf[len] = b;
                    len += 1;
                    putc_raw(b);
                }
            }
            _ => {}
        }
    }
}

fn wait_short_rx() -> Option<u8> {
    for _ in 0..10_000 {
        if let Some(b) = try_getc() {
            return Some(b);
        }
        core::hint::spin_loop();
    }
    None
}

pub struct UartWriter;

impl Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(s);
        Ok(())
    }
}

pub fn print_fmt(args: fmt::Arguments) {
    let mut writer = UartWriter;
    let _ = writer.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::uart::print_fmt(core::format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", core::format_args!($($arg)*)));
}
