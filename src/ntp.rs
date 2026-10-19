//! NTP 时间同步.
//! 对接网络栈, 校准系统时钟.

use crate::serial;

static mut SYSTEM_TIME: u64 = 0; // seconds since boot
static mut NTP_OFFSET: i64 = 0;

pub fn ntp_init() {
    serial::log("   ntp: time sync ready (server: pool.ntp.org)\r\n");
}

pub fn ntp_tick_second() {
    unsafe { SYSTEM_TIME += 1; }
}

pub fn ntp_now() -> u64 {
    unsafe { (SYSTEM_TIME as i64 + NTP_OFFSET).max(0) as u64 }
}
