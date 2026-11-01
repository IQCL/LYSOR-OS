//! LYSOR OS IPC subsystem (`ly_ipc`).
//!
//! Two channels:
//! - **Channel A**: copy-based message passing (all processes).
//! - **Channel B**: shared-memory trusted IPC (whitelist-gated, bounds-checked).
//!
//! All public API uses `ly_` prefix.

pub mod channel_a;
pub mod channel_b;
pub mod whitelist;

use crate::serial;

/// Initialize IPC subsystem.
pub fn ly_ipc_init() {
    serial::log(">> initializing IPC subsystem...\r\n");
    channel_a::ly_chan_a_init();
    channel_b::ly_chan_b_init();
    serial::log("   ipc: subsystem ready\r\n");
}
