//! LFS - LYSOR File System (real, block-device-backed).
//!
//! A from-scratch extent filesystem that lives on a real block device
//! (`/dev/sda` ATA disk or `/dev/ram0` ramdisk) through the `ly_blk` layer.
//! No RAM-array stand-in: every metadata block and data block is read from /
//! written to the device.
//!
//! Layout (all little-endian, blocks of 4096 bytes):
//!   block 0            : superblock
//!   block 1            : inode bitmap (64 inodes)
//!   blocks 2..2+bmp    : block bitmap (1 bit per LFS block)
//!   blocks 2+bmp..     : inode table (64 x 128B = 2 blocks)
//!   data_start ..      : data blocks
//!
//! Files up to 12 direct + 1 indirect block (1024 ptrs) = ~4 MiB.
//! Every inode and the superblock carry a CRC32 integrity check.

use crate::serial;
use crate::drivers::blockdev;

pub const LFS_MAGIC: u32 = 0x4C465331; // "LFS1"
pub const LFS_VERSION: u32 = 1;
pub const LFS_BLOCK: usize = 4096;
pub const LFS_INODES: usize = 64;
pub const LFS_INODE_SIZE: usize = 128;
pub const LFS_INODE_TBL_BLOCKS: usize = (LFS_INODES * LFS_INODE_SIZE + LFS_BLOCK - 1) / LFS_BLOCK;
pub const LFS_DIRECT: usize = 12;
pub const LFS_INDIRECT_PTRS: usize = LFS_BLOCK / 4; // 1024
pub const LFS_NAME_MAX: usize = 56;
pub const LFS_DTYPE_FILE: u8 = 0;
pub const LFS_DTYPE_DIR: u8 = 1;
pub const LFS_SUPER_CKSUM_OFF: usize = 44;  // offset of LfsSuper.checksum
pub const LFS_INODE_CKSUM_OFF: usize = 76;  // offset of LfsInode.checksum

/// Superblock (first 256 bytes of block 0).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LfsSuper {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub lfs_blocks: u32,
    pub free_blocks: u32,
    pub inode_count: u32,
    pub free_inodes: u32,
    pub bmp_start: u32,
    pub inode_tbl_start: u32,
    pub data_start: u32,
    pub flags: u32,     // bit0 = dirty
    pub checksum: u32,
}

impl LfsSuper {
    const fn new() -> Self {
        LfsSuper {
            magic: 0, version: 0, block_size: 0, lfs_blocks: 0,
            free_blocks: 0, inode_count: 0, free_inodes: 0,
            bmp_start: 0, inode_tbl_start: 0, data_start: 0,
            flags: 0, checksum: 0,
        }
    }
}

/// Inode (128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LfsInode {
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub nlink: u16,
    pub size: u32,
    pub blocks: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub direct: [u32; LFS_DIRECT],
    pub indirect: u32,
    pub checksum: u32,
    pub parent: u32,   // parent inode (for "..")
    pub _pad: [u8; 44],
}

impl LfsInode {
    const fn new() -> Self {
        LfsInode {
            mode: 0, uid: 0, gid: 0, nlink: 0, size: 0, blocks: 0,
            ctime: 0, mtime: 0, direct: [0u32; LFS_DIRECT], indirect: 0,
            checksum: 0, parent: u32::MAX, _pad: [0u8; 44],
        }
    }
}

/// Directory entry (64 bytes, 64 per block).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LfsDirent {
    pub ino: u32,
    pub name_len: u16,
    pub ftype: u8,
    pub _pad: u8,
    pub name: [u8; LFS_NAME_MAX],
}

impl LfsDirent {
    pub const fn empty() -> Self {
        LfsDirent { ino: 0, name_len: 0, ftype: 0, _pad: 0, name: [0u8; LFS_NAME_MAX] }
    }
}

const DIRENTS_PER_BLOCK: usize = LFS_BLOCK / core::mem::size_of::<LfsDirent>(); // 64

// --- CRC32 (IEEE 802.3, poly 0xEDB88320) ---

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
            j += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFFFFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFFFFFF
}

// --- Mount state ------------------------------------------------------------

