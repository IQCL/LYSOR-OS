//! 系统日志服务 (syslog).
//! 收集内核+服务日志, 分级存储.

use crate::serial;

const LOG_BUF: usize = 256;
static mut LOGS: [[u8; 64]; LOG_BUF] = [[0u8; 64]; LOG_BUF];
static mut LOG_HEAD: usize = 0;
static mut LOG_COUNT: usize = 0;

pub fn log_write(msg: &str, level: u8) {
    unsafe {
        let slot = LOG_HEAD % LOG_BUF;
        let b = msg.as_bytes();
        let n = b.len().min(63);
        LOGS[slot][..n].copy_from_slice(&b[..n]);
        LOGS[slot][63] = level;
        LOG_HEAD += 1;
        if LOG_COUNT < LOG_BUF { LOG_COUNT += 1; }
    }
}

pub fn log_dump() {
    unsafe {
        serial::log("   syslog: ");
        serial::log_u(LOG_COUNT as u64);
        serial::log(" entries\r\n");
    }
}

pub fn syslog_init() {
    log_write("syslog started", 6);
    serial::log("   syslog: service ready\r\n");
}
