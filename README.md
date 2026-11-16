# Hinode BIOS (AArch64 Bare-Metal Firmware)

A full-featured, zero-dependency `#![no_std]` bare-metal BIOS and UEFI/ELF boot firmware for AArch64 written in Rust. Configured for the QEMU `virt` machine, **Hinode BIOS** performs multi-core CPU parking, Exception Level (`EL1`/`EL2`/`EL3`) initialization, a 16-entry AArch64 Exception Vector Table, Flattened Device Tree (FDT/DTB) hardware discovery, a 10-stage Power-On Self-Test (POST), PCIe ECAM BAR enumeration, VirtIO Block & FAT12/16/32 disk reading, multi-format OS/payload booting (**PE32+ UEFI `.efi`**, **ELF64**, and **Linux ARM64 `Image`**), and an interactive serial BIOS monitor shell.

---

## File Structure

- [`.cargo/config.toml`](.cargo/config.toml): Configures Cargo to target `aarch64-unknown-none` and passes `linker.ld` to the linker.
- [`linker.ld`](linker.ld): Maps code (`.text`, exception vectors, `.rodata`) to NOR Flash (`0x0000_0000`), preserves the QEMU Flattened Device Tree (DTB) region (`0x4000_0000..0x4020_0000`), and places volatile firmware sections (`.data`, `.bss`, 128 KB stack, 64 KB page-aligned DMA arena, and 1 MB heap) at `0x4020_0000` with free payload RAM starting at `0x4040_0000`.
- [`src/entry.S`](src/entry.S): Low-level AArch64 assembly bootstrap (CPU parking via `mpidr_el1`, `EL3`/`EL2`/`EL1` configuration, FP/SIMD enablement via `CPACR_EL1`, `.data` relocation, `.bss` zeroing) and the 2048-byte aligned 16-entry Exception Vector Table (`_exception_vectors`).
- [`src/main.rs`](src/main.rs): Firmware entry point (`rust_start`), peripheral initialization, POST execution, auto-boot orchestration, and bare-metal panic handler.
- [`src/arch.rs`](src/arch.rs): AArch64 system register accessors, `MIDR_EL1` CPU decoder, and I/D-cache maintenance (`dc cvau` / `ic ivau`).
- [`src/exceptions.rs`](src/exceptions.rs): Synchronous & IRQ exception dispatcher, `ESR_EL1` syndrome decoder, recoverable `BRK` / `SVC` trap handling, and crash register dumps.
- [`src/mem.rs`](src/mem.rs): Global `#[global_allocator]` free-list + bump heap allocator (`alloc`), physical memory map reporter, and 4-pattern RAM `memtest`.
- [`src/uart.rs`](src/uart.rs): ARM PrimeCell PL011 UART driver (`0x0900_0000`) with FIFO initialization, TX/RX polling, interactive line editing, and `print!` / `println!` macros.
- [`src/timer.rs`](src/timer.rs): ARMv8 Generic Timer driver (`CNTFRQ_EL0`, `CNTPCT_EL0`) providing monotonic uptime and microsecond/millisecond delays.
- [`src/rtc.rs`](src/rtc.rs): ARM PrimeCell PL031 Real-Time Clock driver (`0x0901_0000`) with UTC Gregorian calendar conversion.
- [`src/gic.rs`](src/gic.rs): ARM GICv2 Interrupt Controller driver (`0x0800_0000` / `0x0801_0000`) with Software Generated Interrupt (`SGI`) self-testing.
- [`src/gpio.rs`](src/gpio.rs): ARM PrimeCell PL061 GPIO (`0x0903_0000`) and QEMU ACPI Generic Event Device (`0x0908_0000`) driver.
- [`src/power.rs`](src/power.rs): ARM PSCI driver supporting `PSCI_VERSION`, clean VM shutdown (`SYSTEM_OFF`), and reboot (`SYSTEM_RESET`) over `HVC` or `SMC`.
- [`src/dtb.rs`](src/dtb.rs): Zero-dependency Flattened Device Tree (FDT/DTB) binary parser (`0x4000_0000`) for automatic discovery of RAM, CPUs, caches, PSCI method, and MMIO peripherals.
- [`src/fw_cfg.rs`](src/fw_cfg.rs): QEMU Firmware Configuration (`fw_cfg`) MMIO & DMA driver (`0x0902_0000`) for listing/reading host-provided files and boot scripts.
- [`src/pci.rs`](src/pci.rs): Generic PCIe ECAM Bus Scanner (`0x40_1000_0000`) and BIOS BAR resource allocator (I/O, 32-bit MMIO, and 64-bit MMIO windows).
- [`src/virtio.rs`](src/virtio.rs): VirtIO Block device driver supporting both VirtIO-MMIO (`0x0a00_0000..0x0a00_3e00`, v1 & v2) and VirtIO-PCI (`1AF4:1001`).
- [`src/fat.rs`](src/fat.rs): MBR / GPT partition detector and FAT12 / FAT16 / FAT32 filesystem reader with VFAT Long File Name (LFN) support.
- [`src/elf.rs`](src/elf.rs): ELF64 AArch64 executable parser and `PT_LOAD` segment loader.
- [`src/pe.rs`](src/pe.rs): PE32+ AArch64 UEFI executable (`.efi`) parser, section mapper, and `.reloc` (`IMAGE_REL_BASED_DIR64`) base relocator.
- [`src/uefi.rs`](src/uefi.rs): Lightweight UEFI 2.10 `EFI_SYSTEM_TABLE`, `EFI_BOOT_SERVICES`, `EFI_RUNTIME_SERVICES`, `ConOut`, `ConIn`, and FDT Configuration Table environment.
- [`src/boot.rs`](src/boot.rs): Multi-format Boot Manager and built-in ROM ELF64 & PE32+ UEFI demo payloads.
- [`src/post.rs`](src/post.rs): 10-stage Power-On Self-Test (POST) diagnostic suite.
- [`src/shell.rs`](src/shell.rs): Interactive BIOS Monitor Shell (`hinode> `) and command interpreter.
- [`tests/run_qemu_tests.sh`](tests/run_qemu_tests.sh): Automated end-to-end QEMU integration test suite.

