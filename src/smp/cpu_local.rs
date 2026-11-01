//! 每 CPU 存储：`GS` 段指向的 `CpuLocal` 结构。
//!
//! 每个 CPU 通过 `MSR_GS_BASE` 把 GS 指向自己的 `CpuLocal`。之后用
//! `rdgsbase` + Rust 指针偏移快速访问当前 CPU 状态。
//!
//! 所有字段都是 `Copy` 类型，方便用 `[CpuLocal; MAX_CPUS]` 数组。

use super::MAX_CPUS;

/// MSR_GS_BASE（per-CPU）。
const MSR_GS_BASE: u32 = 0xC0000101;

/// 每 CPU 的本地数据。`Copy` 类型确保可以用 `[CpuLocal; MAX_CPUS]` 静态数组。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CpuLocal {
    pub cpu_id: u32,
    pub current_tid: u32,
    /// 本 CPU 的 TSS 指针（ring3 中断时自动切栈到这里的 rsp0）。
    pub tss_ptr: u64,
    /// 本 CPU 的 GDT 基址。
    pub gdt_base: u64,
    /// lapic MMIO 基址（通常 0xFEE0_0000，固定）。
    pub lapic_base: u64,
    /// 该 CPU 在线状态（1=在线）。
    pub online: u64,
    // ---- 填充到 cache-line 64B，避免 false sharing ----
    _pad: [u8; 16],
}

impl CpuLocal {
    pub const fn zero() -> Self {
        CpuLocal {
            cpu_id: 0,
            current_tid: 0,
            tss_ptr: 0,
            gdt_base: 0,
            lapic_base: 0xFEE0_0000,
            online: 0,
            _pad: [0u8; 16],
        }
    }
}

/// 所有 CPU 的 CpuLocal 静态数组（BSS），BSP/AP 用 `wrmsr GS_BASE` 挂上自己的 slot。
pub static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [CpuLocal::zero(); MAX_CPUS];

/// 读 `MSR_GS_BASE`（当前 CPU）。
#[inline(always)]
pub fn read_gs_base() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") MSR_GS_BASE,
            out("rax") v,
            out("rdx") _,
            options(nostack, preserves_flags)
        );
    }
    v
}

/// 写 `MSR_GS_BASE`（当前 CPU）。
#[inline(always)]
pub fn write_gs_base(v: u64) {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") MSR_GS_BASE,
            in("rax") v as u32,
            in("rdx") (v >> 32) as u32,
            options(nostack, preserves_flags)
        );
    }
}

/// 返回当前 CPU 的 `&CpuLocal`（先 rdgsbase 再转引用）。
#[inline(always)]
pub fn cur() -> &'static CpuLocal {
    let base = read_gs_base();
    if base == 0 {
        return unsafe { &CPU_LOCALS[0] };
    }
    unsafe { &*(base as *const CpuLocal) }
}

#[inline(always)]
pub fn cur_mut() -> &'static mut CpuLocal {
    let base = read_gs_base();
    if base == 0 {
        return unsafe { &mut CPU_LOCALS[0] };
    }
    unsafe { &mut *(base as *mut CpuLocal) }
}

/// 当前 CPU id（0-based）。
#[inline(always)]
pub fn cpu_id() -> u32 {
    cur().cpu_id
}

/// 当前 tid（调度器切换时更新）。
#[inline(always)]
pub fn current_tid() -> u32 {
    cur().current_tid
}

/// 初始化 CPU0 的 CpuLocal（BSP 在 ly_sched_init 后调用）。
pub fn bsp_init() {
    unsafe {
        let slot = &mut CPU_LOCALS[0];
        slot.cpu_id = 0;
        slot.current_tid = crate::sched::current_tid();
        let base = slot as *const CpuLocal as u64;
        write_gs_base(base);
    }
}
