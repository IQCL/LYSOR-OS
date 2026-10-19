//! LYSOR OS System Call Layer (`ly_syscall`).
//!
//! Unified syscall entry point with:
//! - Argument validation
//! - Capability/permission interception (real per-thread caps from the TCB)
//! - Dispatch to VFS, IPC (channel A), scheduler (cooperative yield)
//!
//! Entered via INT 0x80: the ISR dispatcher (`idt::ly_isr_handler`) passes the
//! full TrapFrame as a **raw mutable pointer** (writing the result into
//! `frame.rax` must be a real side effect; a shared-reference cast would let
//! the optimizer elide the store under the immutable aliasing rule). The
//! normal iretq path returns the patched rax to the caller. A syscall may also
//! return a non-zero rsp to force a cooperative context switch
//! (SYS_SCHED_YIELD).

use crate::idt::TrapFrame;
use crate::ipc::channel_a::{ly_chan_recv, ly_chan_send, LyMsg};
use crate::sched;
use crate::serial;
use crate::vfs;

// Syscall numbers
pub const SYS_OPEN: usize = 1;
pub const SYS_READ: usize = 2;
pub const SYS_WRITE: usize = 3;
pub const SYS_CLOSE: usize = 4;
pub const SYS_EXIT: usize = 5;
pub const SYS_IPC_SEND: usize = 10;
pub const SYS_IPC_RECV: usize = 11;
pub const SYS_SCHED_YIELD: usize = 20;

/// Capability bits checked before syscall dispatch (match sched::capability).
const CAP_FILE_IO: u8 = 4;
const CAP_IPC: u8 = 7;
const CAP_SCHED: u8 = 8;

/// Real capability check: the calling thread's CapSet from the TCB.
fn ly_cap_has(cap: u8, tid: u32) -> bool {
    sched::ly_sched_thread_caps(tid).has(cap)
}

/// Validate a user-supplied pointer: non-null and inside the 1 GiB identity
/// map (user code runs in low memory in this kernel).
fn user_ptr(p: usize) -> bool {
    p != 0 && p < 0x4000_0000
}

/// Unified syscall entry. Called from the ISR dispatcher with the TrapFrame.
/// Returns the rsp to switch to (0 = no switch).
pub extern "C" fn ly_syscall_entry(frame: *mut TrapFrame) -> u64 {
    let nr = unsafe { (*frame).rax as usize };
    let a0 = unsafe { (*frame).rdi as usize };
    let a1 = unsafe { (*frame).rsi as usize };
    let a2 = unsafe { (*frame).rdx as usize };
    let tid = sched::ly_sched_current_tid();
    let result: isize;

    match nr {
        SYS_OPEN => {
            if !ly_cap_has(CAP_FILE_IO, tid) { result = -13; }
            else if !user_ptr(a0) { result = -14; }
            else {
                let path = unsafe { core::ffi::CStr::from_ptr(a0 as *const i8) };
                result = vfs::ly_vfs_open(path.to_str().unwrap_or(""), a1 as u32, tid);
            }
        }
        SYS_READ => {
            if !ly_cap_has(CAP_FILE_IO, tid) { result = -13; }
            else if !user_ptr(a1) { result = -14; }
            else {
                result = vfs::ly_vfs_read(a0 as isize, a1 as *mut u8, a2) as isize;
            }
        }
        SYS_WRITE => {
            if !ly_cap_has(CAP_FILE_IO, tid) { result = -13; }
            else if !user_ptr(a1) { result = -14; }
            else {
                result = vfs::ly_vfs_write(a0 as isize, a1 as *const u8, a2) as isize;
            }
        }
        SYS_CLOSE => {
            if !ly_cap_has(CAP_FILE_IO, tid) { result = -13; }
            else {
                result = vfs::ly_vfs_close(a0 as isize) as isize;
            }
        }

        SYS_EXIT => {
            serial::log("   syscall: tid ");
            serial::log_u(tid as u64);
            serial::log(" exit(0x");
            serial::log_hex(a0 as u64);
            serial::log(")\r\n");
            sched::ly_sched_exit();
        }

        SYS_IPC_SEND => {
            if !ly_cap_has(CAP_IPC, tid) { result = -13; }
            else if !user_ptr(a1) { result = -14; }
            else {
                let msg = unsafe { &*(a1 as *const LyMsg) };
                match ly_chan_send(a0, msg) {
                    Ok(()) => result = 0,
                    Err(e) => {
                        serial::log("   syscall: ipc_send chan=");
                        serial::log_u(a0 as u64);
                        serial::log(" err: ");
                        serial::log(e);
                        serial::log("\r\n");
                        result = -5;
                    }
                }
            }
        }
        SYS_IPC_RECV => {
            if !ly_cap_has(CAP_IPC, tid) { result = -13; }
            else if !user_ptr(a1) { result = -14; }
            else {
                match ly_chan_recv(a0, tid) {
                    Ok(Some(m)) => {
                        // Copy out: header + payload (fixed MSG_SIZE buffer).
                        let buf = a1 as *mut LyMsg;
                        unsafe { core::ptr::write(buf, m) };
                        result = m.len as isize;
                    }
                    Ok(None) => result = 0, // no message: return 0
                    Err(e) => {
                        serial::log("   syscall: ipc_recv chan=");
                        serial::log_u(a0 as u64);
                        serial::log(" err: ");
                        serial::log(e);
                        serial::log("\r\n");
                        result = -5;
                    }
                }
            }
        }

        SYS_SCHED_YIELD => {
            if !ly_cap_has(CAP_SCHED, tid) { result = -13; }
            else {
                // Real cooperative yield: save this TrapFrame as the thread's
                // resume point, mark Ready, switch to the next Ready thread.
                // The caller's rax is patched so the resumed iretq returns 0.
                let next = unsafe { sched::ly_sched_yield_switch(frame as u64) };
                if next != 0 {
                    // volatile: the ISR stub pops this frame after we return.
                    unsafe { core::ptr::write_volatile(&mut (*frame).rax, 0) };
                    return next;
                }
                result = 0;
            }
        }

        _ => {
            serial::log("   syscall: unknown nr=");
            serial::log_u(nr as u64);
            serial::log("\r\n");
            result = -38; // ENOSYS
        }
    }

    // volatile: the ISR stub pops this frame (incl. rax) after we return;
    // a plain store through the raw pointer may be elided as dead memory.
    unsafe { core::ptr::write_volatile(&mut (*frame).rax, result as u64) };
    0
}

/// Initialize syscall layer.
pub fn ly_syscall_init() {
    serial::log("   syscall: layer ready (INT 0x80, 6 handlers, real caps)\r\n");
}
