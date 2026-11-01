//! LYSOR OS Virtual File System (`ly_vfs`).
//!
//! Real implementation: a mount point on a block-device-backed LFS volume,
//! full path resolution (including `.` and `..`), open/read/write/close on
//! file descriptors with position tracking, O_CREAT/O_APPEND semantics, and
//! permission checks wired to the real per-thread capability model.

pub mod memfs;

use crate::serial;
use crate::fs::lfs;
use crate::sched::capability;

/// File open flags.
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0x40;
pub const O_APPEND: u32 = 0x400;
pub const O_DIRECTORY: u32 = 0x10000;

/// File type.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum LyFileType {
    Regular = 0,
    Directory = 1,
    Device = 2,
    Symlink = 3,
}

/// Inode operations: backend drivers fill this in.
#[repr(C)]
pub struct LyInodeOps {
    pub read: extern "C" fn(inode: usize, offset: u64, buf: *mut u8, len: usize) -> isize,
    pub write: extern "C" fn(inode: usize, offset: u64, buf: *const u8, len: usize) -> isize,
    pub open: extern "C" fn(inode: usize) -> i32,
    pub close: extern "C" fn(inode: usize),
}

// --- LFS backend ops (inode number travels in the `inode` arg) ---

extern "C" fn lfs_backend_read(inode: usize, offset: u64, buf: *mut u8, len: usize) -> isize {
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    match lfs::lfs_read(inode as u32, offset, slice) {
        Ok(n) => n as isize,
        Err(_) => -5, // EIO
    }
}

extern "C" fn lfs_backend_write(inode: usize, offset: u64, buf: *const u8, len: usize) -> isize {
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };
    match lfs::lfs_write(inode as u32, offset, slice) {
        Ok(n) => n as isize,
        Err(_) => -5,
    }
}

extern "C" fn lfs_backend_open(_inode: usize) -> i32 {
    0
}

extern "C" fn lfs_backend_close(_inode: usize) {}

static LFS_OPS: LyInodeOps = LyInodeOps {
    read: lfs_backend_read,
    write: lfs_backend_write,
    open: lfs_backend_open,
    close: lfs_backend_close,
};

/// Directory listing entry (for apps / file manager).
#[derive(Clone, Copy)]
pub struct LyDirEntry {
    pub name: [u8; 56],
    pub name_len: u16,
    pub ftype: u8,
    pub size: u32,
}

// --- File descriptor table ---

const MAX_FD: usize = 16;

#[derive(Clone, Copy)]
struct LyFile {
    ino: u32,
    pos: u64,
    flags: u32,
    used: bool,
}

impl LyFile {
    const fn empty() -> Self {
        LyFile { ino: 0, pos: 0, flags: 0, used: false }
    }
}

static mut FDTABLE: [LyFile; MAX_FD] = [LyFile::empty(); MAX_FD];
static mut ROOT_MOUNTED: bool = false;

/// Mount the root filesystem from a block device ("sda" / "ram0").
/// Formats automatically when the volume is blank.
pub fn ly_vfs_mount_root(dev_name: &str) -> Result<(), &'static str> {
    let root = lfs::lfs_mount(dev_name)?;
    unsafe { ROOT_MOUNTED = true; }
    serial::log("   vfs: root mounted from /dev/");
    serial::log(dev_name);
    serial::log(" (inode ");
    serial::log_u(root as u64);
    serial::log(")\r\n");
    Ok(())
}

pub fn ly_vfs_is_mounted() -> bool {
    unsafe { ROOT_MOUNTED }
}

// --- Permission checking (real, capability-backed) ---

/// Capability bits used by the VFS.
pub const CAP_FILE_IO: u8 = 4;
pub const CAP_FILE_WRITE: u8 = 5;
pub const CAP_DIR_OP: u8 = 6;

pub fn ly_vfs_perm_check(_path: &str, tid: u32, write: bool) -> bool {
    let caps = crate::sched::ly_sched_thread_caps(tid);
    if !capability::ly_cap_check(caps, CAP_FILE_IO) {
        return false;
    }
    if write && !capability::ly_cap_check(caps, CAP_FILE_WRITE) {
        return false;
    }
    true
}

