//! SMP 多核启动与调度基础（LYSOR OS，no_std x86_64）。
//!
//! 子模块：
//!   - `spinlock`: `Spinlock<T>` + `SpinGuard<T>`，`AtomicBool` + `pause`
//!   - `cpu_local`: 每 CPU 存储，`GS` 段 / `MSR_GS_BASE` 指向 `CpuLocal`
//!   - `lapic`: 本地 APIC（MMIO 0xFEE00000），ICR/SIPI/Timer/Error/Spurious
//!
//! 启动顺序：
//!   BSP 探测 CPUID → 分配 AP 内核栈 → 把 trampoline 复制到低 phys 地址
//!   → 发 INIT-SIPI-SIPI → 等 AP_START.started=1 → AP 调 ly_ap_entry()

pub mod spinlock;
pub mod cpu_local;
pub mod lapic;

use core::sync::atomic::{AtomicUsize, Ordering};

/// 最大支持 CPU 数（QEMU 默认最多 255，实际很少超过 4-8）。
pub const MAX_CPUS: usize = 16;

/// 当前在线 CPU 数（BSP 启动后递增，AP 在 trampoline 末尾 +1）。
pub static ONLINE_CPUS: AtomicUsize = AtomicUsize::new(0);

/// BSP 在启动时探测到的总 CPU 数（CPUID.EDX[APIC] + EAX[NumOfLogicalProcessors]）。
pub static TOTAL_CPUS: AtomicUsize = AtomicUsize::new(0);

/// 固定物理地址 0x8000 放 AP trampoline（<64KiB，16 位远跳偏移可达；
/// identity map 覆盖），SIPI vector=8。
pub const AP_TRAMPOLINE_PHYS: u64 = 0x8000;
pub const AP_SIPI_VECTOR: u8 = (AP_TRAMPOLINE_PHYS / 4096) as u8;

// Rust 访问 global_asm! 里的 trampoline 符号
extern "C" {
    static ap_trampoline32: u8;
    static ap_trampoline_end: u8;
    static mut ap_params: u64; // struct { start_ip:u64, stack_top:u64, cpu_id:u32, started:u32 }
}

// ---------------------------------------------------------------------------
// CPU 数探测
// ---------------------------------------------------------------------------

/// CPUID 探测物理包 / 核心 / 线程数。QEMU -smp N 时 N 个逻辑核，每个核
/// 一个 LAPIC，所以我们用 EAX 的 NumOfLogicalProcessors 作为 CPU 数。
pub fn probe_cpu_count() -> usize {
    let mut ebx_val: u32 = 0;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov [{0}], ebx",
            "pop rbx",
            in(reg) &mut ebx_val,
            inout("eax") 1u32 => _,
            out("ecx") _,
            out("edx") _,
            options(nostack, preserves_flags),
        );
    }
    let per_pkg = ((ebx_val >> 16) & 0xFF) as usize;
    crate::serial::log("   smp: cpuid.1 ebx=0x");
    crate::serial::log_hex(ebx_val as u64);
    crate::serial::log(" per_pkg=");
    crate::serial::log_u(per_pkg as u64);
    crate::serial::log("\r\n");
    if per_pkg == 0 { return 1; }
    // 总逻辑核数 = 每包逻辑核数（单包机器即总核数）。CPUID.11 的
    // EAX[15:0] 只是"SMT 每核线程数"，不能当总核数用。
    per_pkg
}

// ---------------------------------------------------------------------------
// BSP 启动 AP
// ---------------------------------------------------------------------------

