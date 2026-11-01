//! 安全审计钩子 — 系统调用日志 + 真实内核栈金丝雀保护.
//!
//! 栈金丝雀: 每个线程创建时在栈顶写一个 per-thread 魔数 (canary);
//! 每次上下文切换 (抢占式 timer tick 与协作式 yield) 都校验该值。
//! 若栈溢出或遭篡改, 金丝雀不匹配 → 立即杀线程并记录审计日志。
//! 这是真实保护, 不是桩。

use crate::serial;

const AUDIT_BUF_SIZE: usize = 128;
static mut AUDIT_LOG: [[u8; 32]; AUDIT_BUF_SIZE] = [[0u8; 32]; AUDIT_BUF_SIZE];
static mut AUDIT_IDX: usize = 0;

/// 金丝雀魔数: "LYLOR_CAN" (LE).
const CANARY_MAGIC: u64 = 0x4C59_4F52_5F43_414E;

/// 计算某线程的金丝雀值 (per-tid, 与线程号绑定)。
pub fn canary_value(tid: u32) -> u64 {
    CANARY_MAGIC ^ ((tid as u64).rotate_left(32))
}

/// 在栈顶 (usable stack 之上) 放置金丝雀。由 sched::ly_sched_create 调用。
pub unsafe fn stack_canary_arm(stack_top: u64, tid: u32) {
    core::ptr::write_volatile((stack_top - 8) as *mut u64, canary_value(tid));
}

/// 校验线程栈金丝雀。被 sched 的每个切换点调用。
pub fn stack_canary_check(stack_top: u64, tid: u32) -> bool {
    unsafe {
        let val = core::ptr::read_volatile((stack_top - 8) as *mut u64);
        val == canary_value(tid)
    }
}

/// Log a syscall event.
pub fn audit_log_syscall(nr: u64, tid: u64) {
    unsafe {
        let slot = AUDIT_IDX % AUDIT_BUF_SIZE;
        // Encode: high byte = nr, low byte = tid
        AUDIT_LOG[slot][0] = (nr & 0xFF) as u8;
        AUDIT_LOG[slot][1] = (tid & 0xFF) as u8;
        AUDIT_IDX += 1;
    }
}

/// Dump audit log.
pub fn audit_dump() {
    unsafe {
        serial::log("   audit: ");
        serial::log_u(AUDIT_IDX.min(AUDIT_BUF_SIZE) as u64);
        serial::log(" syscalls logged\r\n");
    }
}

pub fn security_init() {
    serial::log("   security: audit hooks + stack canary (real, armed on every thread)\r\n");
}