static mut SUPER: LfsSuper = LfsSuper::new();
static mut MOUNTED_DEV: u32 = u32::MAX;
static mut LFS_BLOCKS: u32 = 0;
static mut BMP_BLOCKS: u32 = 0;
static mut DATA_START: u32 = 0;
static mut BLK_BITMAP: [u8; 8192] = [0u8; 8192]; // up to 65536 blocks (256 MiB)
static mut INO_BITMAP: [u8; 8] = [0u8; 8];       // 64 inodes

// ---------------------------------------------------------------------------
// Low-level device IO (LFS blocks of 4096 bytes)
// ---------------------------------------------------------------------------

fn dev_read_block(blk: u32, buf: &mut [u8; LFS_BLOCK]) -> Result<(), &'static str> {
    let dev = unsafe { MOUNTED_DEV };
    let r = blockdev::ly_blk_read_bytes(dev, blk as u64 * LFS_BLOCK as u64, buf);
    if r < 0 { Err("lfs: device read error") } else { Ok(()) }
}

fn dev_write_block(blk: u32, buf: &[u8; LFS_BLOCK]) -> Result<(), &'static str> {
    let dev = unsafe { MOUNTED_DEV };
    let r = blockdev::ly_blk_write_bytes(dev, blk as u64 * LFS_BLOCK as u64, buf);
    if r < 0 { Err("lfs: device write error") } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// Bitmap helpers (write-through)
// ---------------------------------------------------------------------------

/// Live filesystem stats: (free_blocks, total_blocks) for the Settings app.
pub fn ly_lfs_stats() -> (u32, u32) {
    unsafe {
        let free = SUPER.free_blocks;
        let total = LFS_BLOCKS;
        (free, total)
    }
}

fn bit_get(bitmap: &[u8], bit: u32) -> bool {
    (bitmap[(bit / 8) as usize] >> (bit % 8)) & 1 != 0
}

fn bit_set(bitmap: &mut [u8], bit: u32) {
    bitmap[(bit / 8) as usize] |= 1 << (bit % 8);
}

fn bit_clear(bitmap: &mut [u8], bit: u32) {
    bitmap[(bit / 8) as usize] &= !(1 << (bit % 8));
}

fn persist_block_bitmap() -> Result<(), &'static str> {
    unsafe {
        let mut b = [0u8; LFS_BLOCK];
        let total = (LFS_BLOCKS + 7) / 8;
        let n = core::cmp::min(total as usize, BLK_BITMAP.len());
        b[..n].copy_from_slice(&BLK_BITMAP[..n]);
        for i in 0..BMP_BLOCKS {
            let mut buf = [0u8; LFS_BLOCK];
            if (i as usize) * LFS_BLOCK < b.len() {
                let from = (i as usize) * LFS_BLOCK;
                let to = core::cmp::min(from + LFS_BLOCK, b.len());
                buf[..to - from].copy_from_slice(&b[from..to]);
            }
            dev_write_block(SUPER.bmp_start + i, &buf)?;
        }
    }
    Ok(())
}

fn persist_inode_bitmap() -> Result<(), &'static str> {
    unsafe {
        let mut b = [0u8; LFS_BLOCK];
        b[..8].copy_from_slice(&INO_BITMAP);
        dev_write_block(1, &b)
    }
}

fn load_bitmaps() -> Result<(), &'static str> {
    unsafe {
        let mut b = [0u8; LFS_BLOCK];
        // block bitmap
        for i in 0..BMP_BLOCKS {
            dev_read_block(SUPER.bmp_start + i, &mut b)?;
            let from = (i as usize) * LFS_BLOCK;
            let to = core::cmp::min(from + LFS_BLOCK, BLK_BITMAP.len());
            BLK_BITMAP[from..to].copy_from_slice(&b[..to - from]);
        }
        // inode bitmap
        dev_read_block(1, &mut b)?;
        INO_BITMAP.copy_from_slice(&b[..8]);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inode IO
// ---------------------------------------------------------------------------

fn inode_load(ino: u32) -> Result<LfsInode, &'static str> {
    if ino as usize >= LFS_INODES { return Err("bad inode"); }
    unsafe {
        let tbl = SUPER.inode_tbl_start;
        let blk = tbl + (ino as u32 * LFS_INODE_SIZE as u32) / LFS_BLOCK as u32;
        let off = (ino as usize * LFS_INODE_SIZE) % LFS_BLOCK;
        let mut b = [0u8; LFS_BLOCK];
        dev_read_block(blk, &mut b)?;
        let mut inode = LfsInode::new();
        core::ptr::copy_nonoverlapping(b.as_ptr().add(off), &mut inode as *mut LfsInode as *mut u8, LFS_INODE_SIZE);
        if inode.mode == 0 { return Err("inode not allocated"); }
        // integrity: CRC32 over the 128 bytes with checksum zeroed
        let mut raw = [0u8; LFS_INODE_SIZE];
        core::ptr::copy_nonoverlapping(b.as_ptr().add(off), raw.as_mut_ptr(), LFS_INODE_SIZE);
        let stored = inode.checksum;
        raw[LFS_INODE_CKSUM_OFF..LFS_INODE_CKSUM_OFF + 4].copy_from_slice(&[0u8; 4]);
        if crc32(&raw) != stored {
            serial::log("   lfs: inode ");
            serial::log_u(ino as u64);
            serial::log(" checksum FAIL\r\n");
            return Err("inode corrupt");
        }
        Ok(inode)
    }
}

