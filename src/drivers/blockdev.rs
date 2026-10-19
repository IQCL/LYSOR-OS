//! LYSOR OS block device abstraction (`ly_blk`).
//!
//! Real device interface shared by all storage backends:
//!   - ATA disk   (`/dev/sda`, 512-byte sectors)
//!   - RAM disk   (`/dev/ram0`, 4096-byte blocks)
//!
//! A driver registers a `LyBlockDev`, the filesystem layer opens it by name
//! and issues block reads/writes through `ly_blk_read` / `ly_blk_write`.
//! This is the single mount point for real devices — no RAM-array stand-ins.

use crate::serial;

pub const BLK_MAX_DEVS: usize = 8;
pub const BLK_NAME_MAX: usize = 16;

/// A registered block device. `read_block`/`write_block` transfer exactly
/// `block_size` bytes; return 0 on success or a negative errno.
#[repr(C)]
pub struct LyBlockDev {
    pub name: &'static str,
    pub block_size: u32,
    pub block_count: u64,
    pub read_block: extern "C" fn(dev: u32, block: u64, buf: *mut u8) -> i32,
    pub write_block: extern "C" fn(dev: u32, block: u64, buf: *const u8) -> i32,
}

static mut DEVS: [Option<LyBlockDev>; BLK_MAX_DEVS] = [const { None }; BLK_MAX_DEVS];

/// Register a block device. Returns the device id used by all other calls.
pub fn ly_blk_register(dev: LyBlockDev) -> Result<u32, &'static str> {
    unsafe {
        for i in 0..BLK_MAX_DEVS {
            if DEVS[i].is_none() {
                DEVS[i] = Some(dev);
                return Ok(i as u32);
            }
        }
    }
    Err("block device table full")
}

/// Open (look up) a block device by name, e.g. "sda" or "ram0".
pub fn ly_blk_open(name: &str) -> Result<u32, &'static str> {
    unsafe {
        for i in 0..BLK_MAX_DEVS {
            if let Some(d) = &DEVS[i] {
                if d.name == name {
                    return Ok(i as u32);
                }
            }
        }
    }
    Err("no such block device")
}

/// Query a device's geometry: (block_size, block_count).
pub fn ly_blk_info(dev: u32) -> Option<(u32, u64)> {
    unsafe {
        DEVS[dev as usize].as_ref().map(|d| (d.block_size, d.block_count))
    }
}

/// Read one device block into `buf` (buf.len() must equal block_size).
pub fn ly_blk_read(dev: u32, block: u64, buf: &mut [u8]) -> i32 {
    unsafe {
        match &DEVS[dev as usize] {
            Some(d) => {
                if buf.len() != d.block_size as usize || block >= d.block_count {
                    return -22; // EINVAL
                }
                (d.read_block)(dev, block, buf.as_mut_ptr())
            }
            None => -19, // ENODEV
        }
    }
}

/// Write one device block from `buf`.
pub fn ly_blk_write(dev: u32, block: u64, buf: &[u8]) -> i32 {
    unsafe {
        match &DEVS[dev as usize] {
            Some(d) => {
                if buf.len() != d.block_size as usize || block >= d.block_count {
                    return -22; // EINVAL
                }
                (d.write_block)(dev, block, buf.as_ptr())
            }
            None => -19, // ENODEV
        }
    }
}

/// Scratch buffer for byte-range IO (sized to the largest supported block).
const SCRATCH_SIZE: usize = 4096;
static mut SCRATCH: [u8; SCRATCH_SIZE] = [0u8; SCRATCH_SIZE];

/// Read an arbitrary byte range (offsets are byte-based). Loops device blocks.
pub fn ly_blk_read_bytes(dev: u32, offset: u64, buf: &mut [u8]) -> i32 {
    let (bs, _) = match ly_blk_info(dev) {
        Some(g) => g,
        None => return -19,
    };
    if bs as usize > SCRATCH_SIZE {
        return -22;
    }
    let bs = bs as u64;
    let mut pos = offset;
    let mut done = 0usize;
    unsafe {
        while done < buf.len() {
            let block = pos / bs;
            let in_block = (pos % bs) as usize;
            let n = core::cmp::min(bs as usize - in_block, buf.len() - done);
            let r = ly_blk_read(dev, block, &mut SCRATCH[..bs as usize]);
            if r < 0 {
                return r;
            }
            buf[done..done + n].copy_from_slice(&SCRATCH[in_block..in_block + n]);
            done += n;
            pos += n as u64;
        }
    }
    0
}

/// Write an arbitrary byte range. Loops device blocks (read-modify-write
/// for partial blocks so adjacent data is preserved).
pub fn ly_blk_write_bytes(dev: u32, offset: u64, buf: &[u8]) -> i32 {
    let (bs, _) = match ly_blk_info(dev) {
        Some(g) => g,
        None => return -19,
    };
    if bs as usize > SCRATCH_SIZE {
        return -22;
    }
    let bs = bs as u64;
    let mut pos = offset;
    let mut done = 0usize;
    unsafe {
        while done < buf.len() {
            let block = pos / bs;
            let in_block = (pos % bs) as usize;
            let n = core::cmp::min(bs as usize - in_block, buf.len() - done);
            // partial block: read-modify-write
            let full = in_block == 0 && n == bs as usize;
            if !full {
                let r = ly_blk_read(dev, block, &mut SCRATCH[..bs as usize]);
                if r < 0 {
                    return r;
                }
            }
            SCRATCH[in_block..in_block + n].copy_from_slice(&buf[done..done + n]);
            let r = ly_blk_write(dev, block, &SCRATCH[..bs as usize]);
            if r < 0 {
                return r;
            }
            done += n;
            pos += n as u64;
        }
    }
    0
}

/// Total capacity of a device in bytes.
pub fn ly_blk_capacity(dev: u32) -> u64 {
    match ly_blk_info(dev) {
        Some((bs, cnt)) => bs as u64 * cnt,
        None => 0,
    }
}

/// Dump the device table (boot diagnostic).
pub fn ly_blk_dump() {
    unsafe {
        for i in 0..BLK_MAX_DEVS {
            if let Some(d) = &DEVS[i] {
                serial::log("   blk: dev");
                serial::log_u(i as u64);
                serial::log(" \"");
                serial::log(d.name);
                serial::log("\" size=");
                serial::log_u(d.block_size as u64);
                serial::log(" blocks=");
                serial::log_u(d.block_count);
                serial::log("\r\n");
            }
        }
    }
}