/// 把 BSP 写好的参数塞进 global_asm! 里的 ap_params 区（phys==virt）。
unsafe fn write_ap_params(start_ip: u64, stack_top: u64, cpu_id: u32) {
    let base = &raw mut ap_params as u64;
    // struct: start_ip (offset 0, 8B), stack_top (offset 8, 8B), cpu_id (offset 16, 4B), started (offset 20, 4B)
    core::ptr::write_volatile(base as *mut u64, start_ip);
    core::ptr::write_volatile((base + 8) as *mut u64, stack_top);
    core::ptr::write_volatile((base + 16) as *mut u32, cpu_id);
    core::ptr::write_volatile((base + 20) as *mut u32, 0); // started 清零
    core::arch::asm!("mfence", options(nostack, preserves_flags));
}

unsafe fn ap_params_started() -> u32 {
    let base = &raw const ap_params as u64;
    core::ptr::read_volatile((base + 20) as *const u32)
}

/// 把编译好的 trampoline 复制到低物理地址（identity map 可直接写）。
unsafe fn copy_trampoline_to_low() {
    let src = &raw const ap_trampoline32 as *const u8;
    let end = &raw const ap_trampoline_end as *const u8;
    let len = end as usize - src as usize;
    crate::serial::log("   smp: trampoline src=0x");
    crate::serial::log_hex(src as u64);
    crate::serial::log(" len=0x");
    crate::serial::log_hex(len as u64);
    crate::serial::log("\r\n");
    let dst = AP_TRAMPOLINE_PHYS as *mut u8;
    core::ptr::copy_nonoverlapping(src, dst, len);
    // 校验：读回前 8 字节与源对比
    let src8 = core::slice::from_raw_parts(src, 8);
    let dst8 = core::slice::from_raw_parts(dst as *const u8, 8);
    crate::serial::log("   smp: trampoline src[0..8]=");
    for b in src8 {
        crate::serial::log_hex(*b as u64);
        crate::serial::log(" ");
    }
    crate::serial::log(" dst[0..8]=");
    for b in dst8 {
        crate::serial::log_hex(*b as u64);
        crate::serial::log(" ");
    }
    crate::serial::log("\r\n");
    // 把 0x8000 所在页保留标记为 used，防止 alloc_frame 错分配
    crate::memory::physical::reserve_frames_at(AP_TRAMPOLINE_PHYS, (len as u64 + 4095) / 4096);
}

/// 分配 AP 内核栈（4 页 = 16 KiB）。注意：alloc_frame 不保证连续，但 boot 早期
/// 分配器顺序分配通常连续；连续性问题另行修复（静态数组曾改变镜像布局触发 crypto 打印 #GP）。
unsafe fn alloc_ap_stack(_cpu: usize) -> Option<u64> {
    let mut base = 0u64;
    for _f in 0..4u64 {
        let frame = crate::memory::physical::alloc_frame()?;
        if base == 0 { base = frame; }
        // 标记 canary
        crate::security::stack_canary_arm(frame + 4096, 0);
    }
    if base == 0 { return None; }
    Some(base + 4 * 4096)
}

