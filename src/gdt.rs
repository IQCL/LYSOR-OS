//! GDT + TSS setup for LYSOR OS (SMP-ready).
//!
//! 共享 GDT：所有 CPU 加载同一张表，但 LTR 指向自己的 TSS 选择子。
//! GDT 布局（按槽号，每槽 8 字节；TSS 占 2 槽 = 16 字节）：
//!   0  null
//!   1  kcode  0x08
//!   2  kdata  0x10
//!   3  ucode  0x18
//!   4  udata  0x20
//!   5 + 2*i   tss_lo for CPU i   (选择子 = 0x28 + 16*i)
//!   6 + 2*i   tss_hi for CPU i
//!
//! 每个 CPU 的 `CpuLocal.tss_sel` 告诉它 ltr 用哪个选择子。
//!
//! `set_tss_rsp0` 在调度器 switch_to 时被调用：直接写该 CPU 的 TSS，
//! 这样 ring3 进内核时自动切到目标线程的 kernel stack。

use core::arch::asm;

use crate::smp::MAX_CPUS;

#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GdtEntry(u64);

impl GdtEntry {
    const fn null() -> Self { GdtEntry(0) }
    const fn kernel_code() -> Self { GdtEntry(0x00AF9A00_0000FFFF) }
    const fn kernel_data() -> Self { GdtEntry(0x00CF9200_0000FFFF) }
    const fn user_code() -> Self { GdtEntry(0x00AFFA00_0000FFFF) }
    const fn user_data() -> Self { GdtEntry(0x00CFF200_0000FFFF) }
}

/// 64-bit Task State Segment（只有 rsp0..2 和 IST 真正被硬件用到）。
/// 注意：x86 标准 TSS 要求 rsp0 在 offset 4。Rust `#[repr(C)]` 会在
/// `reserved0:u32` 后给 `rsp0:u64` 插 4 字节 padding（rsp0 落到 offset 8），
/// CPU 按 TR.base+4 读到的会是 padding（0）→ 内核栈指针 0 →
/// pushq 写非 canonical 地址 → #SS(0) 双故障。故必须 `packed`。
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TaskStateSegment {
    reserved0: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iopb: u16,
}

impl TaskStateSegment {
    const fn empty() -> Self {
        TaskStateSegment {
            reserved0: 0, rsp0: 0, rsp1: 0, rsp2: 0,
            reserved1: 0, ist: [0; 7], reserved2: 0, reserved3: 0, iopb: 0,
        }
    }
}

/// 每 CPU 一份 TSS（BSP 和每个 AP 各自独立）。
pub static mut TSS_PER_CPU: [TaskStateSegment; MAX_CPUS] = [TaskStateSegment::empty(); MAX_CPUS];

/// TSS[i] 在 GDT 里的选择子（ltr 用）。0x28 起，每 CPU 步进 16（2 个 GDT 槽）。
pub const fn tss_sel_for(cpu_id: u32) -> u16 {
    0x28 + (cpu_id as u16) * 16
}

// ---- GDT ----
// 槽数：null(1) + code/data(4) + MAX_CPUS * 2（TSS lo+hi）= 5 + 2*MAX_CPUS

/// GDT 槽类型常量（便于遍历填充）。
#[repr(C)]
struct GdtSlots {
    entries: [u64; 5 + 2 * MAX_CPUS],
}

static mut GDT: GdtSlots = GdtSlots {
    entries: [0u64; 5 + 2 * MAX_CPUS],
};

// ---------------------------------------------------------------------------
// 初始化：BSP 调一次（整个 kernel runtime 只用这张 GDT）
// ---------------------------------------------------------------------------

