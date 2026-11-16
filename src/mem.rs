//! Physical memory layout, global heap allocator (`alloc`), and RAM diagnostic tests.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::println;

unsafe extern "C" {
    static _rom_start: u8;
    static _rom_end: u8;
    static _dtb_start: u8;
    static _dtb_max_end: u8;
    static _sdata: u8;
    static _edata: u8;
    static _sbss: u8;
    static _ebss: u8;
    static _stack_bottom: u8;
    static _stack_top: u8;
    static _dma_start: u8;
    static _dma_end: u8;
    static _heap_start: u8;
    static _heap_end: u8;
    static _payload_start: u8;
}

#[inline]
pub fn rom_start() -> usize {
    (&raw const _rom_start) as usize
}

#[inline]
pub fn rom_end() -> usize {
    (&raw const _rom_end) as usize
}

#[inline]
pub fn dtb_start() -> usize {
    (&raw const _dtb_start) as usize
}

#[inline]
pub fn dtb_max_end() -> usize {
    (&raw const _dtb_max_end) as usize
}

#[inline]
pub fn sdata() -> usize {
    (&raw const _sdata) as usize
}

#[inline]
pub fn edata() -> usize {
    (&raw const _edata) as usize
}

#[inline]
pub fn sbss() -> usize {
    (&raw const _sbss) as usize
}

#[inline]
pub fn ebss() -> usize {
    (&raw const _ebss) as usize
}

#[inline]
pub fn stack_bottom() -> usize {
    (&raw const _stack_bottom) as usize
}

#[inline]
pub fn stack_top() -> usize {
    (&raw const _stack_top) as usize
}

#[inline]
pub fn dma_start() -> usize {
    (&raw const _dma_start) as usize
}

#[inline]
pub fn dma_end() -> usize {
    (&raw const _dma_end) as usize
}

#[inline]
pub fn heap_start() -> usize {
    (&raw const _heap_start) as usize
}

#[inline]
pub fn heap_end() -> usize {
    (&raw const _heap_end) as usize
}

#[inline]
pub fn payload_start() -> usize {
    (&raw const _payload_start) as usize
}

// ============================================================================
// Bare-Metal Heap Allocator (Free-List + Bump Arena)
// ============================================================================

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct AllocatorInner {
    bump_offset: usize,
    free_head: *mut FreeBlock,
    used_bytes: usize,
    alloc_count: usize,
}

pub struct BiosAllocator {
    locked: AtomicUsize,
    inner: UnsafeCell<AllocatorInner>,
}

unsafe impl Sync for BiosAllocator {}

impl BiosAllocator {
    pub const fn new() -> Self {
        Self {
            locked: AtomicUsize::new(0),
            inner: UnsafeCell::new(AllocatorInner {
                bump_offset: 0,
                free_head: core::ptr::null_mut(),
                used_bytes: 0,
                alloc_count: 0,
            }),
        }
    }

    #[inline]
    fn lock(&self) {
        while self
            .locked
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    #[inline]
    fn unlock(&self) {
        self.locked.store(0, Ordering::Release);
    }
}

#[inline]
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

unsafe impl GlobalAlloc for BiosAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock();
        let inner = unsafe { &mut *self.inner.get() };

        let min_size = core::mem::size_of::<FreeBlock>();
        let align = layout.align().max(core::mem::align_of::<FreeBlock>());
        let size = align_up(layout.size().max(min_size), align);

        // 1. Search free list for a matching block with proper alignment
        let mut prev: *mut FreeBlock = core::ptr::null_mut();
        let mut curr = inner.free_head;
        while !curr.is_null() {
            let addr = curr as usize;
            let block_size = unsafe { (*curr).size };
            if addr % align == 0 && block_size >= size {
                let next = unsafe { (*curr).next };
                if prev.is_null() {
                    inner.free_head = next;
                } else {
                    unsafe { (*prev).next = next };
                }
                inner.used_bytes += block_size;
                inner.alloc_count += 1;
                self.unlock();
                return addr as *mut u8;
            }
            prev = curr;
            curr = unsafe { (*curr).next };
        }

        // 2. Bump allocate from [_heap_start .. _heap_end]
        let base = heap_start();
        let end = heap_end();
        let current = base + inner.bump_offset;
        let aligned = align_up(current, align);
        let new_end = aligned.saturating_add(size);

        if new_end > end {
            self.unlock();
            return core::ptr::null_mut();
        }

        inner.bump_offset = new_end - base;
        inner.used_bytes += size;
        inner.alloc_count += 1;
        self.unlock();
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        self.lock();
        let inner = unsafe { &mut *self.inner.get() };

        let min_size = core::mem::size_of::<FreeBlock>();
        let align = layout.align().max(core::mem::align_of::<FreeBlock>());
        let size = align_up(layout.size().max(min_size), align);

