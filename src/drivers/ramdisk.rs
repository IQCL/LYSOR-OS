//! LYSOR OS RAM disk driver (`ly_ram0`).
//!
//! A real block device backed by a static RAM buffer. Used as the early-boot
//! root (when no ATA disk is attached) and as a scratch volume. It implements
//! the same `LyBlockDev` interface as the ATA disk, so LFS mounts identically
//! on `/dev/ram0` and `/dev/sda` — no special-casing in the filesystem.

use crate::drivers::blockdev;
use crate::serial;

pub const RAMDISK_SIZE: usize = 16 * 1024 * 1024; // 16 MiB
pub const RAMDISK_BLOCK: u32 = 4096;

#[repr(align(4096))]
struct RamBuf {
    data: [u8; RAMDISK_SIZE],
}

static mut RAM: RamBuf = RamBuf { data: [0u8; RAMDISK_SIZE] };
static mut REGISTERED: bool = false;

extern "C" fn ram_read_block(dev: u32, block: u64, buf: *mut u8) -> i32 {
    unsafe {
        let _ = dev;
        let off = (block as usize) * RAMDISK_BLOCK as usize;
        if off + RAMDISK_BLOCK as usize > RAMDISK_SIZE {
            return -22; // EINVAL
        }
        core::ptr::copy_nonoverlapping(RAM.data.as_ptr().add(off), buf, RAMDISK_BLOCK as usize);
    }
    0
}

extern "C" fn ram_write_block(dev: u32, block: u64, buf: *const u8) -> i32 {
    unsafe {
        let _ = dev;
        let off = (block as usize) * RAMDISK_BLOCK as usize;
        if off + RAMDISK_BLOCK as usize > RAMDISK_SIZE {
            return -22;
        }
        core::ptr::copy_nonoverlapping(buf, RAM.data.as_mut_ptr().add(off), RAMDISK_BLOCK as usize);
    }
    0
}

/// Register the ramdisk (idempotent).
pub fn ramdisk_init() {
    unsafe {
        if REGISTERED {
            return;
        }
        let dev = blockdev::LyBlockDev {
            name: "ram0",
            block_size: RAMDISK_BLOCK,
            block_count: (RAMDISK_SIZE / RAMDISK_BLOCK as usize) as u64,
            read_block: ram_read_block,
            write_block: ram_write_block,
        };
        match blockdev::ly_blk_register(dev) {
            Ok(_) => {
                REGISTERED = true;
                serial::log("   ramdisk: /dev/ram0 ready (16 MiB, 4K blocks)\r\n");
            }
            Err(e) => serial::log(e),
        }
    }
}