pub fn init_gdt() {
    unsafe {
        let g = &mut GDT.entries;
        g[0] = GdtEntry::null().0;
        g[1] = GdtEntry::kernel_code().0;
        g[2] = GdtEntry::kernel_data().0;
        g[3] = GdtEntry::user_code().0;
        g[4] = GdtEntry::user_data().0;

        // 为每个 CPU 写好 TSS 描述符
        for cpu in 0..MAX_CPUS {
            let tss_base = &raw const TSS_PER_CPU[cpu] as u64;
            let limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;
            // 64-bit TSS descriptor: lo = limit[15:0] | base[15:0]<<16 |
            // base[23:16]<<32 | type<<40 | limit[19:16]<<48 | base[31:24]<<56;
            // hi = base[63:32].  Missing base[31:24] (bits 56-63) would drop
            // the high byte of the base (e.g. 0x2817FB0 -> 0x0817FB0) and make
            // the CPU read rsp0 from a wrong frame -> #SS(0).
            let lo: u64 = limit
                | ((tss_base & 0xFFFF) << 16)
                | (((tss_base >> 16) & 0xFF) << 32)
                | (0x89u64 << 40) // present, ring0, system, 64-bit TSS available
                | (((limit >> 16) & 0xF) << 48)
                | (((tss_base >> 24) & 0xFF) << 56);
            let hi: u64 = (tss_base >> 32) & 0xFFFFFFFF;
            g[5 + 2 * cpu] = lo;
            g[6 + 2 * cpu] = hi;
        }
    }

    let ptr = GdtPtr {
        limit: (core::mem::size_of::<GdtSlots>() - 1) as u16,
        base: &raw const GDT as u64,
    };
    // BSP 先加载 GDT + TSS0
    unsafe {
        asm!(
            "lgdt [{}]",
            in(reg) &ptr,
            options(readonly, nostack)
        );
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            options(nostack)
        );
        asm!(
            "ltr ax",
            in("ax") tss_sel_for(0),
            options(nostack, nomem)
        );
    }
    crate::serial::log("   GDT: loaded, CS=0x08 DS=0x10, per-CPU TSS[0..");
    crate::serial::log_u(MAX_CPUS as u64);
    crate::serial::log("] sel=0x28..\r\n");
}

/// AP 调用：共享 GDT 已经被 BSP 填好，AP 只要 lgdt + ltr(自己的选择子)。
pub fn ap_load_gdt_tss(cpu_id: u32) {
    unsafe {
        let ptr = GdtPtr {
            limit: (core::mem::size_of::<GdtSlots>() - 1) as u16,
            base: &raw const GDT as u64,
        };
        asm!(
            "lgdt [{}]",
            in(reg) &ptr,
            options(readonly, nostack)
        );
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            // 注意：不要 mov gs！64 位模式下加载数据段会把 GS 的
            // shadow base 置 0，覆盖 ly_ap_entry 已写入的
            // MSR_GS_BASE（AP 的 cpu_id() 误判为 0 会引发互相发 IPI
            // 的风暴）。long mode 下 GS 基址由 MSR_GS_BASE 决定，
            // 内核代码一律经 rdgsbase 读取。
            "mov ss, ax",
            options(nostack)
        );
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            options(nostack)
        );
        let sel = tss_sel_for(cpu_id);
        // 诊断：打印本 CPU 的 TSS 描述符（GDT[5+2*cpu] lo），ltr #GP 时定位
        {
            let gdt_base = &raw const GDT as *const u64;
            let e5 = *gdt_base.add(5 + 2 * cpu_id as usize);
            let e6 = *gdt_base.add(6 + 2 * cpu_id as usize);
            crate::serial::log("   [AP] GDT[tss_lo]=");
            crate::serial::log_hex(e5);
            crate::serial::log(" hi=");
            crate::serial::log_hex(e6);
            crate::serial::log("\r\n");
        }
        asm!(
            "ltr ax",
            in("ax") sel,
            options(nostack, nomem)
        );
    }
}

/// 把当前 CPU 的 TSS.rsp0 指到 ring3 中断进入内核要用的栈顶（减 canary）。
/// 调度器 `switch_to` 在切到 ring3 进程前必须调这个。
pub fn set_tss_rsp0(top: u64) {
    let cpu = crate::smp::cpu_local::cpu_id();
    unsafe {
        // 注意：TSS_PER_CPU 是 BSS，identity map 下 phys==virt，多 CPU 可并行写各自 slot。
        // packed 结构字段不可直接借用（E0793），按标准 offset 4 手动写。
        let p = (TSS_PER_CPU.as_ptr().add(cpu as usize) as *mut u8).add(4) as *mut u64;
        p.write_volatile(top);
    }
}

/// 读回指定 CPU 的 TSS.rsp0（写入生效性校验用）。
pub fn tss_rsp0_of(cpu: u32) -> u64 {
    unsafe {
        let p = (TSS_PER_CPU.as_ptr().add(cpu as usize) as *const u8).add(4) as *const u64;
        p.read_volatile()
    }
}

pub const KCODE_SEL: u16 = 0x08;
pub const KDATA_SEL: u16 = 0x10;
pub const UCODE_SEL: u16 = 0x18 | 3;
pub const UDATA_SEL: u16 = 0x20 | 3;
/// BSP 用的 TSS 选择子（其他 CPU 用 `tss_sel_for(cpu_id)`）。
pub const TSS_SEL: u16 = 0x28;
