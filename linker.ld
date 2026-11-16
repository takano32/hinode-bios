ENTRY(_start)

SECTIONS
{
    /* QEMU virt NOR Flash Bank 0 base address */
    . = 0x00000000;

    _rom_start = .;

    .text : {
        KEEP(*(.text.entry))
        . = ALIGN(0x800);
        KEEP(*(.text.vectors))
        *(.text .text.*)
    }

    .rodata : {
        . = ALIGN(8);
        *(.rodata .rodata.*)
    }

    . = ALIGN(16);
    _rom_end = .;

    /*
     * System RAM starts at 0x40000000 on QEMU virt.
     * QEMU places the Flattened Device Tree (DTB) at 0x40000000 (up to 1MB).
     * We reserve 0x40000000 .. 0x40200000 (2MB) so the DTB is never overwritten.
     */
    _ram_base = 0x40000000;
    _dtb_start = 0x40000000;
    _dtb_max_end = 0x40200000;

    . = 0x40200000;
    _ram_start = .;

    .data : AT(_rom_end) {
        . = ALIGN(16);
        _sdata = .;
        *(.data .data.*)
        . = ALIGN(16);
        _edata = .;
    }

    .bss (NOLOAD) : {
        . = ALIGN(16);
        _sbss = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        _ebss = .;
    }

    . = ALIGN(16);
    _stack_bottom = .;
    . = . + 0x20000; /* 128KB BIOS stack */
    _stack_top = .;

    . = ALIGN(4096);
    _dma_start = .;
    . = . + 0x10000; /* 64KB page-aligned DMA & VirtIO ring region */
    _dma_end = .;

    . = ALIGN(16);
    _heap_start = .;
    . = . + 0x100000; /* 1MB BIOS heap */
    _heap_end = .;

    _ram_end = .;

    /* Free RAM region for loading OS kernels / UEFI / ELF payloads */
    . = 0x40400000;
    _payload_start = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.comment)
    }
}
