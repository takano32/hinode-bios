#!/usr/bin/env bash
set -euo pipefail

BIOS_BIN="target/aarch64-unknown-none/release/hinode_bios.bin"

if [ ! -f "$BIOS_BIN" ]; then
    echo "Building $BIOS_BIN..."
    make build
fi

TMP_DIR="$(mktemp -d /tmp/hinode_bios_test.XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

pass() {
    echo "  [PASS] $1"
}

fail() {
    echo "  [FAIL] $1"
    exit 1
}

echo "======================================================================"
echo "             Hinode BIOS Automated QEMU Integration Tests             "
echo "======================================================================"

# ----------------------------------------------------------------------------
# Test 1: EL1 Boot, 10/10 POST, RAM Test, Trap, IRQ, ELF64 & PE32+ Demos
# ----------------------------------------------------------------------------
OUT1="$TMP_DIR/test1.log"
qemu-system-aarch64 -M virt -cpu cortex-a57 -display none -serial stdio \
    -bios "$BIOS_BIN" \
    -fw_cfg name=opt/hinode/cmd,string="info; dtb; memmap; memtest 64; devices; regs; trap; irq; demo; shutdown" \
    > "$OUT1" 2>&1

grep -q "POST Complete: 10/10 checks passed" "$OUT1" || fail "Test 1: POST 10/10 failed"
grep -q "\[PASS\] Verified 64 KB (4 patterns) with 0 errors" "$OUT1" || fail "Test 1: RAM memtest failed"
grep -q "Trap handler returned: SUCCESS" "$OUT1" || fail "Test 1: BRK trap handler failed"
grep -q "IRQ handler returned: SUCCESS" "$OUT1" || fail "Test 1: GICv2 SGI IRQ handler failed"
grep -q "\[Guest ELF64\] Hello from AArch64 ELF64 Payload!" "$OUT1" || fail "Test 1: Built-in ELF64 demo failed"
grep -q "Payload returned to Hinode BIOS with status: 0xD00DFEED" "$OUT1" || fail "Test 1: ELF64 DTB verification failed"
grep -q "\[Guest UEFI\]  Hello from AArch64 PE32+ efi_main" "$OUT1" || fail "Test 1: Built-in PE32+ UEFI demo failed"
pass "Test 1: EL1 Boot, 10/10 POST, RAM Memtest, BRK Trap, GICv2 IRQ, ELF64 & UEFI Demos"

# ----------------------------------------------------------------------------
# Test 2: EL2 Hypervisor Mode (virtualization=on) & PL061 GPIO (acpi=off)
# ----------------------------------------------------------------------------
OUT2="$TMP_DIR/test2.log"
qemu-system-aarch64 -M virt,virtualization=on,acpi=off -cpu cortex-a57 -display none -serial stdio \
    -bios "$BIOS_BIN" \
    -fw_cfg name=opt/hinode/cmd,string="info; trap; irq; shutdown" \
    > "$OUT2" 2>&1

grep -q "Booting on CPU 0 (Exception Level EL2)" "$OUT2" || fail "Test 2: EL2 detection failed"
grep -q "POST Complete: 10/10 checks passed" "$OUT2" || fail "Test 2: EL2 POST 10/10 failed"
grep -q "PL061 (ID=0x00041061)" "$OUT2" || fail "Test 2: PL061 GPIO detection failed"
grep -q "PSCI v1.1 (smc)" "$OUT2" || fail "Test 2: PSCI SMC conduit detection failed"
pass "Test 2: EL2 Hypervisor Mode (virtualization=on), PL061 GPIO, and SMC PSCI Conduit"

# ----------------------------------------------------------------------------
# Test 3: Multi-Core SMP CPU Parking (-smp 4)
# ----------------------------------------------------------------------------
OUT3="$TMP_DIR/test3.log"
qemu-system-aarch64 -M virt -cpu cortex-a57 -smp 4 -display none -serial stdio \
    -bios "$BIOS_BIN" \
    -fw_cfg name=opt/hinode/cmd,string="info; shutdown" \
    > "$OUT3" 2>&1

grep -q "POST Complete: 10/10 checks passed" "$OUT3" || fail "Test 3: SMP 4-core POST failed"
grep -q "4x arm,cortex-a57" "$OUT3" || grep -q "4 core(s)" "$OUT3" || fail "Test 3: SMP 4-core DTB CPU count failed"
pass "Test 3: Multi-Core SMP (-smp 4) Secondary CPU Parking & Topology Discovery"

# ----------------------------------------------------------------------------
# Test 4: VirtIO-PCI & VirtIO-MMIO FAT16 ESP Boot with Real Rust UEFI App
# ----------------------------------------------------------------------------
UEFI_SRC="$TMP_DIR/bootaa64.rs"
UEFI_BIN="$TMP_DIR/BOOTAA64.EFI"
FAT_IMG="$TMP_DIR/esp.img"

