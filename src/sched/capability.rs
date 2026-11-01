//! Capability-based security model (`ly_cap`).
//!
//! Each thread owns a capability bitset that gates privileged operations.
//! Syscalls and the VFS permission hook check the caller's set through
//! `ly_cap_check`. Capabilities are granted at thread creation and can be
//! revoked at runtime (used by the sandbox runtime).

use crate::serial;

/// Capability bitset (32 capabilities, extensible).
#[derive(Clone, Copy)]
pub struct CapSet {
    bits: u32,
}

impl CapSet {
    pub const fn empty() -> Self {
        CapSet { bits: 0 }
    }
    pub const fn all() -> Self {
        CapSet { bits: 0xFFFF_FFFF }
    }
    pub const fn kernel_default() -> Self {
        CapSet { bits: 0xFFFF_FFFF }
    }
    pub const fn user_default() -> Self {
        // file IO + IPC + basic sched; no raw device/IRQ/component powers
        CapSet {
            bits: (1 << CAP_SYS_TIME)
                | (1 << CAP_FILE_IO)
                | (1 << CAP_FILE_WRITE)
                | (1 << CAP_DIR_OP)
                | (1 << CAP_IPC)
                | (1 << CAP_SCHED),
        }
    }
    pub fn has(&self, cap: u8) -> bool {
        if cap >= 32 { return false; }
        self.bits & (1 << cap) != 0
    }
    pub fn grant(&mut self, cap: u8) {
        if cap < 32 { self.bits |= 1 << cap; }
    }
    pub fn revoke(&mut self, cap: u8) {
        if cap < 32 { self.bits &= !(1 << cap); }
    }
    pub fn bits(&self) -> u32 {
        self.bits
    }
}

/// Well-known capability indices.
pub const CAP_SYS_TIME: u8 = 0;
pub const CAP_SYS_MEM: u8 = 1;
pub const CAP_SYS_COMPONENT: u8 = 2;
pub const CAP_SYS_IRQ: u8 = 3;
pub const CAP_FILE_IO: u8 = 4;
pub const CAP_FILE_WRITE: u8 = 5;
pub const CAP_DIR_OP: u8 = 6;
pub const CAP_IPC: u8 = 7;
pub const CAP_SCHED: u8 = 8;
pub const CAP_NET_ADMIN: u8 = 9;
pub const CAP_SYS_POWER: u8 = 10;

/// Check whether a capability is held.
pub fn ly_cap_check(owner_caps: CapSet, cap: u8) -> bool {
    owner_caps.has(cap)
}

/// Initialize the capability subsystem.
pub fn ly_cap_init() {
    serial::log("   cap: capability model ready (32-bit sets)\r\n");
}