// --- Path resolution --------------------------------------------------------

/// Split "/a/b/c" into ("/a/b", "c"). Returns (dir_path, name).
fn split_path(path: &str) -> (&str, &str) {
    let trimmed = path.trim_start_matches('/');
    match trimmed.rfind('/') {
        Some(i) => (&trimmed[..i], &trimmed[i + 1..]),
        None => ("", trimmed),
    }
}

/// Resolve a directory path to an inode number (0 = root).
fn resolve_dir(dir_path: &str) -> Result<u32, &'static str> {
    let mut cur = 0u32; // root
    if dir_path.is_empty() {
        return Ok(cur);
    }
    for comp in dir_path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            let p = lfs::lfs_parent(cur);
            if p == u32::MAX {
                return Err("no parent");
            }
            cur = p;
            continue;
        }
        cur = lfs::lfs_lookup(cur, comp)?;
    }
    // verify final is a directory
    let (_, _, mode) = match lfs::lfs_stat(cur) {
        Ok(s) => s,
        Err(e) => return Err(e),
    };
    if mode & 0o170000 != 0o040000 {
        return Err("not a directory");
    }
    Ok(cur)
}

/// Resolve a full path to an inode number.
fn resolve_path(path: &str) -> Result<u32, &'static str> {
    let (dir, name) = split_path(path);
    let parent = resolve_dir(dir)?;
    if name.is_empty() {
        return Ok(parent); // path ends at a directory
    }
    let ino = lfs::lfs_lookup(parent, name)?;
    let (_, _, mode) = lfs::lfs_stat(ino)?;
    if mode & 0o170000 == 0o040000 {
        // directory: keep it
    }
    Ok(ino)
}

/// Open a file. Returns fd or negative error.
pub fn ly_vfs_open(path: &str, flags: u32, tid: u32) -> isize {
    unsafe {
        if !ROOT_MOUNTED {
            return -5;
        }
    }
    if !ly_vfs_perm_check(path, tid, (flags & (O_WRONLY | O_RDWR)) != 0) {
        return -13; // EACCES
    }

    let (dir_path, name) = split_path(path);
    let parent = match resolve_dir(dir_path) {
        Ok(p) => p,
        Err(e) => {
            serial::log("   vfs: open(");
            serial::log(path);
            serial::log(") path error: ");
            serial::log(e);
            serial::log("\r\n");
            return -2; // ENOENT
        }
    };

    let ino = if name.is_empty() {
        parent
    } else {
        match lfs::lfs_lookup(parent, name) {
            Ok(i) => i,
            Err(_) => {
                if flags & O_CREAT != 0 {
                    let ftype = if flags & O_DIRECTORY != 0 { lfs::LFS_DTYPE_DIR } else { lfs::LFS_DTYPE_FILE };
                    match lfs::lfs_create(parent, name, ftype) {
                        Ok(i) => i,
                        Err(e) => {
                            serial::log("   vfs: create failed: ");
                            serial::log(e);
                            serial::log("\r\n");
                            return -13;
                        }
                    }
                } else {
                    return -2; // ENOENT
                }
            }
        }
    };

    unsafe {
        for i in 0..MAX_FD {
            if !FDTABLE[i].used {
                FDTABLE[i].used = true;
                FDTABLE[i].ino = ino;
                FDTABLE[i].pos = 0;
                FDTABLE[i].flags = flags;
                // O_APPEND: start at end
                if flags & O_APPEND != 0 {
                    if let Ok((size, _, _)) = lfs::lfs_stat(ino) {
                        FDTABLE[i].pos = size as u64;
                    }
                }
                return i as isize;
            }
        }
    }
    -24 // EMFILE
}

