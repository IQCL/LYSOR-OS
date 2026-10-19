//! GDT setup for LYSOR OS.

use core::arch::asm;

#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GdtEntry(u64);

impl GdtEntry {
    const fn null() -> Self { GdtEntry(0) }
    const fn kernel_code() -> Self { GdtEntry(0x00AF9A00_0000FFFF) }
    const fn kernel_data() -> Self { GdtEntry(0x00CF9200_0000FFFF) }
    const fn user_code() -> Self { GdtEntry(0x00AFFA00_0000FFFF) }
    const fn user_data() -> Self { GdtEntry(0x00CFF200_0000FFFF) }
}

#[repr(C)]
struct GdtTable {
    null: GdtEntry,
    kcode: GdtEntry,
    kdata: GdtEntry,
    ucode: GdtEntry,
    udata: GdtEntry,
}

static mut GDT: GdtTable = GdtTable {
    null: GdtEntry::null(),
    kcode: GdtEntry::kernel_code(),
    kdata: GdtEntry::kernel_data(),
    ucode: GdtEntry::user_code(),
    udata: GdtEntry::user_data(),
};

pub fn init_gdt() {
    let ptr = GdtPtr {
        limit: (core::mem::size_of::<GdtTable>() - 1) as u16,
        base: unsafe { &raw const GDT as u64 },
    };
    unsafe {
        asm!(
            "lgdt [{}]",
            in(reg) &ptr,
            options(readonly, nostack)
        );
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            options(nostack)
        );
    }
    crate::serial::log("   GDT: loaded, CS=0x08 DS=0x10\r\n");
}

pub const KCODE_SEL: u16 = 0x08;
pub const KDATA_SEL: u16 = 0x10;
pub const UCODE_SEL: u16 = 0x18 | 3;
pub const UDATA_SEL: u16 = 0x20 | 3;