/// 启动所有 AP：探测 CPU 数，依次 INIT→SIPI→SIPI。
/// 单核（探测到 =1 或 =BSP 自己）直接跳过。
pub fn ly_smp_start_aps() {
    let total = probe_cpu_count();
    let n = total.min(MAX_CPUS);
    TOTAL_CPUS.store(n, Ordering::Relaxed);
    crate::serial::log("   smp: CPU count detected = ");
    crate::serial::log_u(n as u64);
    crate::serial::log("\r\n");

    if n <= 1 {
        crate::serial::log("   smp: single-CPU mode (skipping AP bring-up)\r\n");
        ONLINE_CPUS.store(1, Ordering::SeqCst);
        crate::sched::mark_cpu_online(0);
        return;
    }

    // 把 trampoline 代码复制到低物理地址，再发 SIPI（AP 从 0x8000 开始执行）
    unsafe {
        copy_trampoline_to_low();
    }

    let start_ip = ly_ap_entry as *const () as u64;
    crate::serial::log("   smp: trampoline copied to phys 0x");
    crate::serial::log_hex(AP_TRAMPOLINE_PHYS);
    crate::serial::log(", entry=0x");
    crate::serial::log_hex(start_ip);
    crate::serial::log("\r\n");

    // 启动 AP 1..n-1（CPU0 是 BSP）
    // IPI 发送窗口禁中断：BSP 的 PIT/LAPIC tick 若打断 ICR_HI→ICR_LO
    // 两段写之间的时序，SIPI 可能丢失（实测 CPU3 偶发完全不执行）。
    unsafe { core::arch::asm!("cli"); }
    for cpu in 1..n {
        let stack_top = unsafe {
            match alloc_ap_stack(cpu) {
                Some(s) => s,
                None => {
                    crate::serial::log("   smp: failed to alloc stack for CPU");
                    crate::serial::log_u(cpu as u64);
                    crate::serial::log("\r\n");
                    continue;
                }
            }
        };
        crate::serial::log("   smp: CPU");
        crate::serial::log_u(cpu as u64);
        crate::serial::log(" stack_top=0x");
        crate::serial::log_hex(stack_top);
        crate::serial::log("\r\n");

        unsafe {
            write_ap_params(start_ip, stack_top, cpu as u32);
        }

        // INIT IPI
        lapic::send_init(cpu as u32);
        // QEMU 要求 INIT 后等 >= 10ms 才能收 SIPI；TCG 下 spin 计数不可靠，
        // 用 I/O 延迟（out 0x80，QEMU 下每次约 1us）。
        for _ in 0..12000u32 {
            unsafe { core::arch::asm!("out dx, al", in("dx") 0x80u16, in("al") 0u8, options(nostack)); }
        }
        // SIPI #1
        lapic::send_sipi(cpu as u32, AP_SIPI_VECTOR);
        for _ in 0..300u32 {
            unsafe { core::arch::asm!("out dx, al", in("dx") 0x80u16, in("al") 0u8, options(nostack)); }
        }
        // SIPI #2（QEMU 要求发两次）
        lapic::send_sipi(cpu as u32, AP_SIPI_VECTOR);

        // 等 AP 回报 started（最多 2s）
        for _ in 0..2_000_000u32 {
            if unsafe { ap_params_started() } == 1 { break; }
            for _ in 0..200u32 { core::hint::spin_loop(); }
        }
        let started = unsafe { ap_params_started() };
        crate::serial::log("   smp: CPU");
        crate::serial::log_u(cpu as u64);
        crate::serial::log(" AP started=");
        crate::serial::log_u(started as u64);
        crate::serial::log("\r\n");
    }
    // IPI 窗口结束，恢复中断
    unsafe { core::arch::asm!("sti"); }

    // 等 AP 的 ONLINE_CPUS 递增尽量收敛（SeqCst load；上限约 1s，
    // 避免个别 AP 的 fetch_add 晚到导致 BSP 死等）
    crate::serial::log("   smp: online after started loop = ");
    crate::serial::log_u(ONLINE_CPUS.load(Ordering::SeqCst) as u64);
    crate::serial::log("\r\n");
    let mut waited = 0u32;
    while ONLINE_CPUS.load(Ordering::SeqCst) < n {
        waited += 1;
        for _ in 0..200u32 { core::hint::spin_loop(); }
        if waited > 500_000 { break; } // 上限约 10s（TCG 下 AP 启动可能很慢）
    }
    crate::serial::log("   smp: waited=");
    crate::serial::log_u(waited as u64);
    crate::serial::log(" online=");
    crate::serial::log_u(ONLINE_CPUS.load(Ordering::SeqCst) as u64);
    crate::serial::log("\r\n");

    crate::serial::log("   smp: total online CPUs = ");
    crate::serial::log_u(ONLINE_CPUS.load(Ordering::Relaxed) as u64);
    crate::serial::log("\r\n");
}

