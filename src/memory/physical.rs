//! Bitmap physical frame allocator.
//!
//! We parse the multiboot2 memory map, mark reserved ranges (kernel image,
//! boot-reclaimable, framebuffer), and hand out 4 KiB frames.
//!
//! Scope: physical address space up to 4 GiB (1,048,576 frames -> 128 KiB bitmap).

use crate::serial;

const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 1024 * 1024; // 4 GiB
const BITMAP_WORDS: usize = MAX_FRAMES / 64;

#[repr(align(64))]
struct Bitmap {
    words: [u64; BITMAP_WORDS],
}

static mut BITMAP: Bitmap = Bitmap {
    words: [0u64; BITMAP_WORDS],
};

static mut TOTAL_FRAMES: usize = 0;
static mut USED_FRAMES: usize = 0;

// ---------------------------------------------------------------------------
// Multiboot2 tag parsing
// ---------------------------------------------------------------------------

#[repr(C)]
struct MbTagHeader {
    typ: u32,
    size: u32,
}

#[repr(C)]
struct MbMmapEntry {
    addr: u64,
    len: u64,
    typ: u32,
    zero: u32,
}

unsafe fn cmos_read(reg: u8) -> u8 {
    core::arch::asm!("out dx, al", in("dx") 0x70u16, in("al") reg, options(nostack));
    core::arch::asm!("nop", options(nostack));
    let v: u8;
    core::arch::asm!("in al, dx", in("dx") 0x71u16, out("al") v, options(nostack));
    v
}

/// Probe the total RAM from QEMU fw_cfg (item 0x11 = RAM size in bytes,
/// 8 bytes big-endian). Returns None when fw_cfg is unavailable.
unsafe fn fw_cfg_ram_size() -> Option<u64> {
    // FW_CFG_RAM_SIZE (0x03) selector on I/O ports 0x510 (select) / 0x511 (data).
    core::arch::asm!("out dx, eax", in("dx") 0x510u16, in("eax") 0x03u32, options(nostack));
    let mut v = [0u8; 8];
    for b in v.iter_mut() {
        let mut byte: u8 = 0;
        core::arch::asm!("in al, dx", in("dx") 0x511u16, out("al") byte, options(nostack));
        *b = byte;
    }
    // debug: print the raw bytes
    crate::serial::log("   ram-probe: fw_cfg bytes ");
    for b in v.iter() {
        crate::serial::log_hex(*b as u64);
        crate::serial::log(" ");
    }
    crate::serial::log("\r\n");
    let ram = u64::from_be_bytes(v);
    if ram > (4 * 1024 * 1024 * 1024) || ram < (16 * 1024 * 1024) {
        None
    } else {
        Some(ram)
    }
}

/// Total physical RAM in bytes. Tries fw_cfg (reliable under QEMU even
/// without a BIOS), then CMOS extended-memory registers, then a 256 MiB
/// default when neither answers.
unsafe fn ram_total_bytes() -> usize {
    // Command-line override wins: `ram=NNN` (MiB) on the kernel cmdline.
    let ovr = crate::ly_kcmdline_ram_mib();
    if ovr > 0 {
        return ovr as usize * 1024 * 1024;
    }
    // fw_cfg RAM_SIZE; ignore absurd values.
    if let Some(ram) = fw_cfg_ram_size() {
        if ram >= (32 * 1024 * 1024) {
            return ram as usize;
        }
    }
    // CMOS extended-memory registers (above 16 MiB, KiB). Anything below
    // 32 MiB total is treated as a probe failure.
    let lo = cmos_read(0x35) as usize;
    let hi = cmos_read(0x36) as usize;
    let ext_kb = (hi << 8) | lo;
    if ext_kb > (16 * 1024) {
        return (16 * 1024 * 1024) + ext_kb * 1024;
    }
    // Default when nothing answered reliably.
    256 * 1024 * 1024
}

