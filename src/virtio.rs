//! VirtIO MMIO (`0x0a00_0000..0x0a00_3e00`) and VirtIO PCI Block Device driver.

extern crate alloc;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use crate::{arch, mem, pci, println};

pub const VIRTIO_MMIO_BASE: usize = 0x0a00_0000;
pub const VIRTIO_MMIO_STRIDE: usize = 0x200;
pub const VIRTIO_MMIO_COUNT: usize = 32;
pub const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt"

// VirtIO MMIO register offsets
const MMIO_MAGIC_VALUE: usize = 0x000;
const MMIO_VERSION: usize = 0x004;
const MMIO_DEVICE_ID: usize = 0x008;
const MMIO_VENDOR_ID: usize = 0x00C;
const MMIO_DEVICE_FEATURES: usize = 0x010;
const MMIO_DRIVER_FEATURES: usize = 0x020;
const MMIO_GUEST_PAGE_SIZE: usize = 0x028; // v1
const MMIO_QUEUE_SEL: usize = 0x030;
const MMIO_QUEUE_NUM_MAX: usize = 0x034;
const MMIO_QUEUE_NUM: usize = 0x038;
const MMIO_QUEUE_ALIGN: usize = 0x03C; // v1
const MMIO_QUEUE_PFN: usize = 0x040; // v1
const MMIO_QUEUE_READY: usize = 0x044; // v2
const MMIO_QUEUE_NOTIFY: usize = 0x050;
const MMIO_INTERRUPT_STATUS: usize = 0x060;
const MMIO_INTERRUPT_ACK: usize = 0x064;
const MMIO_STATUS: usize = 0x070;
const MMIO_QUEUE_DESC_LOW: usize = 0x080; // v2
const MMIO_QUEUE_DESC_HIGH: usize = 0x084; // v2
const MMIO_QUEUE_AVAIL_LOW: usize = 0x090; // v2
const MMIO_QUEUE_AVAIL_HIGH: usize = 0x094; // v2
const MMIO_QUEUE_USED_LOW: usize = 0x0A0; // v2
const MMIO_QUEUE_USED_HIGH: usize = 0x0A4; // v2
const MMIO_CONFIG: usize = 0x100;

// VirtIO Legacy PCI PIO register offsets (BAR0)
const PCI_HOST_FEATURES: usize = 0x00;
const PCI_GUEST_FEATURES: usize = 0x04;
const PCI_QUEUE_PFN: usize = 0x08;
const PCI_QUEUE_NUM: usize = 0x0C;
const PCI_QUEUE_SEL: usize = 0x0E;
const PCI_QUEUE_NOTIFY: usize = 0x10;
const PCI_STATUS: usize = 0x12;
const PCI_ISR: usize = 0x13;
const PCI_BLK_CAPACITY: usize = 0x14;

// VirtIO Status bits
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

// Virtqueue Descriptor flags
const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

// VirtIO Block request type
const VIRTIO_BLK_T_IN: u32 = 0;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct VirtIoBlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

#[derive(Clone, Copy)]
pub struct VirtIoMmioSlot {
    pub index: usize,
    pub base: usize,
    pub version: u32,
    pub device_id: u32,
    pub vendor_id: u32,
}

#[derive(Clone, Copy)]
pub enum BlkTransport {
    Mmio { base: usize, version: u32 },
    PciLegacy { pio_base: usize, bdf: (u8, u8, u8) },
}

#[derive(Clone, Copy)]
pub struct VirtIoBlkDevice {
    pub transport: BlkTransport,
    pub capacity_sectors: u64,
}

impl VirtIoBlkDevice {
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_sectors * 512
    }

    pub fn capacity_mb(&self) -> u64 {
        self.capacity_bytes() / (1024 * 1024)
    }
}