cat << 'EOF' > "$UEFI_SRC"
#![no_std]
#![no_main]
use core::ffi::c_void;

#[repr(C)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    pub reset: extern "efiapi" fn(*mut EfiSimpleTextOutputProtocol, bool) -> usize,
    pub output_string: extern "efiapi" fn(*mut EfiSimpleTextOutputProtocol, *const u16) -> usize,
}

#[repr(C)]
pub struct EfiSystemTable {
    pub hdr: EfiTableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: *mut c_void,
    pub con_in: *mut c_void,
    pub console_out_handle: *mut c_void,
    pub con_out: *mut EfiSimpleTextOutputProtocol,
}

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(_handle: *mut c_void, st: *mut EfiSystemTable) -> usize {
    let msg = b"  >> [ESP BOOTAA64.EFI] Booted real Rust UEFI app from FAT16 VirtIO disk!\r\n\0";
    let mut utf16 = [0u16; 80];
    for (i, &b) in msg.iter().enumerate() {
        utf16[i] = b as u16;
    }
    unsafe {
        let con_out = (*st).con_out;
        ((*con_out).output_string)(con_out, utf16.as_ptr());
    }
    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
EOF

rustc --edition 2024 --target aarch64-unknown-uefi -C opt-level=s -C panic=abort "$UEFI_SRC" -o "$UEFI_BIN"
truncate -s 16M "$FAT_IMG"
mkfs.vfat -F 16 -n "HINODE_ESP" "$FAT_IMG" > /dev/null
mmd -i "$FAT_IMG" ::/EFI ::/EFI/BOOT
mcopy -i "$FAT_IMG" "$UEFI_BIN" ::/EFI/BOOT/BOOTAA64.EFI
echo "Hinode BIOS Disk Test OK" > "$TMP_DIR/hello.txt"
mcopy -i "$FAT_IMG" "$TMP_DIR/hello.txt" ::/HELLO.TXT

OUT4="$TMP_DIR/test4.log"
qemu-system-aarch64 -M virt -cpu cortex-a57 -display none -serial stdio \
    -bios "$BIOS_BIN" \
    -drive file="$FAT_IMG",if=virtio,format=raw \
    -fw_cfg name=opt/hinode/cmd,string="ls /; cat /HELLO.TXT; boot; shutdown" \
    > "$OUT4" 2>&1

grep -q "Hinode BIOS Disk Test OK" "$OUT4" || fail "Test 4: FAT16 cat /HELLO.TXT failed"
grep -q "\[ESP BOOTAA64.EFI\] Booted real Rust UEFI app from FAT16 VirtIO disk!" "$OUT4" || fail "Test 4: VirtIO-PCI FAT16 UEFI boot failed"
pass "Test 4: VirtIO-PCI Block Driver, FAT16 Filesystem, and Real Rust BOOTAA64.EFI Execution"

# ----------------------------------------------------------------------------
# Test 5: Interactive UART Shell Input via Standard Input Pipe
# ----------------------------------------------------------------------------
OUT5="$TMP_DIR/test5.log"
printf "help\nmw 0x40600000 0xCAFEBABE12345678\nmd 0x40600000 16\nshutdown\n" | \
    qemu-system-aarch64 -M virt -cpu cortex-a57 -display none -serial stdio \
    -bios "$BIOS_BIN" > "$OUT5" 2>&1

grep -q "Hinode BIOS Monitor Commands:" "$OUT5" || fail "Test 5: Interactive 'help' command failed"
grep -q "Wrote 0xCAFEBABE12345678 -> \[0x000040600000\]" "$OUT5" || fail "Test 5: Interactive 'mw' command failed"
grep -q "78 56 34 12 BE BA FE CA" "$OUT5" || fail "Test 5: Interactive 'md' verification failed"
pass "Test 5: Interactive UART Monitor Shell (help, mw, md, shutdown)"

# ----------------------------------------------------------------------------
# Test 6: Panic Handler Verification
# ----------------------------------------------------------------------------
OUT6="$TMP_DIR/test6.log"
qemu-system-aarch64 -M virt -cpu cortex-a57 -display none -serial stdio \
    -bios "$BIOS_BIN" \
    -fw_cfg name=opt/hinode/panic_off,string="1" \
    -fw_cfg name=opt/hinode/cmd,string="panic" \
    > "$OUT6" 2>&1

grep -q "!!! PANIC !!!" "$OUT6" || fail "Test 6: Panic banner missing"
grep -q "This is a test panic from Hinode BIOS!" "$OUT6" || fail "Test 6: Panic message missing"
pass "Test 6: Custom Bare-Metal Panic Handler Verification"

echo "======================================================================"
echo "               ALL 6 QEMU INTEGRATION TEST SUITES PASSED!             "
echo "======================================================================"
