//! Lightweight UEFI 2.10 System Table, Boot Services, Runtime Services, and Console Protocols
//! allowing AArch64 UEFI applications (`aarch64-unknown-uefi`, `efi_main`) to run directly on Hinode BIOS.

extern crate alloc;
use core::alloc::Layout;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::{mem, pe::PeLoadedInfo, power, println, rtc, timer, uart};

pub type EfiStatus = usize;
pub type EfiHandle = usize;
pub type EfiPhysicalAddress = u64;
pub type EfiVirtualAddress = u64;
pub type Char16 = u16;

const EFI_ERROR_BIT: usize = 1usize << 63;
pub const EFI_SUCCESS: EfiStatus = 0;
pub const EFI_INVALID_PARAMETER: EfiStatus = EFI_ERROR_BIT | 2;
pub const EFI_UNSUPPORTED: EfiStatus = EFI_ERROR_BIT | 3;
pub const EFI_BUFFER_TOO_SMALL: EfiStatus = EFI_ERROR_BIT | 5;
pub const EFI_NOT_READY: EfiStatus = EFI_ERROR_BIT | 6;
pub const EFI_OUT_OF_RESOURCES: EfiStatus = EFI_ERROR_BIT | 9;
pub const EFI_NOT_FOUND: EfiStatus = EFI_ERROR_BIT | 14;

const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249; // "IBI SYST"
const EFI_BOOT_SERVICES_SIGNATURE: u64 = 0x5652_4553_544f_4f42; // "BOOTSERV"
const EFI_RUNTIME_SERVICES_SIGNATURE: u64 = 0x5652_4553_544e_5552; // "RUNTSERV"
const EFI_SPECIFICATION_VERSION: u32 = (2 << 16) | 100; // UEFI 2.10

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