fn inode_store(ino: u32, inode: &LfsInode) -> Result<(), &'static str> {
    if ino as usize >= LFS_INODES { return Err("bad inode"); }
    unsafe {
        let mut raw = [0u8; LFS_INODE_SIZE];
        core::ptr::copy_nonoverlapping(inode as *const LfsInode as *const u8, raw.as_mut_ptr(), LFS_INODE_SIZE);
        raw[LFS_INODE_CKSUM_OFF..LFS_INODE_CKSUM_OFF + 4].copy_from_slice(&[0u8; 4]);
        let csum = crc32(&raw);
        let mut inode2 = *inode;
        inode2.checksum = csum;
        let tbl = SUPER.inode_tbl_start;
        let blk = tbl + (ino as u32 * LFS_INODE_SIZE as u32) / LFS_BLOCK as u32;
        let off = (ino as usize * LFS_INODE_SIZE) % LFS_BLOCK;
        let mut b = [0u8; LFS_BLOCK];
        dev_read_block(blk, &mut b)?;
        core::ptr::copy_nonoverlapping(&inode2 as *const LfsInode as *const u8, b.as_mut_ptr().add(off), LFS_INODE_SIZE);
        dev_write_block(blk, &b)
    }
}

// ---------------------------------------------------------------------------
// Allocation
// ---------------------------------------------------------------------------

fn block_alloc() -> Result<u32, &'static str> {
    unsafe {
        for i in (DATA_START as usize)..LFS_BLOCKS as usize {
            if !bit_get(&BLK_BITMAP, i as u32) {
                bit_set(&mut BLK_BITMAP, i as u32);
                SUPER.free_blocks -= 1;
                persist_block_bitmap()?;
                return Ok(i as u32);
            }
        }
    }
    Err("disk full")
}

fn block_free(blk: u32) {
    unsafe {
        if blk as usize >= DATA_START as usize && blk < LFS_BLOCKS {
            if bit_get(&BLK_BITMAP, blk) {
                bit_clear(&mut BLK_BITMAP, blk);
                SUPER.free_blocks += 1;
                let _ = persist_block_bitmap();
            }
        }
    }
}

fn inode_alloc() -> Result<u32, &'static str> {
    unsafe {
        for i in 1..LFS_INODES as u32 {
            if !bit_get(&INO_BITMAP, i) {
                bit_set(&mut INO_BITMAP, i);
                SUPER.free_inodes -= 1;
                persist_inode_bitmap()?;
                return Ok(i);
            }
        }
    }
    Err("no free inode")
}

fn inode_free(ino: u32) {
    unsafe {
        if (ino as usize) < LFS_INODES && bit_get(&INO_BITMAP, ino) {
            bit_clear(&mut INO_BITMAP, ino);
            SUPER.free_inodes += 1;
            let _ = persist_inode_bitmap();
        }
    }
}

// ---------------------------------------------------------------------------
// Block mapping for file data
// ---------------------------------------------------------------------------

fn inode_block_ptr(inode: &LfsInode, idx: usize) -> u32 {
    if idx < LFS_DIRECT { inode.direct[idx] } else { 0 }
}