pub fn device_type_name(device_id: u32) -> &'static str {
    match device_id {
        0 => "Empty Slot",
        1 => "VirtIO Network Card",
        2 => "VirtIO Block Device",
        3 => "VirtIO Console",
        4 => "VirtIO Entropy RNG",
        5 => "VirtIO Memory Balloon",
        16 => "VirtIO GPU Device",
        18 => "VirtIO Input Device",
        _ => "VirtIO Unknown Device",
    }
}

#[inline]
fn mmio_read32(base: usize, offset: usize) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

#[inline]
fn mmio_write32(base: usize, offset: usize, val: u32) {
    unsafe {
        write_volatile((base + offset) as *mut u32, val);
        arch::dsb_sy();
    }
}

/// Scan all 32 VirtIO MMIO transport slots and return those with an attached device (`device_id != 0`).
pub fn scan_mmio_devices() -> Vec<VirtIoMmioSlot> {
    let mut active = Vec::new();
    for i in 0..VIRTIO_MMIO_COUNT {
        let base = VIRTIO_MMIO_BASE + i * VIRTIO_MMIO_STRIDE;
        let magic = mmio_read32(base, MMIO_MAGIC_VALUE);
        if magic != VIRTIO_MAGIC {
            continue;
        }
        let version = mmio_read32(base, MMIO_VERSION);
        let device_id = mmio_read32(base, MMIO_DEVICE_ID);
        let vendor_id = mmio_read32(base, MMIO_VENDOR_ID);
        if device_id != 0 {
            active.push(VirtIoMmioSlot {
                index: i,
                base,
                version,
                device_id,
                vendor_id,
            });
        }
    }
    active
}

/// Discover all VirtIO Block devices across both VirtIO-MMIO and PCIe ECAM buses.
pub fn find_block_devices(ecam_base: usize) -> Vec<VirtIoBlkDevice> {
    let mut list = Vec::new();

    // 1. Check VirtIO MMIO transports
    for slot in scan_mmio_devices() {
        if slot.device_id == 2 {
            let cap_lo = mmio_read32(slot.base, MMIO_CONFIG) as u64;
            let cap_hi = mmio_read32(slot.base, MMIO_CONFIG + 4) as u64;
            let capacity_sectors = (cap_hi << 32) | cap_lo;
            list.push(VirtIoBlkDevice {
                transport: BlkTransport::Mmio {
                    base: slot.base,
                    version: slot.version,
                },
                capacity_sectors,
            });
        }
    }

    // 2. Check PCIe VirtIO Block devices (1af4:1001)
    for dev in pci::enumerate_and_allocate(ecam_base) {
        if dev.vendor_id == 0x1af4 && (dev.device_id == 0x1001 || dev.device_id == 0x1042) {
            if let Some(bar0) = dev.bars[0] {
                if bar0.is_io {
                    let pio = bar0.cpu_addr;
                    let cap_lo = unsafe { read_volatile((pio + PCI_BLK_CAPACITY) as *const u32) } as u64;
                    let cap_hi =
                        unsafe { read_volatile((pio + PCI_BLK_CAPACITY + 4) as *const u32) } as u64;
                    let capacity_sectors = (cap_hi << 32) | cap_lo;
                    list.push(VirtIoBlkDevice {
                        transport: BlkTransport::PciLegacy {
                            pio_base: pio,
                            bdf: (dev.bus, dev.device, dev.function),
                        },
                        capacity_sectors,
                    });
                }
            }
        }
    }

    list
}

#[inline]
const fn align_up(val: usize, align: usize) -> usize {
    (val + align - 1) & !(align - 1)
}