/// Read from an open fd.
pub fn ly_vfs_read(fd: isize, buf: *mut u8, len: usize) -> isize {
    if fd < 0 || fd as usize >= MAX_FD { return -9; }
    unsafe {
        let f = &mut FDTABLE[fd as usize];
        if !f.used { return -9; }
        let n = (LFS_OPS.read)(f.ino as usize, f.pos, buf, len);
        if n > 0 {
            f.pos += n as u64;
        }
        n
    }
}

/// Write to an open fd.
pub fn ly_vfs_write(fd: isize, buf: *const u8, len: usize) -> isize {
    if fd < 0 || fd as usize >= MAX_FD { return -9; }
    unsafe {
        let f = &mut FDTABLE[fd as usize];
        if !f.used { return -9; }
        // append semantics handled at open; honor O_WRONLY/O_RDWR
        if f.flags & (O_WRONLY | O_RDWR) == 0 {
            return -9; // EBADF
        }
        let n = (LFS_OPS.write)(f.ino as usize, f.pos, buf, len);
        if n > 0 {
            f.pos += n as u64;
        }
        n
    }
}

/// Close an fd.
pub fn ly_vfs_close(fd: isize) -> i32 {
    if fd < 0 || fd as usize >= MAX_FD { return -9; }
    unsafe {
        let f = &mut FDTABLE[fd as usize];
        if !f.used { return -9; }
        (LFS_OPS.close)(f.ino as usize);
        f.used = false;
    }
    0
}

/// Seek: returns new position or -1.
pub fn ly_vfs_seek(fd: isize, offset: i64, whence: u8) -> isize {
    if fd < 0 || fd as usize >= MAX_FD { return -1; }
    unsafe {
        let f = &mut FDTABLE[fd as usize];
        if !f.used { return -1; }
        let base = match whence {
            0 => 0i64,                                       // SEEK_SET
            1 => f.pos as i64,                               // SEEK_CUR
            2 => match lfs::lfs_stat(f.ino) {
                Ok((size, _, _)) => size as i64,
                Err(_) => return -1,
            },
            _ => return -1,
        };
        let new_pos = base + offset;
        if new_pos < 0 { return -1; }
        f.pos = new_pos as u64;
        f.pos as isize
    }
}

/// List a directory through the VFS. Fills `out`; returns entry count.
pub fn ly_vfs_list(path: &str, out: &mut [LyDirEntry]) -> Result<usize, &'static str> {
    let ino = resolve_path(path)?;
    let mut dirents = [lfs::LfsDirent::empty(); 256];
    let n = lfs::lfs_readdir(ino, &mut dirents)?;
    let mut count = 0usize;
    for i in 0..n {
        if count >= out.len() { break; }
        // on-disk data is untrusted: clamp before slicing (no panic on bad dirent)
        let name_len = (dirents[i].name_len as usize).min(56);
        let mut e = LyDirEntry {
            name: [0u8; 56],
            name_len: name_len as u16,
            ftype: dirents[i].ftype,
            size: 0,
        };
        e.name[..name_len].copy_from_slice(&dirents[i].name[..name_len]);
        if let Ok((size, _, _)) = lfs::lfs_stat(dirents[i].ino) {
            e.size = size;
        }
        out[count] = e;
        count += 1;
    }
    Ok(count)
}

/// Stat a file through the VFS: (size, mode). Errors if missing.
pub fn ly_vfs_stat(path: &str) -> Result<(u32, u16), &'static str> {
    let ino = resolve_path(path)?;
    let (size, _, mode) = lfs::lfs_stat(ino)?;
    Ok((size, mode))
}

/// Delete a file through the VFS.
pub fn ly_vfs_unlink(path: &str) -> Result<(), &'static str> {
    let (dir, name) = split_path(path);
    if name.is_empty() { return Err("bad path"); }
    let parent = resolve_dir(dir)?;
    lfs::lfs_unlink(parent, name)
}

/// Initialize VFS.
pub fn ly_vfs_init() {
    serial::log("   vfs: layer ready (fd table ");
    serial::log_u(MAX_FD as u64);
    serial::log(", LFS backend, real perm hook)\r\n");
}
