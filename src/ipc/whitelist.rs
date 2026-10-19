//! Whitelist verification framework for trusted IPC (Channel B).
//!
//! Provides:
//! - Entry whitelist: which tids may attach to a shared-memory channel
//! - Signature verification stub: placeholder for future ed25519 checks
//! - Audit log ring buffer: records access attempts for later review

use crate::serial;

const MAX_WHITELIST: usize = 16;
const MAX_AUDIT: usize = 64;
const MAX_CHAN_NAME: usize = 32;

/// Whitelist entry: a channel name and the tids allowed to use it.
pub struct WhitelistEntry {
    chan_name: [u8; MAX_CHAN_NAME],
    chan_len: usize,
    allowed: [u32; MAX_WHITELIST],
    allowed_count: usize,
}

impl WhitelistEntry {
    const fn new() -> Self {
        WhitelistEntry {
            chan_name: [0u8; MAX_CHAN_NAME],
            chan_len: 0,
            allowed: [0u32; MAX_WHITELIST],
            allowed_count: 0,
        }
    }
}

/// Audit record: one access attempt.
#[derive(Clone, Copy)]
struct AuditRecord {
    tid: u32,
    chan_idx: u8,
    granted: bool,
    tick: u64,
}

impl AuditRecord {
    const fn new() -> Self {
        AuditRecord { tid: 0, chan_idx: 0, granted: false, tick: 0 }
    }
}

static mut WHITELIST: [WhitelistEntry; 4] = [
    WhitelistEntry::new(),
    WhitelistEntry::new(),
    WhitelistEntry::new(),
    WhitelistEntry::new(),
];
static mut AUDIT: [AuditRecord; MAX_AUDIT] = [AuditRecord::new(); MAX_AUDIT];
static mut AUDIT_HEAD: usize = 0;
static mut AUDIT_COUNT: usize = 0;

/// Add a tid to a channel's whitelist. Returns Ok on success.
pub fn ly_whitelist_grant(chan_idx: usize, chan_name: &str, tid: u32) -> Result<(), &'static str> {
    unsafe {
        if chan_idx >= 4 { return Err("bad chan idx"); }
        let e = &mut WHITELIST[chan_idx];
        // Set channel name if first time
        if e.chan_len == 0 {
            let bytes = chan_name.as_bytes();
            let n = bytes.len().min(MAX_CHAN_NAME);
            e.chan_name[..n].copy_from_slice(&bytes[..n]);
            e.chan_len = n;
        }
        // Check duplicate
        for i in 0..e.allowed_count {
            if e.allowed[i] == tid { return Ok(()); }
        }
        if e.allowed_count >= MAX_WHITELIST { return Err("whitelist full"); }
        e.allowed[e.allowed_count] = tid;
        e.allowed_count += 1;
        Ok(())
    }
}

/// Check if tid is allowed on this channel. Records audit trail.
pub fn ly_whitelist_check(chan_idx: usize, tid: u32, tick: u64) -> bool {
    unsafe {
        let granted = if chan_idx < 4 {
            let e = &WHITELIST[chan_idx];
            let mut ok = false;
            for i in 0..e.allowed_count {
                if e.allowed[i] == tid { ok = true; break; }
            }
            ok
        } else {
            false
        };
        // Log audit
        let slot = AUDIT_HEAD % MAX_AUDIT;
        AUDIT[slot] = AuditRecord { tid, chan_idx: chan_idx as u8, granted, tick };
        AUDIT_HEAD = (AUDIT_HEAD + 1) % MAX_AUDIT;
        if AUDIT_COUNT < MAX_AUDIT { AUDIT_COUNT += 1; }
        granted
    }
}

/// Signature verification stub. Always returns true (real ed25519 later).
pub fn ly_verify_signature(_data: &[u8], _sig: &[u8; 64], _pubkey: &[u8; 32]) -> bool {
    // TODO: ed25519 verify
    true
}

/// Dump audit log to serial.
pub fn ly_audit_dump() {
    unsafe {
        serial::log("   audit: ");
        serial::log_u(AUDIT_COUNT as u64);
        serial::log(" records\r\n");
        for i in 0..AUDIT_COUNT.min(10) {
            let r = AUDIT[i];
            serial::log("     tid=");
            serial::log_u(r.tid as u64);
            serial::log(" chan=");
            serial::log_u(r.chan_idx as u64);
            serial::log(if r.granted { " OK" } else { " DENIED" });
            serial::log("\r\n");
        }
    }
}

/// Initialize whitelist subsystem.
pub fn ly_whitelist_init() {
    unsafe {
        AUDIT_HEAD = 0;
        AUDIT_COUNT = 0;
    }
    serial::log("   ipc/whitelist: ready (4 channels, 16 slots each)\r\n");
}