/// Read `sector_count` 512-byte sectors starting at `lba` from `blk_dev` into `buf`.
pub fn read_sectors(
    blk_dev: &VirtIoBlkDevice,
    lba: u64,
    sector_count: usize,
    buf: &mut [u8],
) -> Result<(), &'static str> {
    if buf.len() < sector_count * 512 {
        return Err("Destination buffer too small for requested sector count");
    }
    if lba + (sector_count as u64) > blk_dev.capacity_sectors {
        return Err("LBA read exceeds block device sector capacity");
    }

    // Read one sector (512B) at a time using our page-aligned DMA ring at `_dma_start + 0x1000`
    let ring_base = align_up(mem::dma_start() + 0x1000, 4096);
    let req_hdr_addr = mem::dma_start() + 0x0100;
    let req_status_addr = mem::dma_start() + 0x0120;
    let sector_bounce_addr = mem::dma_start() + 0x0200;

    match blk_dev.transport {
        BlkTransport::Mmio { base, version } => {
            read_sectors_mmio(
                base,
                version,
                ring_base,
                req_hdr_addr,
                req_status_addr,
                sector_bounce_addr,
                lba,
                sector_count,
                buf,
            )
        }
        BlkTransport::PciLegacy { pio_base, .. } => {
            read_sectors_pci_legacy(
                pio_base,
                ring_base,
                req_hdr_addr,
                req_status_addr,
                sector_bounce_addr,
                lba,
                sector_count,
                buf,
            )
        }
    }
}

fn read_sectors_mmio(
    base: usize,
    version: u32,
    ring_base: usize,
    req_hdr_addr: usize,
    req_status_addr: usize,
    sector_bounce_addr: usize,
    lba: u64,
    sector_count: usize,
    buf: &mut [u8],
) -> Result<(), &'static str> {
    // Reset device
    mmio_write32(base, MMIO_STATUS, 0);
    let mut status = STATUS_ACKNOWLEDGE | STATUS_DRIVER;
    mmio_write32(base, MMIO_STATUS, status);

    let _host_features = mmio_read32(base, MMIO_DEVICE_FEATURES);
    mmio_write32(base, MMIO_DRIVER_FEATURES, 0);

    if version >= 2 {
        status |= STATUS_FEATURES_OK;
        mmio_write32(base, MMIO_STATUS, status);
    }

    // Configure Queue 0
    mmio_write32(base, MMIO_QUEUE_SEL, 0);
    let max_num = mmio_read32(base, MMIO_QUEUE_NUM_MAX);
    if max_num == 0 {
        return Err("VirtIO-MMIO Queue 0 unavailable");
    }
    let qnum: usize = 16.min(max_num as usize);

    // Zero 3 pages (12KB) for descriptor, avail, and used rings
    unsafe {
        core::ptr::write_bytes(ring_base as *mut u8, 0, 0x3000);
    }

    let desc_base = ring_base;
    let avail_base = ring_base + 16 * qnum;
    let used_base = align_up(avail_base + 6 + 2 * qnum, 4096);

    mmio_write32(base, MMIO_QUEUE_NUM, qnum as u32);
    if version == 1 {
        mmio_write32(base, MMIO_GUEST_PAGE_SIZE, 4096);
        mmio_write32(base, MMIO_QUEUE_ALIGN, 4096);
        mmio_write32(base, MMIO_QUEUE_PFN, (ring_base / 4096) as u32);
    } else {
        mmio_write32(base, MMIO_QUEUE_DESC_LOW, desc_base as u32);
        mmio_write32(base, MMIO_QUEUE_DESC_HIGH, (desc_base >> 32) as u32);
        mmio_write32(base, MMIO_QUEUE_AVAIL_LOW, avail_base as u32);
        mmio_write32(base, MMIO_QUEUE_AVAIL_HIGH, (avail_base >> 32) as u32);
        mmio_write32(base, MMIO_QUEUE_USED_LOW, used_base as u32);
        mmio_write32(base, MMIO_QUEUE_USED_HIGH, (used_base >> 32) as u32);
        mmio_write32(base, MMIO_QUEUE_READY, 1);
    }

    status |= STATUS_DRIVER_OK;
    mmio_write32(base, MMIO_STATUS, status);

    for i in 0..sector_count {
        submit_one_sector_req(
            desc_base,
            avail_base,
            used_base,
            qnum,
            req_hdr_addr,
            req_status_addr,
            sector_bounce_addr,
            lba + (i as u64),
            i as u16,
            || {
                mmio_write32(base, MMIO_QUEUE_NOTIFY, 0);
            },
        )?;
        let src = unsafe { core::slice::from_raw_parts(sector_bounce_addr as *const u8, 512) };
        buf[i * 512..(i + 1) * 512].copy_from_slice(src);
        let isr = mmio_read32(base, MMIO_INTERRUPT_STATUS);
        if isr != 0 {
            mmio_write32(base, MMIO_INTERRUPT_ACK, isr);
        }
    }

    Ok(())
}

