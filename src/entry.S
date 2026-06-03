.section .text.entry, "ax"
.global _start

_start:
    // Park secondary CPUs
    mrs x0, mpidr_el1
    and x0, x0, #0xff
    cbz x0, .Lcpu0
.Lpark:
    wfe
    b .Lpark

.Lcpu0:
    // Set stack pointer
    ldr x0, =_stack_top
    mov sp, x0

    // Copy .data section from FLASH (load address) to RAM (virtual address)
    ldr x0, =_rom_end
    ldr x1, =_sdata
    ldr x2, =_edata
.Lcopy_data_loop:
    cmp x1, x2
    b.hs .Lcopy_data_done
    ldr x3, [x0], #8
    str x3, [x1], #8
    b .Lcopy_data_loop
.Lcopy_data_done:

    // Clear .bss section
    ldr x0, =_sbss
    ldr x1, =_ebss
    mov x2, #0
.Lclear_bss_loop:
    cmp x0, x1
    b.hs .Lclear_bss_done
    str x2, [x0], #8
    b .Lclear_bss_loop
.Lclear_bss_done:

    // Call Rust entry point
    bl rust_start

    // If rust_start returns, loop forever
.Lexit_loop:
    wfe
    b .Lexit_loop