---

## Architecture & Memory Map (QEMU `virt`)

| Physical Address Range | Size | Description |
| :--- | :--- | :--- |
| `0x0000_0000_0000 .. 0x0000_0400_0000` | 64 MB | NOR Flash Bank 0 (`hinode_bios.bin` `.text`, `.rodata`, `.data` LMA) |
| `0x0000_0400_0000 .. 0x0000_0800_0000` | 64 MB | NOR Flash Bank 1 (NVRAM / Optional Flashed Payload) |
| `0x0000_0800_0000 .. 0x0000_0802_0000` | 128 KB | ARM GICv2 Distributor (`0x0800_0000`) & CPU Interface (`0x0801_0000`) |
| `0x0000_0900_0000 .. 0x0000_0900_1000` | 4 KB | PL011 UART0 Serial Console |
| `0x0000_0901_0000 .. 0x0000_0901_1000` | 4 KB | PL031 Real-Time Clock (RTC) |
| `0x0000_0902_0000 .. 0x0000_0902_0018` | 24 B | QEMU `fw_cfg` Firmware Configuration (MMIO + DMA) |
| `0x0000_0903_0000 .. 0x0000_0903_1000` | 4 KB | PL061 GPIO Controller (or ACPI GED at `0x0908_0000`) |
| `0x0000_0A00_0000 .. 0x0000_0A00_4000` | 16 KB | VirtIO MMIO Transport Bus (32 slots) |
| `0x0000_1000_0000 .. 0x0000_3EFF_0000` | 751 MB | PCIe 32-bit MMIO Window |
| `0x0000_3EFF_0000 .. 0x0000_3F00_0000` | 64 KB | PCIe I/O Port Window (`0x0000..0xFFFF`) |
| `0x0000_4000_0000 .. 0x0000_4020_0000` | 2 MB | Reserved System RAM for QEMU Flattened Device Tree (DTB) |
| `0x0000_4020_0000 .. 0x0000_4040_0000` | 2 MB | Hinode BIOS RAM (`.data`, `.bss`, 128 KB Stack, 64 KB DMA, 1 MB Heap) |
| `0x0000_4040_0000 .. 0x0000_4800_0000` | ~124 MB | Free System RAM for OS Kernels, ELF64, and UEFI `.efi` Applications |
| `0x0040_1000_0000 .. 0x0040_2000_0000` | 256 MB | PCIe High ECAM Configuration Space |

---

## Building, Running & Testing

### Prerequisites

```bash
# Add bare-metal (and optional UEFI test) targets using rustup
rustup target add aarch64-unknown-none aarch64-unknown-uefi

# Install QEMU for AArch64 and FAT image tools (Debian/Ubuntu)
sudo apt update
sudo apt install qemu-system-arm binutils dosfstools mtools
```

### Build

```bash
make build
```

### Run Interactive BIOS Monitor in QEMU

```bash
make run
```

### Run Automated End-to-End QEMU Integration Tests

```bash
make test
```

---

## Interactive BIOS Monitor Commands (`hinode> `)

