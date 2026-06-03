# Hinode BIOS (AArch64 Bare-Metal Firmware)

A minimal, bare-metal BIOS (firmware) for AArch64 written in Rust. It is configured to run on the QEMU `virt` machine, demonstrating bootstrapping, memory section loading (copying `.data` to RAM and zero-initializing `.bss`), and character output via a custom PL011 UART driver.

---

## File Structure

- [.cargo/config.toml](file:///home/takano32/GitHub/hinode-bios/.cargo/config.toml): Configures Cargo to target `aarch64-unknown-none` and specifies custom linker arguments.
- [linker.ld](file:///home/takano32/GitHub/hinode-bios/linker.ld): A linker script mapping code (`.text` and `.rodata`) to Flash memory and volatile data (`.data`, `.bss`, and stack) to RAM.
- [src/entry.S](file:///home/takano32/GitHub/hinode-bios/src/entry.S): Low-level assembly entry point that handles multi-core CPU parking, stack initialization, copying initialized data to RAM, zeroing the BSS section, and jumping to Rust.
- [src/main.rs](file:///home/takano32/GitHub/hinode-bios/src/main.rs): The `#![no_std]` Rust entry point (`rust_start`), a custom PL011 UART printing macro (`println!`), memory validation routines, and a custom panic handler.
- [Makefile](file:///home/takano32/GitHub/hinode-bios/Makefile): A convenience Makefile to build, package, and launch the BIOS in QEMU.

---

## Architecture & Boot Flow

### 1. Memory Map (QEMU `virt` machine)
The AArch64 `virt` machine maps peripherals and memory as follows:
- **`0x0000_0000` onwards**: Flash Memory (ROM). The BIOS binary is loaded here via the `-bios` flag.
- **`0x0900_0000`**: PL011 UART peripheral registers.
- **`0x4000_0000` onwards**: System RAM.

### 2. Boot Sequence (`src/entry.S`)
When the QEMU virtual machine powers on:
1. **CPU Parking**: AArch64 systems start all CPU cores simultaneously. We query the CPU affinity register `mpidr_el1`. If the core ID is not `0` (secondary CPUs), it is parked in a loop using `wfe` (Wait For Event) to prevent race conditions.
2. **Stack Setup**: We load the stack pointer `sp` with the address of `_stack_top` (`0x4001_0010` in RAM), reserving a 64KB stack area.
3. **Data Section Initialization**: The `.data` section's initial values are located in Flash (`LMA`), but its runtime destination is in RAM (`VMA`). The assembly code copies these bytes from Flash (`_rom_end`) to RAM (`_sdata` through `_edata`).
4. **BSS Section Initialization**: The uninitialized data segment (`.bss`) is cleared by writing zeroes to RAM from `_sbss` through `_ebss`.
5. **Call Rust Entry**: We jump to `rust_start` using a branch-with-link (`bl`) instruction.

### 3. Rust Code (`src/main.rs`)
- **PL011 UART Driver**: We write to the Data Register (`UARTDR` at `0x0900_0000`) only after checking that the Transmit FIFO is not full (checking bit 5 of the Flag Register `UARTFR` at `0x0900_0018`).
- **Memory Integrity Test**: We verify that BSS clearing and data copying worked by checking the values of static variables (`BSS_TEST` and `DATA_TEST`) before and after mutating them.
- **Panic Handling**: A custom `panic_handler` intercepts errors and prints the file, line number, and panic message directly to the serial console, which facilitates easier bare-metal debugging.

---

## Getting Started

### Prerequisites

You need the AArch64 bare-metal target for Rust and the `qemu-system-aarch64` emulator.

**For Debian/Ubuntu-based systems:**
```bash
# Add the target using rustup
rustup target add aarch64-unknown-none

# Install QEMU for AArch64
sudo apt update
sudo apt install qemu-system-arm
```

### Build Instructions

To build the project in optimized release mode and generate a flat raw binary (`hinode_bios.bin`):

```bash
make build
```

This compiles the Rust code, links it via `linker.ld`, and converts the resulting ELF executable into a flat binary file using `objcopy`.

### Running in QEMU

Launch the BIOS inside QEMU with:

```bash
make run
```

---

## Expected Output

Upon running QEMU, the following output should display in your console:

```text
--------------------------------------------------
     Hinode BIOS (AArch64 Bare-Metal Firmware)    
--------------------------------------------------
Booting on CPU 0...
Hello from Rust on AArch64 bare-metal QEMU!
BSS_TEST initial value (should be 0): 0
BSS_TEST after assignment (should be 42): 42
DATA_TEST initial value (should be 0x123456789ABCDEF): 0x123456789ABCDEF
DATA_TEST after assignment (should be 0xDEADBEEF): 0xDEADBEEF
Triggering a test panic to verify panic output...

!!! PANIC !!!
panicked at src/main.rs:82:5:
This is a test panic from Hinode BIOS!
```

> [!NOTE]
> Once the program hits the panic handler, it enters an infinite loop. 
> To terminate QEMU, press **`Ctrl+A`** followed by **`x`**.
