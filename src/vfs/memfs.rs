//! LYSOR OS in-memory filesystem driver (`ly_memfs`).
//!
//! Pure RAM filesystem for early boot testing. No persistence.
//! Implements the LyInodeOps vtable expected by ly_vfs.

use crate::serial;
use crate::vfs::{LyInodeOps, LyFileType};

const MAX_FILES: usize = 8;
const MAX_NAME: usize = 32;
const FILE_DATA_SIZE: usize = 1024;

#[derive(Clone, Copy)]
struct MemNode {
    used: bool,
    name: [u8; MAX_NAME],
    name_len: usize,
    ftype: LyFileType,
    data: [u8; FILE_DATA_SIZE],
    data_len: usize,
    parent: i32,  // inode index of parent dir, -1 = root
}

impl MemNode {
    const fn empty() -> Self {
        MemNode {
            used: false,
            name: [0u8; MAX_NAME],
            name_len: 0,
            ftype: LyFileType::Regular,
            data: [0u8; FILE_DATA_SIZE],
            data_len: 0,
            parent: -1,
        }
    }
}

static mut NODES: [MemNode; MAX_FILES] = [MemNode::empty(); MAX_FILES];
static mut ROOT_INIT: bool = false;

// --- Inode ops: these are the vtable entries ---

extern "C" fn memfs_read(inode: usize, offset: u64, buf: *mut u8, len: usize) -> isize {
    unsafe {
        let idx = inode as usize;
        if idx >= MAX_FILES || !NODES[idx].used { return -9; }
        let node = &NODES[idx];
        let off = offset as usize;
        if off >= node.data_len { return 0; }
        let avail = node.data_len - off;
        let n = len.min(avail);
        core::ptr::copy_nonoverlapping(node.data.as_ptr().add(off), buf, n);
        n as isize
    }
}

extern "C" fn memfs_write(inode: usize, offset: u64, buf: *const u8, len: usize) -> isize {
    unsafe {
        let idx = inode as usize;
        if idx >= MAX_FILES || !NODES[idx].used { return -9; }
        let node = &mut NODES[idx];
        let off = offset as usize;
        if off + len > FILE_DATA_SIZE { return -28; } // EFBIG
        core::ptr::copy_nonoverlapping(buf, node.data.as_mut_ptr().add(off), len);
        let end = off + len;
        if end > node.data_len { node.data_len = end; }
        len as isize
    }
}

extern "C" fn memfs_open(_inode: usize) -> i32 { 0 }
extern "C" fn memfs_close(_inode: usize) {}

static MEMFS_OPS: LyInodeOps = LyInodeOps {
    read: memfs_read,
    write: memfs_write,
    open: memfs_open,
    close: memfs_close,
};

/// Initialize memfs: create root directory.
pub fn ly_memfs_init() {
    unsafe {
        if !ROOT_INIT {
            NODES[0].used = true;
            NODES[0].ftype = LyFileType::Directory;
            NODES[0].name[0] = b'/';
            NODES[0].name_len = 1;
            NODES[0].parent = -1;
            ROOT_INIT = true;
        }
    }
    serial::log("   memfs: mounted at / (");
    serial::log_u(MAX_FILES as u64);
    serial::log(" entries, 1KB/file)\r\n");
}

/// Look up or create a file path. Returns inode index.
pub fn ly_memfs_open(path: &str) -> Result<usize, &'static str> {
    unsafe {
        // Strip leading /
        let p = path.strip_prefix('/').unwrap_or(path);
        // Search existing
        for i in 1..MAX_FILES {
            if NODES[i].used && NODES[i].name_len == p.len() {
                let mut match_ok = true;
                for j in 0..p.len() {
                    if NODES[i].name[j] != p.as_bytes()[j] { match_ok = false; break; }
                }
                if match_ok { return Ok(i); }
            }
        }
        // Create new
        for i in 1..MAX_FILES {
            if !NODES[i].used {
                let bytes = p.as_bytes();
                let n = bytes.len().min(MAX_NAME);
                NODES[i].name[..n].copy_from_slice(&bytes[..n]);
                NODES[i].name_len = n;
                NODES[i].ftype = LyFileType::Regular;
                NODES[i].data_len = 0;
                NODES[i].parent = 0;
                return Ok(i);
            }
        }
        Err("memfs full")
    }
}

/// Write a string to a file (convenience API for testing).
pub fn ly_memfs_put(path: &str, content: &[u8]) -> Result<(), &'static str> {
    let idx = ly_memfs_open(path)?;
    unsafe {
        if content.len() > FILE_DATA_SIZE { return Err("too large"); }
        let node = &mut NODES[idx];
        node.data[..content.len()].copy_from_slice(content);
        node.data_len = content.len();
    }
    Ok(())
}

/// Read a file to a buffer.
pub fn ly_memfs_get(path: &str, buf: &mut [u8]) -> Result<usize, &'static str> {
    let idx = ly_memfs_open(path)?;
    unsafe {
        let node = &NODES[idx];
        let n = node.data_len.min(buf.len());
        buf[..n].copy_from_slice(&node.data[..n]);
        Ok(n)
    }
}

