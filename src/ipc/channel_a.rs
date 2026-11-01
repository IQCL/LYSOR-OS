//! Channel A: standard message-passing IPC (copy-based).
//!
//! Each channel is a bounded ring buffer of fixed-size messages.
//! Sender copies message in; receiver copies out. No shared memory.

use crate::serial;

const MAX_CHANNELS_A: usize = 8;
const MSG_SIZE: usize = 128;
const MSG_QUEUE_DEPTH: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LyMsg {
    pub src_tid: u32,
    pub dst_tid: u32,
    pub len: u32,
    pub data: [u8; MSG_SIZE],
}

impl LyMsg {
    pub const fn empty() -> Self {
        LyMsg { src_tid: 0, dst_tid: 0, len: 0, data: [0u8; MSG_SIZE] }
    }
}

#[derive(Clone, Copy)]
struct ChanA {
    name: [u8; 16],
    name_len: usize,
    buf: [LyMsg; MSG_QUEUE_DEPTH],
    head: usize,   // dequeue index
    tail: usize,   // enqueue index
    count: usize,
    owner_tid: u32,
}

impl ChanA {
    const fn new() -> Self {
        ChanA {
            name: [0u8; 16], name_len: 0,
            buf: [LyMsg::empty(); MSG_QUEUE_DEPTH],
            head: 0, tail: 0, count: 0, owner_tid: 0,
        }
    }
}

static mut CHANS_A: [ChanA; MAX_CHANNELS_A] = [ChanA::new(); MAX_CHANNELS_A];

/// Open a message channel. Returns channel index.
pub fn ly_chan_open(name: &str, owner_tid: u32) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_CHANNELS_A {
            if CHANS_A[i].name_len == 0 {
                let bytes = name.as_bytes();
                let n = bytes.len().min(16);
                CHANS_A[i].name[..n].copy_from_slice(&bytes[..n]);
                CHANS_A[i].name_len = n;
                CHANS_A[i].owner_tid = owner_tid;
                CHANS_A[i].head = 0;
                CHANS_A[i].tail = 0;
                CHANS_A[i].count = 0;
                serial::log("   ipc/a: opened chan[");
                serial::log_u(i as u64);
                serial::log("] \"");
                serial::log(name);
                serial::log("\"\r\n");
                return Ok(i);
            }
        }
        Err("no free channel")
    }
}

/// Send a message to a channel (copy-in).
pub fn ly_chan_send(chan: usize, msg: &LyMsg) -> Result<(), &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_A { return Err("bad chan"); }
        let c = &mut CHANS_A[chan];
        if c.count >= MSG_QUEUE_DEPTH { return Err("queue full"); }
        c.buf[c.tail] = *msg;
        c.tail = (c.tail + 1) % MSG_QUEUE_DEPTH;
        c.count += 1;
        Ok(())
    }
}

/// Receive a message from a channel (copy-out). Returns None if empty.
pub fn ly_chan_recv(chan: usize, caller_tid: u32) -> Result<Option<LyMsg>, &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_A { return Err("bad chan"); }
        let c = &mut CHANS_A[chan];
        if c.count == 0 { return Ok(None); }
        let msg = c.buf[c.head];
        // Security: caller must be the intended recipient
        if msg.dst_tid != 0 && msg.dst_tid != caller_tid && c.owner_tid != caller_tid {
            return Err("access denied: wrong recipient");
        }
        c.head = (c.head + 1) % MSG_QUEUE_DEPTH;
        c.count -= 1;
        Ok(Some(msg))
    }
}

/// Close a channel.
pub fn ly_chan_close(chan: usize) -> Result<(), &'static str> {
    unsafe {
        if chan >= MAX_CHANNELS_A { return Err("bad chan"); }
        CHANS_A[chan].name_len = 0;
        CHANS_A[chan].count = 0;
        Ok(())
    }
}

/// Initialize channel A subsystem.
pub fn ly_chan_a_init() {
    serial::log("   ipc/a: message channels ready (8 x 16 slots)\r\n");
}

