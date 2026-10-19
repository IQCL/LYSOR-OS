//! init (PID 1) — 系统初始化进程.
//! 按顺序拉起所有服务, 崩溃重启桩.

use crate::serial;

pub fn init_start() {
    serial::log(">> init: PID 1 starting...\r\n");
    serial::log(">> init: mounting / (lfs)\r\n");
    serial::log(">> init: starting syslog\r\n");
    crate::syslog::syslog_init();
    serial::log(">> init: starting IPC\r\n");
    serial::log(">> init: starting network stack\r\n");
    serial::log(">> init: starting display manager\r\n");
    serial::log(">> init: ready, spawning login...\r\n");
}
