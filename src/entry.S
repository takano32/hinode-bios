.section .text.entry, "ax"
.global _start
.global _secondary_entry

_start:
    // Mask all interrupts (D, A, I, F) during early bootstrap
    msr daifset, #0xf

    // Park secondary CPUs (check Aff0, Aff1, Aff2 in MPIDR_EL1)
    mrs x0, mpidr_el1
    ldr x1, =0x00ffffff
    and x0, x0, x1
    cbz x0, .Lcpu0

.Lpark:
    wfe
    b .Lpark

.Lcpu0:
    // Configure Exception Level specific registers (EL3 / EL2 / EL1)
    mrs x0, CurrentEL
    lsr x0, x0, #2

    cmp x0, #3
    b.eq .Linit_el3
    cmp x0, #2
    b.eq .Linit_el2
    b .Linit_el1

.Linit_el3:
    // Enable FP/SIMD and timer access from lower ELs if started in EL3
    mov x1, #0
    msr cptr_el3, x1
    ldr x1, =_exception_vectors
    msr vbar_el3, x1

.Linit_el2:
    // Do not trap FP/SIMD or system registers in EL2
    mov x1, #0
    msr cptr_el2, x1
    // Route physical IRQs to EL2 (HCR_EL2.IMO = bit 4) when running in EL2
    mov x1, #(1 << 4)
    msr hcr_el2, x1
    // Allow EL1/EL0 access to physical counter & timer
    mov x1, #3
    msr cnthctl_el2, x1
    msr cntvoff_el2, xzr
    // Install exception vectors for EL2
    ldr x1, =_exception_vectors
    msr vbar_el2, x1

.Linit_el1:
    // Enable Floating-Point and Advanced SIMD (NEON) at EL1/EL0 (FPEN = 0b11 << 20)
    mrs x1, cpacr_el1
    orr x1, x1, #(3 << 20)
    msr cpacr_el1, x1

    // Install exception vectors for EL1
    ldr x1, =_exception_vectors
    msr vbar_el1, x1

    isb

    // Initialize stack pointer in RAM (above preserved DTB region)
    ldr x0, =_stack_top
    mov sp, x0

    // Copy .data section from FLASH (LMA: _rom_end) to RAM (VMA: _sdata.._edata)
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

    // Zero-initialize .bss section (_sbss.._ebss)
    ldr x0, =_sbss
    ldr x1, =_ebss
.Lclear_bss_loop:
    cmp x0, x1
    b.hs .Lclear_bss_done
    str xzr, [x0], #8
    b .Lclear_bss_loop
.Lclear_bss_done:

    dsb sy
    isb

    // Jump to Rust firmware entry point
    bl rust_start

    // If rust_start ever returns, halt cleanly
.Lexit_loop:
    wfe
    b .Lexit_loop


// ============================================================================
// AArch64 Exception Vector Table (2048-byte aligned, 16 entries x 128 bytes)
// ============================================================================
.macro VECTOR_ENTRY id
    .balign 0x80
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    mov x1, #\id
    b .Lcommon_exception
.endm

.section .text.vectors, "ax"
.balign 0x800
.global _exception_vectors
_exception_vectors:
    // Current EL with SP_EL0
    VECTOR_ENTRY 0   // Synchronous
    VECTOR_ENTRY 1   // IRQ / vIRQ
    VECTOR_ENTRY 2   // FIQ / vFIQ
    VECTOR_ENTRY 3   // SError / vSError

    // Current EL with SP_ELx
    VECTOR_ENTRY 4   // Synchronous
    VECTOR_ENTRY 5   // IRQ / vIRQ
    VECTOR_ENTRY 6   // FIQ / vFIQ
    VECTOR_ENTRY 7   // SError / vSError

    // Lower EL using AArch64
    VECTOR_ENTRY 8   // Synchronous
    VECTOR_ENTRY 9   // IRQ / vIRQ
    VECTOR_ENTRY 10  // FIQ / vFIQ
    VECTOR_ENTRY 11  // SError / vSError

    // Lower EL using AArch32
    VECTOR_ENTRY 12  // Synchronous
    VECTOR_ENTRY 13  // IRQ / vIRQ
    VECTOR_ENTRY 14  // FIQ / vFIQ
    VECTOR_ENTRY 15  // SError / vSError

.Lcommon_exception:
    // Save remaining General Purpose Registers x2..x30
    stp x2,  x3,  [sp, #16]
    stp x4,  x5,  [sp, #32]
    stp x6,  x7,  [sp, #48]
    stp x8,  x9,  [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]

    // Save x30 (LR) and original SP (sp + 288)
    add x2, sp, #288
    stp x30, x2,  [sp, #240]

    // Read ELR, SPSR, ESR, FAR according to CurrentEL
    mrs x2, CurrentEL
    lsr x2, x2, #2
    cmp x2, #2
    b.eq .Lsave_el2_sysregs

    mrs x2, elr_el1
    mrs x3, spsr_el1
    mrs x4, esr_el1
    mrs x5, far_el1
    b .Lsave_sysregs_done

.Lsave_el2_sysregs:
    mrs x2, elr_el2
    mrs x3, spsr_el2
    mrs x4, esr_el2
    mrs x5, far_el2

.Lsave_sysregs_done:
    stp x2, x3, [sp, #256]
    stp x4, x5, [sp, #272]

    // Call Rust exception handler: rust_exception_handler(ctx: *mut ExceptionContext, vector_id: u64)
    mov x0, sp
    // x1 already holds vector_id
    bl rust_exception_handler

    // Restore potentially updated ELR and SPSR
    ldp x2, x3, [sp, #256]
    mrs x4, CurrentEL
    lsr x4, x4, #2
    cmp x4, #2
    b.eq .Lrestore_el2_sysregs

    msr elr_el1, x2
    msr spsr_el1, x3
    b .Lrestore_gprs

.Lrestore_el2_sysregs:
    msr elr_el2, x2
    msr spsr_el2, x3

.Lrestore_gprs:
    ldr x30,      [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8,  x9,  [sp, #64]
    ldp x6,  x7,  [sp, #48]
    ldp x4,  x5,  [sp, #32]
    ldp x2,  x3,  [sp, #16]
    ldp x0,  x1,  [sp, #0]
    add sp, sp, #288
    eret