        let block = ptr as *mut FreeBlock;
        unsafe {
            (*block).size = size;
            (*block).next = inner.free_head;
        }
        inner.free_head = block;
        inner.used_bytes = inner.used_bytes.saturating_sub(size);
        inner.alloc_count = inner.alloc_count.saturating_sub(1);
        self.unlock();
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: BiosAllocator = BiosAllocator::new();

pub struct HeapStats {
    pub start: usize,
    pub end: usize,
    pub total_bytes: usize,
    pub used_bytes: usize,
    pub bump_bytes: usize,
    pub active_allocs: usize,
}

pub fn heap_stats() -> HeapStats {
    GLOBAL_ALLOCATOR.lock();
    let inner = unsafe { &*GLOBAL_ALLOCATOR.inner.get() };
    let start = heap_start();
    let end = heap_end();
    let stats = HeapStats {
        start,
        end,
        total_bytes: end - start,
        used_bytes: inner.used_bytes,
        bump_bytes: inner.bump_offset,
        active_allocs: inner.alloc_count,
    };
    GLOBAL_ALLOCATOR.unlock();
    stats
}

/// Print the complete physical memory map of the QEMU `virt` machine and BIOS sections.
pub fn print_memory_map(ram_base: u64, ram_size: u64) {
    let ram_end = ram_base.saturating_add(ram_size);
    println!("Physical Memory Map (QEMU AArch64 virt):");
    println!("  0x0000_0000_0000 .. 0x0000_0400_0000  [ 64 MB]  NOR Flash Bank 0 (BIOS ROM)");
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} B)  BIOS .text + .rodata + .data LMA",
        rom_start(),
        rom_end() + (edata() - sdata()),
        (rom_end() - rom_start()) + (edata() - sdata())
    );
    println!("  0x0000_0400_0000 .. 0x0000_0800_0000  [ 64 MB]  NOR Flash Bank 1 (NVRAM / Payload)");
    println!("  0x0000_0800_0000 .. 0x0000_0802_0000  [128 KB]  ARM GICv2 Distributor & CPU Interface");
    println!("  0x0000_0900_0000 .. 0x0000_0900_1000  [  4 KB]  PL011 UART0 Serial Controller");
    println!("  0x0000_0901_0000 .. 0x0000_0901_1000  [  4 KB]  PL031 Real-Time Clock (RTC)");
    println!("  0x0000_0902_0000 .. 0x0000_0902_0018  [  24 B]  QEMU fw_cfg Firmware Configuration");
    println!("  0x0000_0903_0000 .. 0x0000_0903_1000  [  4 KB]  PL061 GPIO Controller");
    println!("  0x0000_0A00_0000 .. 0x0000_0A00_4000  [ 16 KB]  VirtIO MMIO Transport (32 slots)");
    println!("  0x0000_1000_0000 .. 0x0000_3EFF_0000  [751 MB]  PCIe 32-bit MMIO Window");
    println!("  0x0000_3EFF_0000 .. 0x0000_3F00_0000  [ 64 KB]  PCIe I/O Port Window");
    println!(
        "  0x{:012X} .. 0x{:012X}  [{:3} MB]  System RAM",
        ram_base,
        ram_end,
        ram_size / (1024 * 1024)
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} KB) Reserved for Flattened Device Tree (DTB)",
        dtb_start(),
        dtb_max_end(),
        (dtb_max_end() - dtb_start()) / 1024
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} B)  BIOS .data section (VMA)",
        sdata(),
        edata(),
        edata() - sdata()
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} B)  BIOS .bss section",
        sbss(),
        ebss(),
        ebss() - sbss()
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} KB) BIOS Stack",
        stack_bottom(),
        stack_top(),
        (stack_top() - stack_bottom()) / 1024
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} KB) Page-Aligned DMA / Virtqueue Arena",
        dma_start(),
        dma_end(),
        (dma_end() - dma_start()) / 1024
    );
    println!(
        "    ├─ 0x{:08X} .. 0x{:08X}  ({:6} KB) BIOS Heap Arena",
        heap_start(),
        heap_end(),
        (heap_end() - heap_start()) / 1024
    );
    println!(
        "    └─ 0x{:08X} .. 0x{:08X}  ({:6} KB) Free Payload / OS RAM",
        payload_start(),
        ram_end,
        (ram_end.saturating_sub(payload_start() as u64)) / 1024
    );
    println!("  0x0040_1000_0000 .. 0x0040_2000_0000  [256 MB]  PCIe High ECAM Configuration Space");
}

pub struct MemtestReport {
    pub base: usize,
    pub size: usize,
    pub patterns_tested: usize,
    pub failed_addr: Option<usize>,
}

/// Run a 4-pattern memory integrity test (all-0s, all-1s, checkerboard 0x55/0xAA, and address XOR)
/// over `[base .. base + size]`, restoring original contents or zeroing afterwards.
pub fn run_memtest(base: usize, size: usize) -> MemtestReport {
    let words = size / 8;
    let ptr = base as *mut u64;

    let patterns: [u64; 3] = [
        0x0000_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF,
        0x5555_5555_5555_5555,
    ];

    for &pat in &patterns {
        for i in 0..words {
            unsafe {
                let p = if (i & 1) == 0 { pat } else { !pat };
                write_volatile(ptr.add(i), p);
            }
        }
        for i in 0..words {
            let expected = if (i & 1) == 0 { pat } else { !pat };
            let actual = unsafe { read_volatile(ptr.add(i)) };
            if actual != expected {
                return MemtestReport {
                    base,
                    size,
                    patterns_tested: 3,
                    failed_addr: Some(base + i * 8),
                };
            }
        }
    }

    // Pattern 4: Address uniqueness test (detects address line aliasing)
    for i in 0..words {
        let addr = (base + i * 8) as u64;
        let val = addr ^ 0xA5A5_5A5A_F0F0_0F0F;
        unsafe {
            write_volatile(ptr.add(i), val);
        }
    }
    for i in 0..words {
        let addr = (base + i * 8) as u64;
        let expected = addr ^ 0xA5A5_5A5A_F0F0_0F0F;
        let actual = unsafe { read_volatile(ptr.add(i)) };
        if actual != expected {
            return MemtestReport {
                base,
                size,
                patterns_tested: 4,
                failed_addr: Some(base + i * 8),
            };
        }
        // Leave tested memory clean (zeroed)
        unsafe {
            write_volatile(ptr.add(i), 0);
        }
    }

    MemtestReport {
        base,
        size,
        patterns_tested: 4,
        failed_addr: None,
    }
}