fn read_sectors_pci_legacy(
    pio_base: usize,
    ring_base: usize,
    req_hdr_addr: usize,
    req_status_addr: usize,
    sector_bounce_addr: usize,
    lba: u64,
    sector_count: usize,
    buf: &mut [u8],
) -> Result<(), &'static str> {
    unsafe {
        // Reset device
        write_volatile((pio_base + PCI_STATUS) as *mut u8, 0);
        arch::dsb_sy();
        write_volatile(
            (pio_base + PCI_STATUS) as *mut u8,
            (STATUS_ACKNOWLEDGE | STATUS_DRIVER) as u8,
        );
        arch::dsb_sy();

        let _features = read_volatile((pio_base + PCI_HOST_FEATURES) as *const u32);
        write_volatile((pio_base + PCI_GUEST_FEATURES) as *mut u32, 0);

        // Select Queue 0 and read fixed queue size
        write_volatile((pio_base + PCI_QUEUE_SEL) as *mut u16, 0);
        arch::dsb_sy();
        let qnum = read_volatile((pio_base + PCI_QUEUE_NUM) as *const u16) as usize;
        if qnum == 0 || qnum > 512 {
            return Err("Invalid VirtIO-PCI Queue 0 size");
        }

        core::ptr::write_bytes(ring_base as *mut u8, 0, 0x4000);
        let desc_base = ring_base;
        let avail_base = ring_base + 16 * qnum;
        let used_base = align_up(avail_base + 6 + 2 * qnum, 4096);

        write_volatile((pio_base + PCI_QUEUE_PFN) as *mut u32, (ring_base >> 12) as u32);
        arch::dsb_sy();

        write_volatile(
            (pio_base + PCI_STATUS) as *mut u8,
            (STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK) as u8,
        );
        arch::dsb_sy();

        for i in 0..sector_count {
            submit_one_sector_req(
                desc_base,
                avail_base,
                used_base,
                qnum,
                req_hdr_addr,
                req_status_addr,
                sector_bounce_addr,
                lba + (i as u64),
                i as u16,
                || {
                    write_volatile((pio_base + PCI_QUEUE_NOTIFY) as *mut u16, 0);
                    arch::dsb_sy();
                },
            )?;
            let src = core::slice::from_raw_parts(sector_bounce_addr as *const u8, 512);
            buf[i * 512..(i + 1) * 512].copy_from_slice(src);
            let _isr = read_volatile((pio_base + PCI_ISR) as *const u8);
        }
    }

    Ok(())
}

