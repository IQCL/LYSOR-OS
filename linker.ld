/* LYSOR OS linker script - x86_64 kernel
 * Linked at 1MiB physical, identity-mapped during boot.
 */

ENTRY(_start)

SECTIONS
{
    . = 0x100000;

    /* QEMU PVH boot note */
    .note.pvh : {
        KEEP(*(.note.pvh))
    }

    . = ALIGN(4K);
    _text = .;
    .text : {
        *(.text.boot)
        *(.text .text.*)
    }
    _etext = .;

    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
    }

    . = ALIGN(4K);
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
    _end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note.gnu.build-id)
        *(.note.ABI-tag)
    }
}
