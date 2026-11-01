//! Channel B: high-speed trusted shared-memory IPC.
//!
//! A fixed-size shared memory region is mapped into multiple threads' address
//! spaces. Access is gated by whitelist. All reads/writes are bounds-checked.

use crate::memory::physical;
use crate::serial;
use super::whitelist;

const MAX_CHANNELS_B: usize = 4;
const SHM_SIZE: usize = 4096; // one page per channel

#[derive(Clone, Copy)]
struct ChanB {
    name: [u8; 16],
    name_len: usize,
    shm_base: u64,      // physical address of shared page
    shm_len: usize,
    ref_count: u32,
    created: bool,
}

impl ChanB {
    const fn new() -> Self {
        ChanB {
            name: [0u8; 16], name_len: 0,
            shm_base: 0, shm_len: 0, ref_count: 0, created: false,
        }
    }
}

static mut CHANS_B: [ChanB; MAX_CHANNELS_B] = [ChanB::new(); MAX_CHANNELS_B];

/// Create a shared-memory channel. Allocates one physical page.
pub fn ly_shm_create(name: &str, creator_tid: u32) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_CHANNELS_B {
            if !CHANS_B[i].created {
                let frame = physical::alloc_frame().ok_or("no frame")?;
                // Zero the page
                core::ptr::write_bytes(frame as *mut u8, 0, SHM_SIZE);

                let bytes = name.as_bytes();
                let n = bytes.len().min(16);
                CHANS_B[i].name[..n].copy_from_slice(&bytes[..n]);
                CHANS_B[i].name_len = n;
                CHANS_B[i].shm_base = frame;
                CHANS_B[i].shm_len = SHM_SIZE;
                CHANS_B[i].ref_count = 1;
                CHANS_B[i].created = true;

                // Add creator to whitelist
                whitelist::ly_whitelist_grant(i, name, creator_tid)?;

                serial::log("   ipc/b: shm created chan[");
                serial::log_u(i as u64);
                serial::log("] base=0x");
                serial::log_hex(frame);
                serial::log("\r\n");
                return Ok(i);
            }
        }
        Err("no free shm channel")
    }
}

/// Attach a thread to a shm channel (checks whitelist).
pub fn ly_shm_attach(chan: usize, caller_tid: u32, tick: u64) -> Result<u64, &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_B || !CHANS_B[chan].created {
            return Err("bad chan");
        }
        // Whitelist check + audit
        if !whitelist::ly_whitelist_check(chan, caller_tid, tick) {
            return Err("whitelist denied");
        }
        CHANS_B[chan].ref_count += 1;
        Ok(CHANS_B[chan].shm_base)
    }
}

/// Write to shared memory with bounds check.
pub fn ly_shm_write(chan: usize, offset: usize, data: &[u8], caller_tid: u32, tick: u64) -> Result<(), &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_B || !CHANS_B[chan].created {
            return Err("bad chan");
        }
        if !whitelist::ly_whitelist_check(chan, caller_tid, tick) {
            return Err("whitelist denied");
        }
        let end = offset + data.len();
        if end > CHANS_B[chan].shm_len {
            return Err("bounds overflow");
        }
        let base = CHANS_B[chan].shm_base as *mut u8;
        core::ptr::copy_nonoverlapping(data.as_ptr(), base.add(offset), data.len());
        Ok(())
    }
}

/// Read from shared memory with bounds check.
pub fn ly_shm_read(chan: usize, offset: usize, buf: &mut [u8], caller_tid: u32, tick: u64) -> Result<usize, &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_B || !CHANS_B[chan].created {
            return Err("bad chan");
        }
        if !whitelist::ly_whitelist_check(chan, caller_tid, tick) {
            return Err("whitelist denied");
        }
        let end = offset + buf.len();
        if end > CHANS_B[chan].shm_len {
            return Err("bounds overflow");
        }
        let base = CHANS_B[chan].shm_base as *const u8;
        core::ptr::copy_nonoverlapping(base.add(offset), buf.as_mut_ptr(), buf.len());
        Ok(buf.len())
    }
}

/// Detach from a shm channel.
pub fn ly_shm_detach(chan: usize, caller_tid: u32) -> Result<(), &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_B || !CHANS_B[chan].created {
            return Err("bad chan");
        }
        if CHANS_B[chan].ref_count > 0 {
            CHANS_B[chan].ref_count -= 1;
        }
        Ok(())
    }
}

/// Initialize channel B subsystem.
pub fn ly_chan_b_init() {
    whitelist::ly_whitelist_init();
    serial::log("   ipc/b: shared-memory channels ready (4 x 4K pages)\r\n");
}

