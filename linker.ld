ENTRY(_start)

SECTIONS
{
    /* QEMU virt flash address */
    . = 0x00000000;

    _rom_start = .;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(8);
    _rom_end = .;

    /* RAM starts at 0x40000000 */
    . = 0x40000000;

    _ram_start = .;

    .data : AT(_rom_end) {
        . = ALIGN(8);
        _sdata = .;
        *(.data .data.*)
        . = ALIGN(8);
        _edata = .;
    }

    .bss : {
        . = ALIGN(8);
        _sbss = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        _ebss = .;
    }

    . = ALIGN(16);
    _stack_bottom = .;
    . = . + 0x10000; /* 64KB stack */
    _stack_top = .;

    _ram_end = .;
}
