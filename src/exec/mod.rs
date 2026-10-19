//! LYSOR OS native executable loader (`ly_exec_lyx`).
//!
//! LYX format: LYSOR OS native binary format (like .exe / ELF).
//! Layout:
//!   [ Header 64 ][ .text ][ .rodata ][ .data ][ algo 1 ][ sig 64 ]
//!
//! Header is 64 bytes, magic = 0x4C595831 ("LYX1"). The 64-byte signature
//! and its 1-byte algorithm tag live at the END of the file, so the signed
//! content is a single contiguous region (everything before the tag).
//! Signature verification is real and dual-track (SM2 国密优先 / Ed25519),
//! delegated to the optional crypto kcomponent via `sig` — an unsigned or
//! badly-signed image is rejected, never silently loaded.

use crate::serial;
use crate::sig;
use crate::memory::physical;
use crate::sched;
use crate::sig::{ALGO_SM2, ALGO_ED25519};

/// LYX magic: "LYX1"
pub const LYX_MAGIC: u32 = 0x4C595831;
pub const LYX_HEADER_SIZE: usize = 64;
/// 文件尾签名区 = 1 字节算法标识 + 64 字节签名。
pub const LYX_TRAILER_SIZE: usize = 1 + 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LyxHeader {
    pub magic: u32,       // LYX_MAGIC
    pub version: u32,      // format version
    pub entry: u64,        // entry point vaddr
    pub text_off: u64,     // .text offset in file
    pub text_size: u64,    // .text size
    pub rodata_off: u64,
    pub rodata_size: u64,
    pub data_off: u64,
    pub data_size: u64,
    pub stack_size: u64,   // suggested stack
    pub flags: u32,        // permission flags
    pub reserved: [u8; 16],// reserved; real signature is at end of file
}

impl LyxHeader {
    pub const fn empty() -> Self {
        LyxHeader {
            magic: 0, version: 0, entry: 0,
            text_off: 0, text_size: 0,
            rodata_off: 0, rodata_size: 0,
            data_off: 0, data_size: 0,
            stack_size: 0, flags: 0,
            reserved: [0u8; 16],
        }
    }
}

/// Verify LYX header magic, sanity, and the trailing real signature.
fn lyx_validate(image: *const u8, len: usize) -> Result<(), &'static str> {
    let h = unsafe { &*(image as *const LyxHeader) };
    if h.magic != LYX_MAGIC {
        return Err("bad magic");
    }
    if h.version > 1 {
        return Err("version too new");
    }
    if h.text_size > 1 << 20 {
        return Err("text too large"); // 1MB
    }
    if h.data_size > 1 << 20 {
        return Err("data too large");
    }
    if len <= LYX_HEADER_SIZE + LYX_TRAILER_SIZE {
        return Err("no signature trailer");
    }

    // 签名区在文件尾部：algo 在 len-65，sig64 在 len-64..len。
    let algo = unsafe { *image.add(len - LYX_TRAILER_SIZE) };
    if algo != ALGO_SM2 && algo != ALGO_ED25519 {
        serial::log("   lyx: unknown signature algo ");
        serial::log_u(algo as u64);
        serial::log(", rejected\r\n");
        return Err("bad sig algo");
    }
    let mut sig64 = [0u8; 64];
    unsafe {
        core::ptr::copy_nonoverlapping(
            image.add(len - 64),
            sig64.as_mut_ptr(),
            64,
        );
    }
    if !sig::ly_verify_lyx(image, len - LYX_TRAILER_SIZE, algo, &sig64) {
        return Err("signature invalid");
    }
    Ok(())
}

/// Load a LYX binary from a memory image and start it as a new thread.
/// `image` is a pointer to the raw file bytes, `len` its length.
pub fn ly_exec_lyx(image: *const u8, len: usize) -> Result<u32, &'static str> {
    if len < LYX_HEADER_SIZE {
        return Err("image too small");
    }

    unsafe {
        let h = &*(image as *const LyxHeader);
        serial::log("   lyx: header magic=0x");
        serial::log_hex(h.magic as u64);
        serial::log(" entry=0x");
        serial::log_hex(h.entry);
        serial::log(" text=");
        serial::log_u(h.text_size);
        serial::log("\r\n");

        lyx_validate(image, len)?;

        // Allocate stack for new thread
        let stack_frame = physical::alloc_frame().ok_or("no stack frame")?;

        // Create a new kernel thread starting at the LYX entry point.
        // In a real implementation we'd map segments into the new address
        // space; for now we run the entry directly in kernel context.
        let entry_fn: extern "C" fn() = core::mem::transmute(h.entry);

        let tid = sched::ly_sched_create("lyx_app", entry_fn).ok_or("thread full")?;

        serial::log("   lyx: started tid=");
        serial::log_u(tid as u64);
        serial::log(" stack=0x");
        serial::log_hex(stack_frame);
        serial::log("\r\n");

        Ok(tid)
    }
}

/// Initialize lyx loader.
pub fn ly_exec_init() {
    serial::log("   lyx: loader ready (v1, dual-track sig enforced)\r\n");
}