fn inode_set_block_ptr(inode: &mut LfsInode, idx: usize, blk: u32) -> Result<(), &'static str> {
    if idx < LFS_DIRECT {
        inode.direct[idx] = blk;
        Ok(())
    } else if idx < LFS_DIRECT + LFS_INDIRECT_PTRS {
        if inode.indirect == 0 {
            let nb = block_alloc()?;
            inode.indirect = nb;
            // zero the indirect block
            let zero: [u32; LFS_INDIRECT_PTRS] = [0u32; LFS_INDIRECT_PTRS];
            let mut b = [0u8; LFS_BLOCK];
            unsafe {
                core::ptr::copy_nonoverlapping(zero.as_ptr() as *const u8, b.as_mut_ptr(), LFS_BLOCK);
            }
            dev_write_block(nb, &b)?;
        }
        let ib = inode.indirect;
        let mut b = [0u8; LFS_BLOCK];
        dev_read_block(ib, &mut b)?;
        let off = (idx - LFS_DIRECT) * 4;
        b[off..off + 4].copy_from_slice(&blk.to_le_bytes());
        dev_write_block(ib, &b)?;
        Ok(())
    } else {
        Err("file too large")
    }
}

/// Read a file's data block number at byte offset `off`.
fn file_block_for(inode: &LfsInode, off: u64) -> Result<u32, &'static str> {
    let idx = (off / LFS_BLOCK as u64) as usize;
    if idx < LFS_DIRECT {
        let blk = inode.direct[idx];
        if blk == 0 { return Err("block not allocated"); }
        Ok(blk)
    } else if idx < LFS_DIRECT + LFS_INDIRECT_PTRS {
        if inode.indirect == 0 { return Err("block not allocated"); }
        let mut b = [0u8; LFS_BLOCK];
        dev_read_block(inode.indirect, &mut b)?;
        let off4 = (idx - LFS_DIRECT) * 4;
        let blk = u32::from_le_bytes(b[off4..off4 + 4].try_into().unwrap());
        if blk == 0 { return Err("block not allocated"); }
        Ok(blk)
    } else {
        Err("file too large")
    }
}

fn file_free_blocks(inode: &LfsInode) {
    for i in 0..LFS_DIRECT {
        if inode.direct[i] != 0 {
            block_free(inode.direct[i]);
        }
    }
    if inode.indirect != 0 {
        // read indirect block, free all referenced blocks, then the block itself
        let mut b = [0u8; LFS_BLOCK];
        if dev_read_block(inode.indirect, &mut b).is_ok() {
            for i in 0..LFS_INDIRECT_PTRS {
                let blk = u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
                if blk != 0 { block_free(blk); }
            }
        }
        block_free(inode.indirect);
    }
}

// ---------------------------------------------------------------------------
// File read / write
// ---------------------------------------------------------------------------

/// Read `len` bytes at `offset` from inode `ino` into `buf`. Returns bytes read.
pub fn lfs_read(ino: u32, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str> {
    let inode = inode_load(ino)?;
    if offset >= inode.size as u64 { return Ok(0); }
    let avail = (inode.size as u64 - offset).min(buf.len() as u64) as usize;
    let mut done = 0usize;
    while done < avail {
        let pos = offset + done as u64;
        let blk = file_block_for(&inode, pos)?;
        let in_block = (pos % LFS_BLOCK as u64) as usize;
        let n = core::cmp::min(LFS_BLOCK - in_block, avail - done);
        let mut b = [0u8; LFS_BLOCK];
        dev_read_block(blk, &mut b)?;
        buf[done..done + n].copy_from_slice(&b[in_block..in_block + n]);
        done += n;
    }
    Ok(done)
}

/// Write `data` at `offset`, allocating blocks as needed. Returns bytes written.
pub fn lfs_write(ino: u32, offset: u64, data: &[u8]) -> Result<usize, &'static str> {
    let mut inode = inode_load(ino)?;
    let end = offset + data.len() as u64;
    if end > ((LFS_DIRECT + LFS_INDIRECT_PTRS) * LFS_BLOCK) as u64 {
        return Err("file too large");
    }
    // ensure all blocks up to end are allocated
    let first_idx = (offset / LFS_BLOCK as u64) as usize;
    let last_idx = ((end - 1) / LFS_BLOCK as u64) as usize;
    for idx in first_idx..=last_idx {
        if inode_block_ptr(&inode, idx) == 0 {
            let nb = block_alloc()?;
            inode_set_block_ptr(&mut inode, idx, nb)?;
            inode.blocks += 1;
        }
    }
    // write data
    let mut done = 0usize;
    while done < data.len() {
        let pos = offset + done as u64;
        let idx = (pos / LFS_BLOCK as u64) as usize;
        let blk = if idx < LFS_DIRECT {
            inode.direct[idx]
        } else {
            // re-read indirect entry (may have changed above)
            let mut b = [0u8; LFS_BLOCK];
            dev_read_block(inode.indirect, &mut b)?;
            let off4 = (idx - LFS_DIRECT) * 4;
            u32::from_le_bytes(b[off4..off4 + 4].try_into().unwrap())
        };
        let in_block = (pos % LFS_BLOCK as u64) as usize;
        let n = core::cmp::min(LFS_BLOCK - in_block, data.len() - done);
        let mut b = [0u8; LFS_BLOCK];
        // partial block: preserve the rest
        if in_block != 0 || n != LFS_BLOCK {
            dev_read_block(blk, &mut b)?;
        }
        b[in_block..in_block + n].copy_from_slice(&data[done..done + n]);
        dev_write_block(blk, &b)?;
        done += n;
    }
    if end > inode.size as u64 {
        inode.size = end as u32;
    }
    inode.blocks = 0;
    for i in 0..LFS_DIRECT {
        if inode.direct[i] != 0 { inode.blocks += 1; }
    }
    if inode.indirect != 0 { inode.blocks += 1; }
    inode.mtime = 1; // coarse clock for now; real RTC comes with the clock service
    inode_store(ino, &inode)?;
    Ok(data.len())
}