/// Rust 侧 AP 入口（被 trampoline 64-bit 段 call 进来）。
/// 参数：rdi=cpu_id, rsi=stack_top, rdx=start_ip（调用约定见 asm）。
#[no_mangle]
pub extern "C" fn ly_ap_entry(cpu_id: u32, _stack_top: u64, _start_ip: u64) -> ! {
    // 0. 最先写 started：BSP 的等待循环以此为准。先写可以让 BSP 不被
    //    AP 后续初始化（GDT/IDT/LAPIC）拖住；AP 自身继续完成完整初始化。
    unsafe {
        let base = &raw mut ap_params as u64;
        core::ptr::write_volatile((base + 20) as *mut u32, 1);
    }

    // 1. 设置 GS_BASE → CPU_LOCALS[cpu_id]
    unsafe {
        let slot = &mut cpu_local::CPU_LOCALS[cpu_id as usize];
        slot.cpu_id = cpu_id;
        slot.current_tid = 0; // 先 0，进入调度器后更新
        cpu_local::write_gs_base(slot as *mut cpu_local::CpuLocal as u64);
        // 读回诊断：确认 GS_BASE 真的写了（AP 偶发 GS_BASE=0 → cpu_id 误判）
        let gs = cpu_local::read_gs_base();
        if gs == 0 {
            crate::serial::log("   [AP] WARN gs_base=0 after write!\r\n");
        }
    }

    // 2. 加载共享 GDT + 本 CPU 的 TSS
    crate::gdt::ap_load_gdt_tss(cpu_id);

    // 2b. 加载共享 IDT（SIPI 后 IDTR 无效，sti 前必须装好）
    crate::idt::ap_load_idt();

    crate::serial::log("   [AP] lapic enable...\r\n");

    // 3. 打开 LAPIC（spurious + error 向量，enable bit）
    lapic::lapic_enable();

    // 4. AP 的调度由 BSP 的 PIT tick 发固定 IPI（0xF0）驱动（QEMU TCG
    //    下 AP 的 LAPIC timer 在 hlt 时冻结，偶尔在非 hlt 窗口触发会与
    //    BSP 的 spawn 抢锁，导致随机卡死）。因此 AP 的 LAPIC timer 保持
    //    masked（不使能），只读回寄存器做诊断。
    lapic::lapic_timer_mask();
    // 读回诊断：确认 AP 的 LAPIC timer 寄存器真的写进去了
    {
        let lvtt = lapic::read_lvtt_timer();
        let tic = lapic::read_tic();
        let tcc = lapic::read_tcc();
        crate::serial::log("   [AP] timer regs lvtt=");
        crate::serial::log_hex(lvtt as u64);
        crate::serial::log(" tic=");
        crate::serial::log_hex(tic as u64);
        crate::serial::log(" tcc=");
        crate::serial::log_hex(tcc as u64);
        crate::serial::log("\r\n");
    }

    crate::serial::log("   [AP] timer started, writing started\r\n");

    // 5. 标记 started（已提前到函数最前，BSP 不等待后续初始化）
    //    （原第 5 步已移除）

    // 6. 递增 ONLINE_CPUS，serial 打印（SeqCst：与 BSP 的等待循环/load 强一致）
    ONLINE_CPUS.fetch_add(1, Ordering::SeqCst);

    // 7. 更新调度器 per-CPU current_tid 表
    crate::sched::mark_cpu_online(cpu_id);

    crate::serial::log("   [AP] cpu");
    crate::serial::log_u(cpu_id as u64);
    crate::serial::log(" online\r\n");

    // 8. 进入调度循环（和 BSP 一样）。空闲用 hlt：AP 的调度由 BSP 的
    //    PIT tick 通过固定 vector IPI 驱动（QEMU TCG 下 AP 的 LAPIC
    //    timer 在 hlt 时会冻结虚拟时钟不触发；IPI 唤醒可靠且可控频率）。
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe {
            core::arch::asm!(
                "hlt",
                options(nostack, preserves_flags),
            );
        }
    }
}

// ---- End of file ----