/// # Safety
/// `mb_info` must be a valid physical pointer to a multiboot2 info struct,
/// passed by the bootloader in ebx.
pub unsafe fn init_physical(mb_info: usize) {
    // First pass: mark ALL frames as used.
    for w in BITMAP.words.iter_mut() {
        *w = u64::MAX;
    }
    USED_FRAMES = MAX_FRAMES;

    // PVH boot: probe real RAM size from CMOS (regs 0x35/0x36 = extended
    // memory above 16 MiB, in KiB) instead of hardcoding 256 MiB. This makes
    // the physical allocator adaptive to the actual QEMU -m size.
    let total_bytes = ram_total_bytes();
    serial::log("   PVH boot: probed ");
    serial::log_u((total_bytes / (1024 * 1024)) as u64);
    serial::log(" MiB physical RAM (adaptive)\r\n");
    if total_bytes > 0x100000 {
        mark_free(0x100000, total_bytes - 0x100000);
    } else {
        // unreachable fallback
        mark_free(0x100000, 256 * 1024 * 1024 - 0x100000);
    }

    // Reserve the kernel image itself.
    extern "C" {
        static _end: u8;
    }
    let kernel_end = &raw const _end as usize;
    serial::log("   kernel end: 0x");
    serial::log_hex(kernel_end as u64);
    serial::log("\r\n");
    reserve(0x100000, kernel_end - 0x100000);

    // Also reserve the first 1MB (IVT, BDA, BIOS, etc.)
    reserve(0, 0x100000);

    TOTAL_FRAMES = MAX_FRAMES;
    serial::log("   physical: ");
    serial::log_u(USED_FRAMES as u64);
    serial::log(" / ");
    serial::log_u(MAX_FRAMES as u64);
    serial::log(" frames used\r\n");
}

unsafe fn mark_free(base: usize, len: usize) {
    let start_frame = base / FRAME_SIZE;
    let end_frame = (base + len + FRAME_SIZE - 1) / FRAME_SIZE;
    for f in start_frame..end_frame {
        if f >= MAX_FRAMES {
            break;
        }
        clear_bit(f);
    }
}

unsafe fn reserve(base: usize, len: usize) {
    let start_frame = base / FRAME_SIZE;
    let end_frame = (base + len + FRAME_SIZE - 1) / FRAME_SIZE;
    for f in start_frame..end_frame {
        if f >= MAX_FRAMES {
            break;
        }
        set_bit(f);
    }
}

unsafe fn set_bit(frame: usize) {
    let word = frame / 64;
    let bit = frame % 64;
    if BITMAP.words[word] & (1u64 << bit) == 0 {
        BITMAP.words[word] |= 1u64 << bit;
        USED_FRAMES += 1;
    }
}

unsafe fn clear_bit(frame: usize) {
    let word = frame / 64;
    let bit = frame % 64;
    if BITMAP.words[word] & (1u64 << bit) != 0 {
        BITMAP.words[word] &= !(1u64 << bit);
        USED_FRAMES -= 1;
    }
}

/// Allocate a single 4 KiB frame. Returns the physical address.
pub fn alloc_frame() -> Option<u64> {
    unsafe {
        for (wi, w) in BITMAP.words.iter().enumerate() {
            if *w != u64::MAX {
                // find first zero bit
                let inv = !*w;
                let bit = inv.trailing_zeros() as usize;
                let frame = wi * 64 + bit;
                set_bit(frame);
                return Some((frame * FRAME_SIZE) as u64);
            }
        }
        None
    }
}

/// Free a previously allocated frame.
pub fn free_frame(paddr: u64) {
    unsafe {
        let frame = (paddr as usize) / FRAME_SIZE;
        clear_bit(frame);
    }
}

pub fn frame_total() -> usize {
    unsafe { TOTAL_FRAMES }
}

pub fn frame_used_count() -> usize {
    unsafe { USED_FRAMES }
}