fn submit_one_sector_req<F: FnOnce()>(
    desc_base: usize,
    avail_base: usize,
    used_base: usize,
    qnum: usize,
    req_hdr_addr: usize,
    req_status_addr: usize,
    sector_bounce_addr: usize,
    sector: u64,
    seq_idx: u16,
    notify: F,
) -> Result<(), &'static str> {
    unsafe {
        write_volatile(
            req_hdr_addr as *mut VirtIoBlkReqHeader,
            VirtIoBlkReqHeader {
                req_type: VIRTIO_BLK_T_IN,
                reserved: 0,
                sector,
            },
        );
        write_volatile(req_status_addr as *mut u8, 0xff);

        let desc_ptr = desc_base as *mut VirtqDesc;
        // Descriptor 0: 16-byte request header (device-readable)
        write_volatile(
            desc_ptr.add(0),
            VirtqDesc {
                addr: req_hdr_addr as u64,
                len: core::mem::size_of::<VirtIoBlkReqHeader>() as u32,
                flags: VRING_DESC_F_NEXT,
                next: 1,
            },
        );
        // Descriptor 1: 512-byte sector data buffer (device-writable)
        write_volatile(
            desc_ptr.add(1),
            VirtqDesc {
                addr: sector_bounce_addr as u64,
                len: 512,
                flags: VRING_DESC_F_WRITE | VRING_DESC_F_NEXT,
                next: 2,
            },
        );
        // Descriptor 2: 1-byte status byte (device-writable)
        write_volatile(
            desc_ptr.add(2),
            VirtqDesc {
                addr: req_status_addr as u64,
                len: 1,
                flags: VRING_DESC_F_WRITE,
                next: 0,
            },
        );

        let avail_idx_ptr = (avail_base + 2) as *mut u16;
        let avail_ring_ptr = (avail_base + 4) as *mut u16;
        let used_idx_ptr = (used_base + 2) as *const u16;

        let prev_used = read_volatile(used_idx_ptr);
        let avail_slot = (seq_idx as usize) % qnum;
        write_volatile(avail_ring_ptr.add(avail_slot), 0);
        arch::dsb_sy();
        write_volatile(avail_idx_ptr, seq_idx.wrapping_add(1));
        arch::dsb_sy();

        notify();

        let mut completed = false;
        for _ in 0..2_000_000 {
            if read_volatile(used_idx_ptr) != prev_used {
                completed = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !completed {
            return Err("VirtIO block I/O request timed out");
        }

        let status_byte = read_volatile(req_status_addr as *const u8);
        if status_byte != 0 {
            return Err("VirtIO block device returned I/O error status");
        }
    }
    Ok(())
}

pub fn print_devices(ecam_base: usize) {
    let mmio_devs = scan_mmio_devices();
    println!(
        "VirtIO MMIO Bus (0x{:08X} .. 0x{:08X}, 32 slots): {} active device(s)",
        VIRTIO_MMIO_BASE,
        VIRTIO_MMIO_BASE + VIRTIO_MMIO_COUNT * VIRTIO_MMIO_STRIDE,
        mmio_devs.len()
    );
    for d in &mmio_devs {
        println!(
            "  Slot {:02} @ 0x{:08X}: {} (DeviceID={}, Version={}, Vendor=0x{:08X})",
            d.index,
            d.base,
            device_type_name(d.device_id),
            d.device_id,
            d.version,
            d.vendor_id
        );
    }

    let blk_devs = find_block_devices(ecam_base);
    if blk_devs.is_empty() {
        println!("VirtIO Block Storage: No block devices attached");
    } else {
        println!("VirtIO Block Storage: {} drive(s) ready", blk_devs.len());
        for (idx, b) in blk_devs.iter().enumerate() {
            match b.transport {
                BlkTransport::Mmio { base, version } => {
                    println!(
                        "  hd{}: VirtIO-MMIO v{} @ 0x{:08X} - {} sectors ({} KB / {} MB)",
                        idx,
                        version,
                        base,
                        b.capacity_sectors,
                        b.capacity_bytes() / 1024,
                        b.capacity_mb()
                    );
                }
                BlkTransport::PciLegacy { pio_base, bdf } => {
                    println!(
                        "  hd{}: VirtIO-PCI {:02X}:{:02X}.{} (PIO @ 0x{:08X}) - {} sectors ({} KB / {} MB)",
                        idx,
                        bdf.0,
                        bdf.1,
                        bdf.2,
                        pio_base,
                        b.capacity_sectors,
                        b.capacity_bytes() / 1024,
                        b.capacity_mb()
                    );
                }
            }
        }
    }
}