/// Rewrite a file from scratch (truncate + write).
pub fn lfs_write_all(ino: u32, data: &[u8]) -> Result<usize, &'static str> {
    let inode = inode_load(ino)?;
    file_free_blocks(&inode);
    let mut inode = inode_load(ino)?;
    inode.size = 0;
    inode.blocks = 0;
    inode.direct = [0u32; LFS_DIRECT];
    inode.indirect = 0;
    inode_store(ino, &inode)?;
    lfs_write(ino, 0, data)
}

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

fn dir_block_count(inode: &LfsInode) -> usize {
    // directory data is stored like a file; each block holds DIRENTS_PER_BLOCK entries
    ((inode.size as usize) + LFS_BLOCK - 1) / LFS_BLOCK
}

fn dir_read_entries(parent: u32, out: &mut [LfsDirent]) -> Result<usize, &'static str> {
    let inode = inode_load(parent)?;
    if inode.mode & 0o170000 != 0o040000 { return Err("not a directory"); }
    let blocks = dir_block_count(&inode);
    let mut count = 0usize;
    let mut b = [0u8; LFS_BLOCK];
    for bi in 0..blocks {
        let blk = file_block_for(&inode, (bi * LFS_BLOCK) as u64)?;
        dev_read_block(blk, &mut b)?;
        for i in 0..DIRENTS_PER_BLOCK {
            let mut d = LfsDirent::empty();
            unsafe {
                core::ptr::copy_nonoverlapping(b.as_ptr().add(i * 64), &mut d as *mut LfsDirent as *mut u8, 64);
            }
            if d.ino != 0 {
                if count >= out.len() { return Ok(count); }
                out[count] = d;
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Find a directory entry by name. Returns the inode number.
pub fn lfs_lookup(parent: u32, name: &str) -> Result<u32, &'static str> {
    let mut entries = [LfsDirent::empty(); DIRENTS_PER_BLOCK * 8];
    let n = dir_read_entries(parent, &mut entries)?;
    for i in 0..n {
        // on-disk data is untrusted: clamp name_len before slicing
        let name_len = (entries[i].name_len as usize).min(LFS_NAME_MAX);
        let dname = core::str::from_utf8(&entries[i].name[..name_len]).unwrap_or("");
        if dname == name {
            return Ok(entries[i].ino);
        }
    }
    Err("not found")
}

fn dir_append(parent: u32, ino: u32, name: &str, ftype: u8) -> Result<(), &'static str> {
    let mut inode = inode_load(parent)?;
    if inode.mode & 0o170000 != 0o040000 { return Err("not a directory"); }
    if name.len() > LFS_NAME_MAX { return Err("name too long"); }
    let blocks = dir_block_count(&inode);
    let mut b = [0u8; LFS_BLOCK];
    for bi in 0..blocks {
        let blk = file_block_for(&inode, (bi * LFS_BLOCK) as u64)?;
        dev_read_block(blk, &mut b)?;
        for i in 0..DIRENTS_PER_BLOCK {
            let off = i * 64;
            let ino_at = u32::from_le_bytes(b[off..off + 4].try_into().unwrap());
            if ino_at == 0 {
                // free slot: fill it
                let mut d = LfsDirent::empty();
                d.ino = ino;
                d.name_len = name.len() as u16;
                d.ftype = ftype;
                let nb = name.as_bytes();
                d.name[..nb.len()].copy_from_slice(nb);
                unsafe {
                    core::ptr::copy_nonoverlapping(&d as *const LfsDirent as *const u8, b.as_mut_ptr().add(off), 64);
                }
                dev_write_block(blk, &b)?;
                return Ok(());
            }
        }
    }
    // need a new data block
    let nb = block_alloc()?;
    let new_blk_idx = blocks; // = inode.blocks if contiguous; use blocks count
    // ensure parent inode has a block ptr for this index
    if new_blk_idx < LFS_DIRECT {
        inode.direct[new_blk_idx] = nb;
    } else {
        inode_set_block_ptr(&mut inode, new_blk_idx, nb)?;
    }
    inode.blocks += 1;
    inode.size = ((new_blk_idx + 1) * LFS_BLOCK) as u32;
    let mut d = LfsDirent::empty();
    d.ino = ino;
    d.name_len = name.len() as u16;
    d.ftype = ftype;
    let nb2 = name.as_bytes();
    d.name[..nb2.len()].copy_from_slice(nb2);
    let mut b = [0u8; LFS_BLOCK];
    unsafe {
        core::ptr::copy_nonoverlapping(&d as *const LfsDirent as *const u8, b.as_mut_ptr(), 64);
    }
    dev_write_block(nb, &b)?;
    inode_store(parent, &inode)?;
    Ok(())
}

fn dir_remove(parent: u32, name: &str) -> Result<(), &'static str> {
    let inode = inode_load(parent)?;
    if inode.mode & 0o170000 != 0o040000 { return Err("not a directory"); }
    let blocks = dir_block_count(&inode);
    let mut b = [0u8; LFS_BLOCK];
    for bi in 0..blocks {
        let blk = file_block_for(&inode, (bi * LFS_BLOCK) as u64)?;
        dev_read_block(blk, &mut b)?;
        for i in 0..DIRENTS_PER_BLOCK {
            let off = i * 64;
            let mut d = LfsDirent::empty();
            unsafe {
                core::ptr::copy_nonoverlapping(b.as_ptr().add(off), &mut d as *mut LfsDirent as *mut u8, 64);
            }
            if d.ino != 0 {
                // on-disk data is untrusted: clamp name_len before slicing
                let name_len = (d.name_len as usize).min(LFS_NAME_MAX);
                let dname = core::str::from_utf8(&d.name[..name_len]).unwrap_or("");
                if dname == name {
                    // zero the slot
                    let zero = LfsDirent::empty();
                    unsafe {
                        core::ptr::copy_nonoverlapping(&zero as *const LfsDirent as *const u8, b.as_mut_ptr().add(off), 64);
                    }
                    dev_write_block(blk, &b)?;
                    return Ok(());
                }
            }
        }
    }
    Err("not found")
}