// Standard UEFI FDT Configuration Table GUID: b1b621d5-f19c-41a5-830b-d9152c69aae0
pub const EFI_DTB_TABLE_GUID: EfiGuid = EfiGuid {
    data1: 0xb1b6_21d5,
    data2: 0xf19c,
    data3: 0x41a5,
    data4: [0x83, 0x0b, 0xd9, 0x15, 0x2c, 0x69, 0xaa, 0xe0],
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiConfigurationTable {
    pub vendor_guid: EfiGuid,
    pub vendor_table: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SimpleTextOutputMode {
    pub max_mode: i32,
    pub mode: i32,
    pub attribute: i32,
    pub cursor_column: i32,
    pub cursor_row: i32,
    pub cursor_visible: bool,
}

#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    pub reset: extern "C" fn(*mut EfiSimpleTextOutputProtocol, bool) -> EfiStatus,
    pub output_string: extern "C" fn(*mut EfiSimpleTextOutputProtocol, *const Char16) -> EfiStatus,
    pub test_string: extern "C" fn(*mut EfiSimpleTextOutputProtocol, *const Char16) -> EfiStatus,
    pub query_mode:
        extern "C" fn(*mut EfiSimpleTextOutputProtocol, usize, *mut usize, *mut usize) -> EfiStatus,
    pub set_mode: extern "C" fn(*mut EfiSimpleTextOutputProtocol, usize) -> EfiStatus,
    pub set_attribute: extern "C" fn(*mut EfiSimpleTextOutputProtocol, usize) -> EfiStatus,
    pub clear_screen: extern "C" fn(*mut EfiSimpleTextOutputProtocol) -> EfiStatus,
    pub set_cursor_position:
        extern "C" fn(*mut EfiSimpleTextOutputProtocol, usize, usize) -> EfiStatus,
    pub enable_cursor: extern "C" fn(*mut EfiSimpleTextOutputProtocol, bool) -> EfiStatus,
    pub mode: *mut SimpleTextOutputMode,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiInputKey {
    pub scan_code: u16,
    pub unicode_char: Char16,
}

#[repr(C)]
pub struct EfiSimpleTextInputProtocol {
    pub reset: extern "C" fn(*mut EfiSimpleTextInputProtocol, bool) -> EfiStatus,
    pub read_key_stroke:
        extern "C" fn(*mut EfiSimpleTextInputProtocol, *mut EfiInputKey) -> EfiStatus,
    pub wait_for_key: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub pad1: u8,
    pub nanosecond: u32,
    pub time_zone: i16,
    pub daylight: u8,
    pub pad2: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiTimeCapabilities {
    pub resolution: u32,
    pub accuracy: u32,
    pub sets_to_zero: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiMemoryDescriptor {
    pub ty: u32,
    pub pad: u32,
    pub physical_start: EfiPhysicalAddress,
    pub virtual_start: EfiVirtualAddress,
    pub number_of_pages: u64,
    pub attribute: u64,
}

#[repr(C)]
pub struct EfiRuntimeServices {
    pub hdr: EfiTableHeader,
    pub get_time: extern "C" fn(*mut EfiTime, *mut EfiTimeCapabilities) -> EfiStatus,
    pub set_time: extern "C" fn(*const EfiTime) -> EfiStatus,
    pub get_wakeup_time: extern "C" fn(*mut bool, *mut bool, *mut EfiTime) -> EfiStatus,
    pub set_wakeup_time: extern "C" fn(bool, *const EfiTime) -> EfiStatus,
    pub set_virtual_address_map:
        extern "C" fn(usize, usize, u32, *const EfiMemoryDescriptor) -> EfiStatus,
    pub convert_pointer: extern "C" fn(usize, *mut usize) -> EfiStatus,
    pub get_variable:
        extern "C" fn(*const Char16, *const EfiGuid, *mut u32, *mut usize, *mut u8) -> EfiStatus,
    pub get_next_variable_name:
        extern "C" fn(*mut usize, *mut Char16, *mut EfiGuid) -> EfiStatus,
    pub set_variable:
        extern "C" fn(*const Char16, *const EfiGuid, u32, usize, *const u8) -> EfiStatus,
    pub get_next_high_monotonic_count: extern "C" fn(*mut u32) -> EfiStatus,
    pub reset_system: extern "C" fn(u32, EfiStatus, usize, *const u8) -> !,
}

#[repr(C)]
pub struct EfiBootServices {
    pub hdr: EfiTableHeader,
    pub raise_tpl: extern "C" fn(usize) -> usize,
    pub restore_tpl: extern "C" fn(usize),
    pub allocate_pages:
        extern "C" fn(u32, u32, usize, *mut EfiPhysicalAddress) -> EfiStatus,
    pub free_pages: extern "C" fn(EfiPhysicalAddress, usize) -> EfiStatus,
    pub get_memory_map: extern "C" fn(
        *mut usize,
        *mut EfiMemoryDescriptor,
        *mut usize,
        *mut usize,
        *mut u32,
    ) -> EfiStatus,
    pub allocate_pool: extern "C" fn(u32, usize, *mut *mut u8) -> EfiStatus,
    pub free_pool: extern "C" fn(*mut u8) -> EfiStatus,
    pub create_event: usize,
    pub set_timer: usize,
    pub wait_for_event: extern "C" fn(usize, *const usize, *mut usize) -> EfiStatus,
    pub signal_event: usize,
    pub close_event: usize,
    pub check_event: usize,
    pub install_protocol_interface: usize,
    pub reinstall_protocol_interface: usize,
    pub uninstall_protocol_interface: usize,
    pub handle_protocol:
        extern "C" fn(EfiHandle, *const EfiGuid, *mut *mut u8) -> EfiStatus,
    pub reserved: usize,
    pub register_protocol_notify: usize,
    pub locate_handle: usize,
    pub locate_device_path: usize,
    pub install_configuration_table: extern "C" fn(*const EfiGuid, usize) -> EfiStatus,
    pub load_image: usize,
    pub start_image: usize,
    pub exit: extern "C" fn(EfiHandle, EfiStatus, usize, *const Char16) -> EfiStatus,
    pub unload_image: usize,
    pub exit_boot_services: extern "C" fn(EfiHandle, usize) -> EfiStatus,
    pub get_next_monotonic_count: extern "C" fn(*mut u64) -> EfiStatus,
    pub stall: extern "C" fn(usize) -> EfiStatus,
    pub set_watchdog_timer:
        extern "C" fn(usize, u64, usize, *const Char16) -> EfiStatus,
    pub connect_controller: usize,
    pub disconnect_controller: usize,
    pub open_protocol: usize,
    pub close_protocol: usize,
    pub open_protocol_information: usize,
    pub protocols_per_handle: usize,
    pub locate_handle_buffer: usize,
    pub locate_protocol:
        extern "C" fn(*const EfiGuid, usize, *mut *mut u8) -> EfiStatus,
    pub install_multiple_protocol_interfaces: usize,
    pub uninstall_multiple_protocol_interfaces: usize,
    pub calculate_crc32: extern "C" fn(*const u8, usize, *mut u32) -> EfiStatus,
    pub copy_mem: extern "C" fn(*mut u8, *const u8, usize),
    pub set_mem: extern "C" fn(*mut u8, usize, u8),
    pub create_event_ex: usize,
}

#[repr(C)]
pub struct EfiSystemTable {
    pub hdr: EfiTableHeader,
    pub firmware_vendor: *const Char16,
    pub firmware_revision: u32,
    pub console_in_handle: EfiHandle,
    pub con_in: *mut EfiSimpleTextInputProtocol,
    pub console_out_handle: EfiHandle,
    pub con_out: *mut EfiSimpleTextOutputProtocol,
    pub standard_error_handle: EfiHandle,
    pub std_err: *mut EfiSimpleTextOutputProtocol,
    pub runtime_services: *mut EfiRuntimeServices,
    pub boot_services: *mut EfiBootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *mut EfiConfigurationTable,
}

static NEXT_PAGE_ADDR: AtomicUsize = AtomicUsize::new(0x4100_0000);
static MONOTONIC_COUNTER: AtomicUsize = AtomicUsize::new(1);

// ============================================================================
// SimpleTextOutput & SimpleTextInput Protocol Implementations
// ============================================================================

extern "C" fn con_out_reset(_this: *mut EfiSimpleTextOutputProtocol, _ext: bool) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn con_out_output_string(
    _this: *mut EfiSimpleTextOutputProtocol,
    string: *const Char16,
) -> EfiStatus {
    if string.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    let mut idx = 0usize;
    unsafe {
        loop {
            let u = *string.add(idx);
            if u == 0 {
                break;
            }
            idx += 1;
            if u < 0x80 {
                let b = u as u8;
                if b == b'\r' {
                    uart::putc_raw(b'\r');
                } else if b == b'\n' {
                    uart::putc_raw(b'\r');
                    uart::putc_raw(b'\n');
                } else {
                    uart::putc_raw(b);
                }
            } else if let Some(ch) = char::from_u32(u as u32) {
                let mut utf8 = [0u8; 4];
                let s = ch.encode_utf8(&mut utf8);
                uart::puts(s);
            }
        }
    }
    EFI_SUCCESS
}

extern "C" fn con_out_test_string(
    _this: *mut EfiSimpleTextOutputProtocol,
    _string: *const Char16,
) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn con_out_query_mode(
    _this: *mut EfiSimpleTextOutputProtocol,
    mode_number: usize,
    columns: *mut usize,
    rows: *mut usize,
) -> EfiStatus {
    if mode_number != 0 {
        return EFI_UNSUPPORTED;
    }
    unsafe {
        if !columns.is_null() {
            *columns = 80;
        }
        if !rows.is_null() {
            *rows = 25;
        }
    }
    EFI_SUCCESS
}

extern "C" fn con_out_set_mode(_this: *mut EfiSimpleTextOutputProtocol, mode: usize) -> EfiStatus {
    if mode == 0 {
        EFI_SUCCESS
    } else {
        EFI_UNSUPPORTED
    }
}

extern "C" fn con_out_set_attribute(
    this: *mut EfiSimpleTextOutputProtocol,
    attr: usize,
) -> EfiStatus {
    unsafe {
        if !this.is_null() && !(*this).mode.is_null() {
            (*(*this).mode).attribute = attr as i32;
        }
    }
    EFI_SUCCESS
}

extern "C" fn con_out_clear_screen(_this: *mut EfiSimpleTextOutputProtocol) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn con_out_set_cursor_pos(
    _this: *mut EfiSimpleTextOutputProtocol,
    _col: usize,
    _row: usize,
) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn con_out_enable_cursor(
    this: *mut EfiSimpleTextOutputProtocol,
    visible: bool,
) -> EfiStatus {
    unsafe {
        if !this.is_null() && !(*this).mode.is_null() {
            (*(*this).mode).cursor_visible = visible;
        }
    }
    EFI_SUCCESS
}

extern "C" fn con_in_reset(_this: *mut EfiSimpleTextInputProtocol, _ext: bool) -> EfiStatus {
    while uart::try_getc().is_some() {}
    EFI_SUCCESS
}

extern "C" fn con_in_read_key(
    _this: *mut EfiSimpleTextInputProtocol,
    key: *mut EfiInputKey,
) -> EfiStatus {
    if key.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    if let Some(b) = uart::try_getc() {
        unsafe {
            (*key).scan_code = 0;
            (*key).unicode_char = b as u16;
        }
        EFI_SUCCESS
    } else {
        EFI_NOT_READY
    }
}

// ============================================================================
// Runtime Services & Boot Services Implementations
// ============================================================================

extern "C" fn rt_get_time(
    time: *mut EfiTime,
    caps: *mut EfiTimeCapabilities,
) -> EfiStatus {
    if time.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    let dt = rtc::now_utc();
    unsafe {
        *time = EfiTime {
            year: dt.year,
            month: dt.month,
            day: dt.day,
            hour: dt.hour,
            minute: dt.minute,
            second: dt.second,
            pad1: 0,
            nanosecond: 0,
            time_zone: 0,
            daylight: 0,
            pad2: 0,
        };
        if !caps.is_null() {
            *caps = EfiTimeCapabilities {
                resolution: 1,
                accuracy: 50_000_000,
                sets_to_zero: false,
            };
        }
    }
    EFI_SUCCESS
}

extern "C" fn rt_set_time(_time: *const EfiTime) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn rt_get_wakeup_time(
    _enabled: *mut bool,
    _pending: *mut bool,
    _time: *mut EfiTime,
) -> EfiStatus {
    EFI_UNSUPPORTED
}

extern "C" fn rt_set_wakeup_time(_enable: bool, _time: *const EfiTime) -> EfiStatus {
    EFI_UNSUPPORTED
}

extern "C" fn rt_set_virtual_address_map(
    _map_size: usize,
    _desc_size: usize,
    _desc_ver: u32,
    _virt_map: *const EfiMemoryDescriptor,
) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn rt_convert_pointer(_debug_disp: usize, _addr: *mut usize) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn rt_get_variable(
    _name: *const Char16,
    _vendor: *const EfiGuid,
    _attr: *mut u32,
    _data_size: *mut usize,
    _data: *mut u8,
) -> EfiStatus {
    EFI_NOT_FOUND
}

extern "C" fn rt_get_next_variable_name(
    _size: *mut usize,
    _name: *mut Char16,
    _vendor: *mut EfiGuid,
) -> EfiStatus {
    EFI_NOT_FOUND
}

extern "C" fn rt_set_variable(
    _name: *const Char16,
    _vendor: *const EfiGuid,
    _attr: u32,
    _size: usize,
    _data: *const u8,
) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn rt_get_next_high_monotonic(high_count: *mut u32) -> EfiStatus {
    if !high_count.is_null() {
        unsafe { *high_count = 1 };
    }
    EFI_SUCCESS
}

extern "C" fn rt_reset_system(
    reset_type: u32,
    _reset_status: EfiStatus,
    _data_size: usize,
    _reset_data: *const u8,
) -> ! {
    // EfiResetShutdown = 2
    if reset_type == 2 {
        power::system_off()
    } else {
        power::system_reset()
    }
}

extern "C" fn bs_raise_tpl(new_tpl: usize) -> usize {
    new_tpl
}

extern "C" fn bs_restore_tpl(_old_tpl: usize) {}

extern "C" fn bs_allocate_pages(
    alloc_type: u32,
    _mem_type: u32,
    pages: usize,
    memory: *mut EfiPhysicalAddress,
) -> EfiStatus {
    if memory.is_null() || pages == 0 {
        return EFI_INVALID_PARAMETER;
    }
    let bytes = pages * 4096;
    // AllocateAddress == 2
    if alloc_type == 2 {
        return EFI_SUCCESS;
    }
    let addr = NEXT_PAGE_ADDR.fetch_add(bytes, Ordering::SeqCst);
    unsafe {
        *memory = addr as u64;
    }
    EFI_SUCCESS
}

extern "C" fn bs_free_pages(_memory: EfiPhysicalAddress, _pages: usize) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn bs_get_memory_map(
    memory_map_size: *mut usize,
    memory_map: *mut EfiMemoryDescriptor,
    map_key: *mut usize,
    descriptor_size: *mut usize,
    descriptor_version: *mut u32,
) -> EfiStatus {
    if memory_map_size.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    let desc_sz = core::mem::size_of::<EfiMemoryDescriptor>();
    let required = desc_sz * 3;
    unsafe {
        if !descriptor_size.is_null() {
            *descriptor_size = desc_sz;
        }
        if !descriptor_version.is_null() {
            *descriptor_version = 1;
        }
        if !map_key.is_null() {
            *map_key = 0x1001;
        }
        if *memory_map_size < required || memory_map.is_null() {
            *memory_map_size = required;
            return EFI_BUFFER_TOO_SMALL;
        }
        *memory_map_size = required;

        // Region 0: Reserved DTB (0x4000_0000 .. 0x4020_0000)
        *memory_map.add(0) = EfiMemoryDescriptor {
            ty: 0, // EfiReservedMemoryType
            pad: 0,
            physical_start: 0x4000_0000,
            virtual_start: 0x4000_0000,
            number_of_pages: 512,
            attribute: 0x8, // EFI_MEMORY_WB
        };
        // Region 1: BIOS Firmware Data/Code (0x4020_0000 .. 0x4040_0000)
        *memory_map.add(1) = EfiMemoryDescriptor {
            ty: 5, // EfiRuntimeServicesCode
            pad: 0,
            physical_start: 0x4020_0000,
            virtual_start: 0x4020_0000,
            number_of_pages: 512,
            attribute: 0x8000_0000_0000_0008,
        };
        // Region 2: Conventional Free System RAM (0x4040_0000 .. 0x4800_0000)
        *memory_map.add(2) = EfiMemoryDescriptor {
            ty: 7, // EfiConventionalMemory
            pad: 0,
            physical_start: 0x4040_0000,
            virtual_start: 0x4040_0000,
            number_of_pages: (124 * 1024 * 1024) / 4096,
            attribute: 0x8,
        };
    }
    EFI_SUCCESS
}

extern "C" fn bs_allocate_pool(
    _pool_type: u32,
    size: usize,
    buffer: *mut *mut u8,
) -> EfiStatus {
    if buffer.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    let layout = match Layout::from_size_align(size.max(16) + 16, 16) {
        Ok(l) => l,
        Err(_) => return EFI_INVALID_PARAMETER,
    };
    let raw = unsafe { alloc::alloc::alloc(layout) };
    if raw.is_null() {
        return EFI_OUT_OF_RESOURCES;
    }
    unsafe {
        *(raw as *mut usize) = size.max(16) + 16;
        *buffer = raw.add(16);
    }
    EFI_SUCCESS
}

extern "C" fn bs_free_pool(buffer: *mut u8) -> EfiStatus {
    if buffer.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    unsafe {
        let raw = buffer.sub(16);
        let total = *(raw as *const usize);
        if let Ok(layout) = Layout::from_size_align(total, 16) {
            alloc::alloc::dealloc(raw, layout);
        }
    }
    EFI_SUCCESS
}

extern "C" fn bs_wait_for_event(
    _num_events: usize,
    _events: *const usize,
    index: *mut usize,
) -> EfiStatus {
    while !uart::has_rx_data() {
        core::hint::spin_loop();
    }
    if !index.is_null() {
        unsafe { *index = 0 };
    }
    EFI_SUCCESS
}

extern "C" fn bs_handle_protocol(
    _handle: EfiHandle,
    _protocol: *const EfiGuid,
    interface: *mut *mut u8,
) -> EfiStatus {
    if !interface.is_null() {
        unsafe { *interface = core::ptr::null_mut() };
    }
    EFI_UNSUPPORTED
}

extern "C" fn bs_install_config_table(_guid: *const EfiGuid, _table: usize) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn bs_exit(
    _image_handle: EfiHandle,
    exit_status: EfiStatus,
    _exit_data_size: usize,
    _exit_data: *const Char16,
) -> EfiStatus {
    exit_status
}

extern "C" fn bs_exit_boot_services(_image_handle: EfiHandle, _map_key: usize) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn bs_get_next_monotonic_count(count: *mut u64) -> EfiStatus {
    if !count.is_null() {
        let c = MONOTONIC_COUNTER.fetch_add(1, Ordering::SeqCst) as u64;
        unsafe { *count = c };
    }
    EFI_SUCCESS
}

extern "C" fn bs_stall(microseconds: usize) -> EfiStatus {
    timer::delay_us(microseconds as u64);
    EFI_SUCCESS
}

extern "C" fn bs_set_watchdog_timer(
    _timeout: usize,
    _watchdog_code: u64,
    _data_size: usize,
    _watchdog_data: *const Char16,
) -> EfiStatus {
    EFI_SUCCESS
}

extern "C" fn bs_locate_protocol(
    _protocol: *const EfiGuid,
    _registration: usize,
    interface: *mut *mut u8,
) -> EfiStatus {
    if !interface.is_null() {
        unsafe { *interface = core::ptr::null_mut() };
    }
    EFI_NOT_FOUND
}

extern "C" fn bs_calculate_crc32(data: *const u8, data_size: usize, crc32: *mut u32) -> EfiStatus {
    if data.is_null() || crc32.is_null() {
        return EFI_INVALID_PARAMETER;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, data_size) };
    let mut crc: u32 = 0xffff_ffff;
    for &b in slice {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    unsafe { *crc32 = !crc };
    EFI_SUCCESS
}

extern "C" fn bs_copy_mem(dest: *mut u8, src: *const u8, len: usize) {
    if !dest.is_null() && !src.is_null() && len > 0 {
        unsafe { core::ptr::copy(src, dest, len) };
    }
}

extern "C" fn bs_set_mem(buffer: *mut u8, size: usize, value: u8) {
    if !buffer.is_null() && size > 0 {
        unsafe { core::ptr::write_bytes(buffer, value, size) };
    }
}

static FIRMWARE_VENDOR_UTF16: [Char16; 12] = [
    b'H' as u16,
    b'i' as u16,
    b'n' as u16,
    b'o' as u16,
    b'd' as u16,
    b'e' as u16,
    b' ' as u16,
    b'B' as u16,
    b'I' as u16,
    b'O' as u16,
    b'S' as u16,
    0,
];

/// Execute a loaded PE32+ AArch64 UEFI image (`efi_main(image_handle, &system_table)`).
pub fn execute_pe32_efi(pe_info: &PeLoadedInfo) -> EfiStatus {
    let mut text_mode = SimpleTextOutputMode {
        max_mode: 1,
        mode: 0,
        attribute: 0x07,
        cursor_column: 0,
        cursor_row: 0,
        cursor_visible: true,
    };

    let mut con_out = EfiSimpleTextOutputProtocol {
        reset: con_out_reset,
        output_string: con_out_output_string,
        test_string: con_out_test_string,
        query_mode: con_out_query_mode,
        set_mode: con_out_set_mode,
        set_attribute: con_out_set_attribute,
        clear_screen: con_out_clear_screen,
        set_cursor_position: con_out_set_cursor_pos,
        enable_cursor: con_out_enable_cursor,
        mode: &raw mut text_mode,
    };

    let mut con_in = EfiSimpleTextInputProtocol {
        reset: con_in_reset,
        read_key_stroke: con_in_read_key,
        wait_for_key: 1,
    };

    let mut runtime_services = EfiRuntimeServices {
        hdr: EfiTableHeader {
            signature: EFI_RUNTIME_SERVICES_SIGNATURE,
            revision: EFI_SPECIFICATION_VERSION,
            header_size: core::mem::size_of::<EfiRuntimeServices>() as u32,
            crc32: 0,
            reserved: 0,
        },
        get_time: rt_get_time,
        set_time: rt_set_time,
        get_wakeup_time: rt_get_wakeup_time,
        set_wakeup_time: rt_set_wakeup_time,
        set_virtual_address_map: rt_set_virtual_address_map,
        convert_pointer: rt_convert_pointer,
        get_variable: rt_get_variable,
        get_next_variable_name: rt_get_next_variable_name,
        set_variable: rt_set_variable,
        get_next_high_monotonic_count: rt_get_next_high_monotonic,
        reset_system: rt_reset_system,
    };

    let mut boot_services = EfiBootServices {
        hdr: EfiTableHeader {
            signature: EFI_BOOT_SERVICES_SIGNATURE,
            revision: EFI_SPECIFICATION_VERSION,
            header_size: core::mem::size_of::<EfiBootServices>() as u32,
            crc32: 0,
            reserved: 0,
        },
        raise_tpl: bs_raise_tpl,
        restore_tpl: bs_restore_tpl,
        allocate_pages: bs_allocate_pages,
        free_pages: bs_free_pages,
        get_memory_map: bs_get_memory_map,
        allocate_pool: bs_allocate_pool,
        free_pool: bs_free_pool,
        create_event: 0,
        set_timer: 0,
        wait_for_event: bs_wait_for_event,
        signal_event: 0,
        close_event: 0,
        check_event: 0,
        install_protocol_interface: 0,
        reinstall_protocol_interface: 0,
        uninstall_protocol_interface: 0,
        handle_protocol: bs_handle_protocol,
        reserved: 0,
        register_protocol_notify: 0,
        locate_handle: 0,
        locate_device_path: 0,
        install_configuration_table: bs_install_config_table,
        load_image: 0,
        start_image: 0,
        exit: bs_exit,
        unload_image: 0,
        exit_boot_services: bs_exit_boot_services,
        get_next_monotonic_count: bs_get_next_monotonic_count,
        stall: bs_stall,
        set_watchdog_timer: bs_set_watchdog_timer,
        connect_controller: 0,
        disconnect_controller: 0,
        open_protocol: 0,
        close_protocol: 0,
        open_protocol_information: 0,
        protocols_per_handle: 0,
        locate_handle_buffer: 0,
        locate_protocol: bs_locate_protocol,
        install_multiple_protocol_interfaces: 0,
        uninstall_multiple_protocol_interfaces: 0,
        calculate_crc32: bs_calculate_crc32,
        copy_mem: bs_copy_mem,
        set_mem: bs_set_mem,
        create_event_ex: 0,
    };

    let mut config_tables = [EfiConfigurationTable {
        vendor_guid: EFI_DTB_TABLE_GUID,
        vendor_table: mem::dtb_start(),
    }];

    let mut system_table = EfiSystemTable {
        hdr: EfiTableHeader {
            signature: EFI_SYSTEM_TABLE_SIGNATURE,
            revision: EFI_SPECIFICATION_VERSION,
            header_size: core::mem::size_of::<EfiSystemTable>() as u32,
            crc32: 0,
            reserved: 0,
        },
        firmware_vendor: FIRMWARE_VENDOR_UTF16.as_ptr(),
        firmware_revision: 0x0001_0000,
        console_in_handle: 0x10,
        con_in: &raw mut con_in,
        console_out_handle: 0x20,
        con_out: &raw mut con_out,
        standard_error_handle: 0x30,
        std_err: &raw mut con_out,
        runtime_services: &raw mut runtime_services,
        boot_services: &raw mut boot_services,
        number_of_table_entries: config_tables.len(),
        configuration_table: config_tables.as_mut_ptr(),
    };

    let image_handle: EfiHandle = pe_info.image_base;
    println!(
        "  [UEFI]  Jumping to efi_main @ 0x{:016X} (ImageBase=0x{:08X}, SystemTable=0x{:08X})...",
        pe_info.entry_point,
        pe_info.image_base,
        (&raw const system_table) as usize
    );

    let efi_main: extern "C" fn(EfiHandle, *mut EfiSystemTable) -> EfiStatus =
        unsafe { core::mem::transmute(pe_info.entry_point) };
    let status = efi_main(image_handle, &raw mut system_table);
    println!("  [UEFI]  efi_main returned status: 0x{:X}", status);
    status
}