Once booted into the `hinode> ` prompt, the following commands are available:

| Command | Description |
| :--- | :--- |
| `help` | List all available BIOS monitor commands |
| `info` / `sysinfo` | Display CPU, EL, caches, RAM, heap, GICv2, timer, RTC, and PSCI summary |
| `post` | Re-run the 10-stage Power-On Self-Test diagnostic suite |
| `dtb [full]` | Show Flattened Device Tree hardware summary (or dump full FDT tree) |
| `memmap` | Display physical memory layout and BIOS RAM sections |
| `memtest [kb]` | Run 4-pattern RAM integrity test (default: 64 KB) |
| `devices` | Enumerate all buses (PCIe ECAM, VirtIO, `fw_cfg`, NOR Flash) |
| `pci` | Scan PCIe ECAM Bus 0 and show allocated BARs |
| `virtio` | Scan VirtIO MMIO slots and attached VirtIO Block drives |
| `fwcfg` | List files in the QEMU `fw_cfg` firmware directory |
| `flash` | Inspect CFI NOR Flash Bank 0 & Bank 1 |
| `ls [path]` | List files on an attached VirtIO FAT12/16/32 disk (default: `/`) |
| `cat <path>` | Display contents of a file on the VirtIO FAT disk |
| `boot [path\|auto]` | Boot OS/payload from `fw_cfg`, VirtIO FAT disk (`/EFI/BOOT/BOOTAA64.EFI`), or Flash |
| `demo [all\|elf\|efi]` | Execute built-in AArch64 ELF64 and PE32+ UEFI `.efi` self-contained demos |
| `regs` | Dump AArch64 architectural and system registers |
| `md <addr> [len]` | Hex + ASCII dump of physical memory (e.g., `md 0x40000000 64`) |
| `mw <addr> <val>` | Write a 64-bit value to physical memory |
| `date` / `uptime` | Show current UTC date/time from PL031 RTC and Generic Timer uptime |
| `trap` | Trigger a recoverable `BRK #0x42` breakpoint exception self-test |
| `irq` | Trigger a GICv2 Software Generated Interrupt (`SGI #1`) self-test |
| `panic` | Trigger a test panic to verify the bare-metal panic handler |
| `reboot` / `reset` | Reboot the virtual machine via PSCI `SYSTEM_RESET` |
| `shutdown` / `exit` | Power off the virtual machine cleanly via PSCI `SYSTEM_OFF` |

---

## Sample Boot & POST Output

```text
--------------------------------------------------
     Hinode BIOS (AArch64 Bare-Metal Firmware)    
--------------------------------------------------
Booting on CPU 0 (Exception Level EL1)...
Hello from Rust on AArch64 bare-metal QEMU!
BSS_TEST initial value (should be 0): 0
BSS_TEST after assignment (should be 42): 42
DATA_TEST initial value (should be 0x123456789ABCDEF): 0x123456789ABCDEF
DATA_TEST after assignment (should be 0xDEADBEEF): 0xDEADBEEF

--- Power-On Self-Test (POST) Diagnostics ---
  [ OK ] 01. Memory Sections    : .data (32 B) & .bss (176 B) verified in RAM
  [ OK ] 02. CPU & Architecture : ARM Limited Cortex-A57 (r1p0, EL1, PA=44b, FP/SIMD=ON)
  [ OK ] 03. Heap Allocator     : 1024 KB arena @ 0x40231000 (alloc/dealloc verified)
  [ OK ] 04. Exception Vectors  : VBAR_EL1=0x00000800, BRK #0x42 trap & resume OK
  [ OK ] 05. Timer & UART0      : CNTFRQ=62500000 Hz (62 MHz), PL011 ID=0x00141011
  [ OK ] 06. RTC & Power Events : PL031 (2026-11-16 17:10:02 UTC) & ACPI GED @ 0x09080000 (status=0x0)
  [ OK ] 07. GICv2 Interrupts   : 288 IRQs, 1 CPU(s), SGI #1 IRQ delivery verified
  [ OK ] 08. Device Tree (FDT)  : 0x40000000 (1048576 B, 128 MB RAM, 1 CPU, 54 nodes)
  [ OK ] 09. fw_cfg & PSCI      : QEMU fw_cfg (DMA=ON), PSCI v1.1 (hvc)
  [ OK ] 10. PCIe & VirtIO Bus  : 2 PCIe dev(s), 0 VirtIO-MMIO, 0 Block drive(s)
--- POST Complete: 10/10 checks passed in 20 ms ---

Entering Hinode BIOS Interactive Monitor. Type 'help' for commands.
hinode> 
```