// ---------------------------------------------------------------------------
// Public filesystem API
// ---------------------------------------------------------------------------

/// Create a file or directory in `parent`.
pub fn lfs_create(parent: u32, name: &str, ftype: u8) -> Result<u32, &'static str> {
    if lfs_lookup(parent, name).is_ok() { return Err("exists"); }
    let ino = inode_alloc()?;
    let mut inode = LfsInode::new();
    inode.mode = if ftype == LFS_DTYPE_DIR { 0o40755 } else { 0o100644 };
    inode.nlink = 1;
    inode.ctime = 1;
    inode.mtime = 1;
    inode.parent = parent;
    inode_store(ino, &inode)?;
    dir_append(parent, ino, name, ftype)?;
    // bump parent nlink for directories (POSIX semantics)
    if ftype == LFS_DTYPE_DIR {
        if let Ok(mut p) = inode_load(parent) {
            p.nlink += 1;
            let _ = inode_store(parent, &p);
        }
    }
    Ok(ino)
}

/// Parent inode number of `ino` (u32::MAX = none).
pub fn lfs_parent(ino: u32) -> u32 {
    match inode_load(ino) {
        Ok(i) => i.parent,
        Err(_) => u32::MAX,
    }
}

/// Unlink a file (or empty dir) by name.
pub fn lfs_unlink(parent: u32, name: &str) -> Result<(), &'static str> {
    let ino = lfs_lookup(parent, name)?;
    let mut inode = inode_load(ino)?;
    if inode.mode & 0o170000 == 0o040000 && inode.size > 0 {
        return Err("directory not empty");
    }
    dir_remove(parent, name)?;
    inode.nlink -= 1;
    if inode.nlink == 0 {
        file_free_blocks(&inode);
        inode_free(ino);
    } else {
        inode_store(ino, &inode)?;
    }
    if inode.mode & 0o170000 == 0o040000 {
        if let Ok(mut p) = inode_load(parent) {
            p.nlink -= 1;
            let _ = inode_store(parent, &p);
        }
    }
    Ok(())
}

/// Stat an inode: (size, mode, uid).
pub fn lfs_stat(ino: u32) -> Result<(u32, u16, u16), &'static str> {
    let inode = inode_load(ino)?;
    Ok((inode.size, inode.uid, inode.mode))
}

/// List directory entries into `out`. Returns entry count.
pub fn lfs_readdir(parent: u32, out: &mut [LfsDirent]) -> Result<usize, &'static str> {
    dir_read_entries(parent, out)
}

/// Write a superblock block from cached state.
fn write_super() -> Result<(), &'static str> {
    unsafe {
        let mut b = [0u8; LFS_BLOCK];
        let sb = SUPER;
        let mut raw = [0u8; 256];
        core::ptr::copy_nonoverlapping(&sb as *const LfsSuper as *const u8, raw.as_mut_ptr(), 256);
        raw[LFS_SUPER_CKSUM_OFF..LFS_SUPER_CKSUM_OFF + 4].copy_from_slice(&[0u8; 4]); // zero checksum field
        let csum = crc32(&raw);
        b[..256].copy_from_slice(&raw);
        b[LFS_SUPER_CKSUM_OFF..LFS_SUPER_CKSUM_OFF + 4].copy_from_slice(&csum.to_le_bytes());
        dev_write_block(0, &b)
    }
}

fn read_super() -> Result<(), &'static str> {
    let mut b = [0u8; LFS_BLOCK];
    dev_read_block(0, &mut b)?;
    unsafe {
        let mut sb = LfsSuper::new();
        core::ptr::copy_nonoverlapping(b.as_ptr(), &mut sb as *mut LfsSuper as *mut u8, 256);
        let stored = sb.checksum;
        let mut raw = [0u8; 256];
        core::ptr::copy_nonoverlapping(b.as_ptr(), raw.as_mut_ptr(), 256);
        raw[LFS_SUPER_CKSUM_OFF..LFS_SUPER_CKSUM_OFF + 4].copy_from_slice(&[0u8; 4]);
        if crc32(&raw) != stored {
            return Err("superblock checksum mismatch");
        }
        SUPER = sb;
    }
    Ok(())
}

/// Format a fresh LFS volume on the mounted device.
pub fn lfs_format() -> Result<(), &'static str> {
    unsafe {
        let total = LFS_BLOCKS;
        let bmp_blocks = ((total + 7) / 8 + LFS_BLOCK as u32 - 1) / LFS_BLOCK as u32;
        let data_start = 2 + bmp_blocks + LFS_INODE_TBL_BLOCKS as u32;
        SUPER = LfsSuper {
            magic: LFS_MAGIC,
            version: LFS_VERSION,
            block_size: LFS_BLOCK as u32,
            lfs_blocks: total,
            free_blocks: total - data_start, // metadata blocks are used
            inode_count: LFS_INODES as u32,
            free_inodes: (LFS_INODES - 1) as u32, // inode 0 = root
            bmp_start: 2,
            inode_tbl_start: 2 + bmp_blocks,
            data_start,
            flags: 0,
            checksum: 0,
        };
        LFS_BLOCKS = total;
        BMP_BLOCKS = bmp_blocks;
        DATA_START = data_start;
        BLK_BITMAP = [0u8; 8192];
        INO_BITMAP = [0u8; 8];

        // mark metadata blocks used
        for blk in 0..data_start {
            bit_set(&mut BLK_BITMAP, blk);
        }
        // root inode
        bit_set(&mut INO_BITMAP, 0);

        persist_block_bitmap()?;
        persist_inode_bitmap()?;

        // write root inode
        let mut root = LfsInode::new();
        root.mode = 0o40755;
        root.uid = 0;
        root.nlink = 2;
        root.ctime = 1;
        root.mtime = 1;
        root.parent = 0;
        inode_store(0, &root)?;

        // zero the inode table's other slots and the whole bitmap region cleanly
        write_super()?;
        serial::log("   lfs: formatted (");
        serial::log_u(total as u64);
        serial::log(" blocks, ");
        serial::log_u(data_start as u64);
        serial::log(" metadata, ");
        serial::log_u(SUPER.free_blocks as u64);
        serial::log(" free)\r\n");
    }
    Ok(())
}

/// Mount an LFS volume by block device name. Returns the root inode number.
/// Formats automatically if the volume is blank or corrupt.
pub fn lfs_mount(dev_name: &str) -> Result<u32, &'static str> {
    let dev = blockdev::ly_blk_open(dev_name)?;
    let capacity = blockdev::ly_blk_capacity(dev);
    if capacity < 2 * 1024 * 1024 {
        return Err("lfs: device too small");
    }
    unsafe {
        MOUNTED_DEV = dev;
        LFS_BLOCKS = (capacity / LFS_BLOCK as u64) as u32;
    }

    let mut valid = false;
    if read_super().is_ok() {
        unsafe {
            if SUPER.magic == LFS_MAGIC
                && SUPER.version == LFS_VERSION
                && SUPER.block_size == LFS_BLOCK as u32
                && SUPER.lfs_blocks == LFS_BLOCKS
            {
                valid = true;
                BMP_BLOCKS = SUPER.bmp_start - 2;
                DATA_START = SUPER.data_start;
            }
        }
    }
    if !valid {
        serial::log("   lfs: no valid volume - formatting\r\n");
        lfs_format()?;
    }

    // Load the real bitmaps FIRST; fsck must recount from actual on-disk
    // allocation state, never from an unloaded (all-zero) in-memory bitmap.
    load_bitmaps()?;
    if unsafe { SUPER.flags & 1 != 0 } {
        serial::log("   lfs: volume dirty (unclean shutdown) - running fsck\r\n");
        lfs_fsck();
    }
    // mark dirty
    unsafe { SUPER.flags |= 1; }
    write_super()?;

    unsafe {
        serial::log("   lfs: mounted /dev/");
        serial::log(dev_name);
        serial::log(" (");
        serial::log_u(LFS_BLOCKS as u64);
        serial::log(" blocks, ");
        serial::log_u(SUPER.free_blocks as u64);
        serial::log(" free)\r\n");
    }
    Ok(0)
}

/// Write-through sync: flush superblock and clear the dirty flag.
pub fn lfs_sync() {
    unsafe {
        SUPER.flags &= !1;
    }
    let _ = write_super();
}

/// Lightweight fsck: recount free blocks/inodes and report inconsistencies.
pub fn lfs_fsck() {
    unsafe {
        let mut used = 0u32;
        for i in 0..LFS_BLOCKS {
            if bit_get(&BLK_BITMAP, i) { used += 1; }
        }
        let mut used_ino = 0u32;
        for i in 0..LFS_INODES as u32 {
            if bit_get(&INO_BITMAP, i) { used_ino += 1; }
        }
        let free = LFS_BLOCKS - used;
        let free_ino = LFS_INODES as u32 - used_ino;
        if free != SUPER.free_blocks || free_ino != SUPER.free_inodes {
            serial::log("   lfs: fsck found inconsistency - repairing counters\r\n");
            SUPER.free_blocks = free;
            SUPER.free_inodes = free_ino;
            let _ = write_super();
        } else {
            serial::log("   lfs: fsck clean\r\n");
        }
    }
}

/// Total and free capacity (bytes).
pub fn lfs_capacity() -> (u64, u64) {
    unsafe {
        let total = LFS_BLOCKS as u64 * LFS_BLOCK as u64;
        let free = SUPER.free_blocks as u64 * LFS_BLOCK as u64;
        (total, free)
    }
}
