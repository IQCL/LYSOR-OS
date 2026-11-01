//! LYSOR OS - Process / Thread Scheduler (`ly_sched`) — SMP-ready.
//!
//! Preemptive round-robin scheduling via timer interrupt. 全局 TCB_TABLE
//! 由一个 `Spinlock` 保护，`NEXT_TID` 用 `AtomicUsize` 分配，每 CPU 的
//! 当前 tid 存在 `CURRENT_TID_PER_CPU[cpu_id]` 里，不再是单一全局变量。
//!
//! 抢占源：BSP 走 PIT (vector 32)，AP 走 LAPIC Timer (vector 0xFD)，
//! 两者最终都调 `ly_sched_timer_tick`，里面拿锁 + 挑下一个 Ready 线程。

pub mod capability;
pub mod pit;

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::memory::physical;
use crate::serial;
use crate::idt::TrapFrame;
use crate::smp::spinlock::Spinlock;
use crate::smp::cpu_local;
use crate::smp::MAX_CPUS;
use capability::CapSet;

const MAX_THREADS: usize = 32;
const STACK_PAGES: usize = 4; // 16 KiB per kernel stack
#[allow(dead_code)] // 派生值，预留（栈大小基准）
const STACK_SIZE: usize = STACK_PAGES * 4096;

#[derive(Clone, Copy, PartialEq)]
enum ThreadState {
    Ready,
    Running,
    Zombie,
}

#[derive(Clone, Copy)]
struct Tcb {
    tid: u32,
    name: &'static str,
    state: ThreadState,
    saved_rsp: u64,     // points to a TrapFrame on this thread's kernel stack
    stack_base: u64,
    stack_pages: usize,
    #[allow(dead_code)] // 线程入口，预留（user 态启动路径）
    entry: u64,
    caps: CapSet,
    runtime_ticks: u64,
    /// Address space root (PML4 physical). 0 = kernel thread (boot PML4).
    pml4: u64,
    /// Address-space registry slot for teardown, usize::MAX = none.
    as_slot: usize,
    /// 当前在哪个 CPU 上跑（-1 表示 Ready/Zombie）。
    home_cpu: u32,
    /// CPU 亲和：只允许在这个 CPU 上跑（u32::MAX = 任意 CPU）。
    aff_cpu: u32,
}

impl Tcb {
    const fn empty() -> Self {
        Tcb {
            tid: 0, name: "", state: ThreadState::Zombie,
            saved_rsp: 0, stack_base: 0, stack_pages: 0,
            entry: 0, caps: CapSet::empty(), runtime_ticks: 0,
            pml4: 0, as_slot: usize::MAX, home_cpu: u32::MAX,
            aff_cpu: u32::MAX,
        }
    }
}

// ---- 共享数据 ----
static TCB_LOCK: Spinlock<()> = Spinlock::new(());
static NEXT_TID: AtomicUsize = AtomicUsize::new(1);
static TOTAL_SWITCHES: AtomicUsize = AtomicUsize::new(0);

static mut TCB_TABLE: [Tcb; MAX_THREADS] = [Tcb::empty(); MAX_THREADS];

// ---- 每 CPU 独立数据 ----
static mut CURRENT_TID_PER_CPU: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

pub fn current_tid() -> u32 {
    let cid = cpu_local::cpu_id() as usize;
    // 若 GS 未初始化（极早），回退 CPU0
    let c = if cid >= MAX_CPUS { 0 } else { cid };
    unsafe { CURRENT_TID_PER_CPU[c].load(Ordering::Relaxed) as u32 }
}

pub fn mark_cpu_online(cpu_id: u32) {
    let c = cpu_id as usize;
    if c < MAX_CPUS {
        unsafe { CURRENT_TID_PER_CPU[c].store(0, Ordering::Relaxed); }
    }
}

// ---------------------------------------------------------------------------
// Thread creation（拿全局锁）
// ---------------------------------------------------------------------------

pub fn ly_sched_create(name: &'static str, entry: extern "C" fn()) -> Option<u32> {
    ly_sched_create_on_cpu(name, entry, u32::MAX)
}

/// 创建内核线程并绑定 CPU 亲和（aff = 指定 CPU；u32::MAX = 任意）。
pub fn ly_sched_create_on_cpu(
    name: &'static str,
    entry: extern "C" fn(),
    aff: u32,
) -> Option<u32> {
    let _g = TCB_LOCK.lock();
    unsafe {
        let mut slot = None;
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == 0 { slot = Some(i); break; }
        }
        let idx = slot?;

        let mut base = 0u64;
        for f in 0..STACK_PAGES {
            let frame = physical::alloc_frame()?;
            if f == 0 { base = frame; }
        }
        let stack_top = base + (STACK_PAGES as u64) * 4096;

        let rsp = build_fake_trapframe(stack_top, entry as u64);
        let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed) as u32;
        crate::security::stack_canary_arm(stack_top, tid);

        TCB_TABLE[idx] = Tcb {
            tid, name, state: ThreadState::Ready,
            saved_rsp: rsp, stack_base: base, stack_pages: STACK_PAGES,
            entry: entry as u64, caps: CapSet::all(), runtime_ticks: 0,
            pml4: 0, as_slot: usize::MAX, home_cpu: u32::MAX,
            aff_cpu: aff,
        };

        serial::log("   sched: created tid=");
        serial::log_u(tid as u64);
        serial::log(" \"");
        serial::log(name);
        if aff != u32::MAX {
            serial::log("\" aff=cpu");
            serial::log_u(aff as u64);
        } else {
            serial::log("\"");
        }
        serial::log("\r\n");
        Some(tid)
    }
}

pub fn ly_sched_create_process(
    name: &'static str,
    entry: u64,
    user_rsp: u64,
    pml4: u64,
    as_slot: usize,
) -> Option<u32> {
    let _g = TCB_LOCK.lock();
    unsafe {
        let mut slot = None;
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == 0 { slot = Some(i); break; }
        }
        let idx = slot?;

        let mut base = 0u64;
        for f in 0..STACK_PAGES {
            let frame = physical::alloc_frame()?;
            if f == 0 { base = frame; }
        }
        let stack_top = base + (STACK_PAGES as u64) * 4096;

        let rsp = build_fake_trapframe_user(stack_top, entry, user_rsp);
        let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed) as u32;
        crate::security::stack_canary_arm(stack_top, tid);

        TCB_TABLE[idx] = Tcb {
            tid, name, state: ThreadState::Ready,
            saved_rsp: rsp, stack_base: base, stack_pages: STACK_PAGES,
            entry, caps: CapSet::all(), runtime_ticks: 0,
            pml4, as_slot, home_cpu: u32::MAX, aff_cpu: u32::MAX,
        };

        serial::log("   sched: created PROCESS tid=");
        serial::log_u(tid as u64);
        serial::log(" \"");
        serial::log(name);
        serial::log("\" ring3 cr3=0x");
        serial::log_hex(pml4);
        serial::log("\r\n");
        Some(tid)
    }
}

// ---------------------------------------------------------------------------
// Fake TrapFrame builders（无锁，只在创建期调用）
// ---------------------------------------------------------------------------

unsafe fn build_fake_trapframe(stack_top: u64, entry: u64) -> u64 {
    let mut sp = stack_top - 8 - 176;
    sp -= 8; core::ptr::write(sp as *mut u64, 0x10);          // ss
    sp -= 8; core::ptr::write(sp as *mut u64, sp + 16);       // rsp (just above frame)
    sp -= 8; core::ptr::write(sp as *mut u64, 0x202);         // rflags (IF=1)
    sp -= 8; core::ptr::write(sp as *mut u64, 0x08);          // cs
    sp -= 8; core::ptr::write(sp as *mut u64, entry);          // rip
    sp -= 8; core::ptr::write(sp as *mut u64, 0);             // error_code
    sp -= 8; core::ptr::write(sp as *mut u64, 32);            // vector (timer)
    for _ in 0..15 {
        sp -= 8;
        core::ptr::write(sp as *mut u64, 0);
    }
    sp
}

unsafe fn build_fake_trapframe_user(stack_top: u64, entry: u64, user_rsp: u64) -> u64 {
    let mut sp = stack_top - 8 - 176;
    sp -= 8; core::ptr::write(sp as *mut u64, crate::gdt::UDATA_SEL as u64);
    sp -= 8; core::ptr::write(sp as *mut u64, user_rsp);
    sp -= 8; core::ptr::write(sp as *mut u64, 0x202);
    sp -= 8; core::ptr::write(sp as *mut u64, crate::gdt::UCODE_SEL as u64);
    sp -= 8; core::ptr::write(sp as *mut u64, entry);
    sp -= 8; core::ptr::write(sp as *mut u64, 0);
    sp -= 8; core::ptr::write(sp as *mut u64, 32);
    for _ in 0..15 {
        sp -= 8;
        core::ptr::write(sp as *mut u64, 0);
    }
    sp
}

// ---------------------------------------------------------------------------
// Exit / Reap
// ---------------------------------------------------------------------------

pub fn ly_sched_exit() -> ! {
    let _g = TCB_LOCK.lock();
    unsafe {
        let tid = current_tid();
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                TCB_TABLE[i].state = ThreadState::Zombie;
                break;
            }
        }
        ly_sched_pick_next_locked();
    }
}

pub fn ly_sched_reap() {
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].state == ThreadState::Zombie && TCB_TABLE[i].tid != 0 {
                for f in 0..TCB_TABLE[i].stack_pages {
                    physical::free_frame(TCB_TABLE[i].stack_base + (f as u64) * 4096);
                }
                if TCB_TABLE[i].as_slot != usize::MAX {
                    let freed = crate::memory::paging::release_address_space(TCB_TABLE[i].as_slot);
                    serial::log("   sched: address space of tid=");
                    serial::log_u(TCB_TABLE[i].tid as u64);
                    serial::log(" reclaimed (");
                    serial::log_u(freed as u64);
                    serial::log(" frames)\r\n");
                }
                TCB_TABLE[i].tid = 0;
                TCB_TABLE[i].name = "";
                TCB_TABLE[i].stack_base = 0;
                TCB_TABLE[i].stack_pages = 0;
                TCB_TABLE[i].pml4 = 0;
                TCB_TABLE[i].as_slot = usize::MAX;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Context switch
// ---------------------------------------------------------------------------

/// 在持有 TCB_LOCK 的前提下切换到 slot `idx`。返回值给 ISR stub 交换 rsp。
#[inline]
unsafe fn switch_to(idx: usize) -> u64 {
    let next_pml4 = TCB_TABLE[idx].pml4;
    let target = if next_pml4 == 0 { crate::memory::paging::kernel_pml4() } else { next_pml4 };
    if target != crate::memory::paging::current_cr3() {
        crate::memory::paging::switch_cr3(target);
    }
    if next_pml4 != 0 {
        let top = TCB_TABLE[idx].stack_base + (TCB_TABLE[idx].stack_pages as u64) * 4096 - 8;
        crate::gdt::set_tss_rsp0(top);
        let sel = crate::gdt::UDATA_SEL;
        core::arch::asm!(
            "mov ds, ax", "mov es, ax", "mov fs, ax", "mov gs, ax",
            in("ax") sel, options(nostack)
        );
        // mov gs 把 GS shadow base 清 0 了，重设 MSR_GS_BASE 指向本 CPU
        // 的 CpuLocal，保证后续 cpu_id()/current_tid() 仍正确。
        let c = crate::smp::lapic::apic_id() as usize;
        let c = if c < MAX_CPUS { c } else { 0 };
        let slot = &raw mut crate::smp::cpu_local::CPU_LOCALS[c];
        crate::smp::cpu_local::write_gs_base(slot as u64);
    }
    let cpu = crate::smp::lapic::apic_id() as usize;
    let cpu = if cpu < MAX_CPUS { cpu } else { 0 };
    CURRENT_TID_PER_CPU[cpu].store(TCB_TABLE[idx].tid as usize, Ordering::Relaxed);
    TCB_TABLE[idx].home_cpu = cpu as u32;
    TCB_TABLE[idx].saved_rsp
}

/// Timer ISR 入口（PIT vector 32 和 LAPIC Timer vector 0xFD 都会走到这里）。
/// 拿 TCB_LOCK，记录当前线程 rsp，挑下一个 Ready 线程，返回其 saved_rsp。
#[no_mangle]
pub extern "C" fn ly_sched_timer_tick(frame: &TrapFrame) -> u64 {
    // EOI：先 ACK 再调度（frame.vector 是切换前的中断号；>=0xF0 走 LAPIC，
    // 否则 PIC）。中断上下文 IF=0，EOI 后同 vector 可重入，但切换发生在
    // 本函数返回后由 asm 完成，此时已无 EOI 依赖。
    if frame.vector >= 0xF0 {
        crate::smp::lapic::eoi();
    } else {
        unsafe { core::arch::asm!("out 0x20, al", in("al") 0x20u8, options(nostack)); }
    }

    // boot 阶段（SMP_RUNNING=false）所有 CPU 都不抢占、不拿锁：
    // BSP 的 spawn/main 必须连续执行直到 worker 全部就绪。worker 是
    // 无限 ++ 循环不 yield，若 tick 提前把 main 切走，调度器每 tick
    // 只 pick 到 worker 自己（返回 0 继续），main 被永久饿死，spawn
    // 卡在中间——实测随机卡死（tid=3/4/7 后）。SMP_RUNNING 后才开始
    // 调度。boot 阶段 PIT 计数照常推进（dump 门控以此为基准）。
    if !SMP_RUNNING.load(Ordering::Relaxed) {
        pit::ly_pit_tick();
        return 0;
    }

    // try_lock：当前线程（或另一 CPU）可能正持有 TCB_LOCK（如 create_process/
    // create_on_cpu 的锁内段被本 tick 抢占），若自旋等锁 → main 等 tick 释放、
    // tick 等 main 释放 → 双死锁（实测：verify 秒过但卡在 user frame 打印）。
    // 抢不到锁就跳过本次调度（不切换、不保存），持锁线程继续执行到解锁。
    let _g = match TCB_LOCK.try_lock() {
        Some(g) => g,
        None => {
            pit::ly_pit_tick();
            return 0;
        }
    };
    let ticks = pit::ly_pit_tick();

    unsafe {
        // 用 LAPIC APIC ID 判定当前 CPU（读 MMIO，不经 GS——GS_BASE 在
        // AP 上偶发未生效，会导致 cpu_id()=0 误入 CPU0 分支互相发 IPI
        // 风暴，实测 int 日志 613MB / 43 万次 0xF0）。
        let my_cpu = crate::smp::lapic::apic_id();
        // CPU0 的 PIT tick：向所有 AP 发固定 IPI（vector 0xF0）驱动调度。
        // QEMU TCG 下 AP 的 LAPIC timer 在 hlt 时冻结，IPI 是可靠时钟源。
        // 放在拿锁后（AP 的 handler 也在等这把锁，发完即放）。
        // 每 2 个 PIT tick 发一轮降 TCG 开销；不依赖 ONLINE_CPUS 判定
        //（AP 的 online 计数与 BSP 的 load 偶发可见性滞后，漏发导致该
        // AP 永远不被调度；对未就绪 CPU 的 IPI 被 QEMU 丢弃，无害）。
        if my_cpu == 0 {
            if (ticks & 1) == 0 {
                let total = crate::smp::TOTAL_CPUS.load(Ordering::Relaxed);
                for c in 1..total {
                    crate::smp::lapic::send_ipi_fixed(c as u32, 0xF0);
                }
            }
        }
        // 诊断：AP 的 tick 计数（确认调度驱动到达 AP）
        if my_cpu != 0 {
            let idx = (my_cpu as usize).min(MAX_CPUS - 1);
            AP_TICK_CNT[idx].fetch_add(1, Ordering::Relaxed);
        }

        // 5 秒 SMP dump（只 CPU0 触发；PIT 100Hz，LAPIC timer 双计
        // 会提前到 ~2.5s，不影响验收）。
        // 门控：只有 SMP_RUNNING（main 已 entering preemptive loop）后
        // 才允许 dump——boot 阶段的 tick 不算（TCG 慢时 boot 可能
        // 超过 5 秒墙钟，worker 还没创建就 dump 会误报全 0）。
        // 失败不锁存：全 OK 才置 SMP_DUMPED，否则下个 500 tick 重试。
        if my_cpu == 0
            && SMP_RUNNING.load(Ordering::Relaxed)
            && ticks >= 50
            && ticks % 50 == 0
            && !SMP_DUMPED.load(Ordering::Relaxed)
        {
            drop(_g);
            if ly_sched_smp_dump() {
                SMP_DUMPED.store(true, Ordering::Relaxed);
            }
            return 0;
        }

        let cur_tid = current_tid();

        // SMP idle path: on every CPU, tid 0 means "no real thread" (APs have
        // no idle TCB; CPU0's idle TCB[0] must not be clobbered by APs).
        // Do not save/rotate a fake context — just pick a ready thread.
        if cur_tid == 0 {
            return pick_next_ready(0);
        }

        let current_rsp = frame as *const TrapFrame as u64;
        let cur_idx = TCB_TABLE.iter().position(|t| t.tid == cur_tid).unwrap_or(0);

        // Stack canary check（boot 栈 main：stack_pages==0 非 sched 分配，跳过）
        let cur_top = TCB_TABLE[cur_idx].stack_base
            + (TCB_TABLE[cur_idx].stack_pages as u64) * 4096;
        if TCB_TABLE[cur_idx].stack_pages != 0 && !crate::security::stack_canary_check(cur_top, cur_tid) {
            serial::log("   security: STACK CANARY VIOLATION tid=");
            serial::log_u(cur_tid as u64);
            serial::log(", thread killed\r\n");
            TCB_TABLE[cur_idx].state = ThreadState::Zombie;
            return pick_next_ready(cur_idx);
        }

        TCB_TABLE[cur_idx].saved_rsp = current_rsp;
        TCB_TABLE[cur_idx].runtime_ticks += 1;
        TCB_TABLE[cur_idx].state = ThreadState::Ready;
        TCB_TABLE[cur_idx].home_cpu = u32::MAX;

        let rsp = pick_next_ready(cur_idx);
        if rsp != 0 {
            TOTAL_SWITCHES.fetch_add(1, Ordering::Relaxed);
        }

        if ticks % 50 == 0 {
            drop(_g);
            ly_sched_reap();
        }
        rsp
    }
}

/// 每 CPU worker 计数器（SMP 验证用）。
pub static CPU_WORK_CNT: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static SMP_DUMPED: AtomicBool = AtomicBool::new(false);
/// main 进入 preemptive loop 后置 true；dump 只在此时之后触发。
pub static SMP_RUNNING: AtomicBool = AtomicBool::new(false);
/// 诊断：AP 各 CPU 的 timer tick 计数。
pub static AP_TICK_CNT: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// 每 CPU 专用 worker：在自己绑定的 CPU 上不停递增计数器。
/// 计数语义不变（每 1024 次空转 +1 次批量原子自增 1024，计数速率等同纯忙等），
/// 但原子操作频率降 1024 倍——MTTCG 多线程模拟下 AP 忙等不再以原子同步开销
/// 拖累其他 vCPU（Ed25519 大数验签等 CPU0 重计算因此不受 AP 竞争影响）。
pub extern "C" fn ly_sched_smp_worker() {
    let c = cpu_local::cpu_id() as usize;
    let c = if c < MAX_CPUS { c } else { 0 };
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 1023 == 0 {
            CPU_WORK_CNT[c].fetch_add(1024, Ordering::Relaxed);
        }
    }
}

/// Dump 各 CPU worker 计数（跑 5 秒后由 CPU0 tick 触发）。
/// 返回是否全部 CPU >= 1000（OK 才锁存 SMP_DUMPED）。
pub fn ly_sched_smp_dump() -> bool {
    serial::log(">> SMP verification: per-CPU worker counters (after 5s)\r\n");
    // 用探测 CPU 总数作为遍历边界（个别 AP 的 ONLINE_CPUS 递增可能晚到，
    // 但探测数才是权威）
    let total = crate::smp::TOTAL_CPUS.load(Ordering::Relaxed);
    let online = crate::smp::ONLINE_CPUS.load(Ordering::Relaxed);
    let mut all_ok = true;
    for c in 0..MAX_CPUS {
        if (c as usize) >= total && c != 0 {
            continue;
        }
        let v = CPU_WORK_CNT[c].load(Ordering::Relaxed);
        serial::log("   cpu");
        serial::log_u(c as u64);
        serial::log(" counter=");
        serial::log_u(v);
        if c != 0 {
            serial::log(" ticks=");
            serial::log_u(AP_TICK_CNT[c].load(Ordering::Relaxed));
        }
        serial::log(if v >= 1000 { " OK\r\n" } else { " LOW (<1000)\r\n" });
        // CPU0 无 smp_worker（main 独占跑启动+自检），SMP 验证只看 AP
        if c != 0 && c < online { all_ok &= v >= 1000; }
    }
    serial::log(if all_ok {
        ">> SMP OK: all online CPUs busy\r\n"
    } else {
        ">> SMP FAIL: some CPU counter below 1000\r\n"
    });
    all_ok
}

/// SMP dump 是否已完成（供 main 线程在非中断上下文做 BSOD 取证等）。
pub fn smp_dump_done() -> bool {
    SMP_DUMPED.load(Ordering::Relaxed)
}

/// 从 `cur_idx + 1` 开始扫 TCB_TABLE，找下一个可跑线程并切换。
/// 调用者必须持有 TCB_LOCK。
///
/// SMP 规则：
/// - idx 0 是 BSP 的 idle 线程，**只允许 CPU0 跑**（AP 永远跳过它）；
/// - 亲和优先：先挑 `aff_cpu == 本 CPU` 的 Ready 线程；
/// - 再挑 `aff_cpu == u32::MAX`（任意 CPU）的 Ready 线程；
/// - 都不满足时继续跑当前线程（返回 0）。
unsafe fn pick_next_ready(cur_idx: usize) -> u64 {
    // 亲和判定用 LAPIC APIC ID（不经 GS，理由同 ly_sched_timer_tick）
    let my_cpu = crate::smp::lapic::apic_id();
    for pass in 0..2u32 {
        for step in 1..=MAX_THREADS {
            let idx = (cur_idx + step) % MAX_THREADS;
            if idx == 0 { continue; } // BSP idle is pinned to CPU0
            let t = &TCB_TABLE[idx];
            if t.state != ThreadState::Ready { continue; }
            let ok = match pass {
                0 => t.aff_cpu == my_cpu,
                _ => t.aff_cpu == u32::MAX,
            };
            if !ok { continue; }
            TCB_TABLE[idx].state = ThreadState::Running;
            return switch_to(idx);
        }
    }
    // 没找到：继续跑当前线程
    TCB_TABLE[cur_idx].state = ThreadState::Running;
    0
}

/// 无返回的 pick_next（exit 路径、当前线程已 Zombie）。
unsafe fn ly_sched_pick_next_locked() -> ! {
    let my_cpu = cpu_local::cpu_id() as u32;
    for pass in 0..2u32 {
        for step in 1..=MAX_THREADS {
            let idx = (current_tid() as usize + step) % MAX_THREADS;
            if idx == 0 { continue; }
            let t = &TCB_TABLE[idx];
            if t.state != ThreadState::Ready { continue; }
            let ok = match pass {
                0 => t.aff_cpu == my_cpu,
                _ => t.aff_cpu == u32::MAX,
            };
            if !ok { continue; }
            TCB_TABLE[idx].state = ThreadState::Running;
            let rsp = switch_to(idx);
            // exit 路径持锁切换：ly_sched_exit 的 SpinGuard 永不 drop（栈被
            // 切走且 `-> !` 不返回），必须在此显式释放，否则新线程之后所有
            // tick 的 try_lock 永远失败、调度瘫痪（实测 exit(0x2A) 后其余
            // 线程全部饿死、只剩被切进去的线程在跑）。
            TCB_LOCK.force_unlock();
            // exit 路径没有 ISR stub 的尾部（stub call 永不返回），必须在此
            // 完成与 stub 相同的"切栈 + 弹 GPR + iretq"序列：新线程 saved_rsp
            // 指向的是其上次被切走时的 stub 帧（15×GPR + vector + err + 5×hw）。
            core::arch::asm!(
                "mov rsp, {0}",
                "pop r15", "pop r14", "pop r13", "pop r12",
                "pop r11", "pop r10", "pop r9", "pop r8",
                "pop rbp", "pop rdi", "pop rsi",
                "pop rdx", "pop rcx", "pop rbx", "pop rax",
                "add rsp, 16",
                "iretq",
                in(reg) rsp,
                options(nostack)
            );
            unreachable!();
        }
    }
    // 没有别的线程——停 halt 等中断
    loop {
        core::arch::asm!("sti; hlt", options(nostack, preserves_flags));
    }
}

/// Cooperative yield（syscall 路径）。拿锁 + 切走。
pub unsafe fn ly_sched_yield_switch(cur_frame: u64) -> u64 {
    let _g = TCB_LOCK.lock();
    let cur_tid = current_tid();
    if cur_tid == 0 {
        // SMP idle path: no real thread to save; just pick a ready one.
        return pick_next_ready(0);
    }
    let cur_idx = TCB_TABLE.iter().position(|t| t.tid == cur_tid).unwrap_or(0);

    let cur_top = TCB_TABLE[cur_idx].stack_base
        + (TCB_TABLE[cur_idx].stack_pages as u64) * 4096;
    if !crate::security::stack_canary_check(cur_top, cur_tid) {
        serial::log("   security: STACK CANARY VIOLATION tid=");
        serial::log_u(cur_tid as u64);
        serial::log(", thread killed\r\n");
        TCB_TABLE[cur_idx].state = ThreadState::Zombie;
        return pick_next_ready(cur_idx);
    }

    TCB_TABLE[cur_idx].saved_rsp = cur_frame;
    TCB_TABLE[cur_idx].state = ThreadState::Ready;
    TCB_TABLE[cur_idx].home_cpu = u32::MAX;

    let rsp = pick_next_ready(cur_idx);
    if rsp != 0 {
        TOTAL_SWITCHES.fetch_add(1, Ordering::Relaxed);
        return rsp;
    }
    TCB_TABLE[cur_idx].state = ThreadState::Running;
    0
}

// ---------------------------------------------------------------------------
// Init + entry
// ---------------------------------------------------------------------------

extern "C" {
    #[link_name = "stack_bottom"]
    static ly_boot_stack_bottom: u8;
    #[link_name = "stack_top"]
    static ly_boot_stack_top: u8;
}

pub fn ly_sched_init() {
    let _g = TCB_LOCK.lock();
    unsafe {
        let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed) as u32;
        CURRENT_TID_PER_CPU[0].store(tid as usize, Ordering::Relaxed);

        let boot_top = &ly_boot_stack_top as *const u8 as u64;
        let boot_bottom = &ly_boot_stack_bottom as *const u8 as u64;
        TCB_TABLE[0] = Tcb {
            tid,
            name: "idle",
            state: ThreadState::Running,
            saved_rsp: 0,
            stack_base: boot_bottom,
            stack_pages: ((boot_top - boot_bottom) / 4096) as usize,
            entry: 0,
            caps: CapSet::all(),
            runtime_ticks: 0,
            pml4: 0,
            as_slot: usize::MAX,
            home_cpu: 0,
            aff_cpu: 0,
        };
        crate::security::stack_canary_arm(boot_top, tid);
    }
    // 先锁外再初始化 PIT（PIT init 不碰 TCB）
    capability::ly_cap_init();
    pit::ly_pit_init();

    serial::log("   sched: ready, threads=");
    serial::log_u(MAX_THREADS as u64);
    serial::log(" tick=100Hz\r\n");
}

pub fn ly_sched_run() -> ! {
    serial::log("   sched: entering preemptive loop\r\n");
    // dump 门控：进入调度循环后 SMP 验证 dump 才生效
    SMP_RUNNING.store(true, Ordering::Release);
    // BSP 进入 GS_BASE 设置
    crate::smp::cpu_local::bsp_init();
    // 注册 main 线程（tid=1）：boot 阶段 main 直接跑（无 TCB，current_tid()=0），
    // 一旦 SMP_RUNNING=true 后第一个 tick 抢占会把 cur_tid==0 当 idle 直接切走
    // 且不保存上下文 → main 永久丢失（ring3 selftest / play-tips 卡死根因）。
    // 此处把当前 rsp 登记为可恢复上下文，main 参与 round-robin。
    unsafe {
        let rsp: u64;
        core::arch::asm!("mov {}, rsp", out(reg) rsp);
        let idx = {
            let mut found = 1usize;
            for i in 1..MAX_THREADS {
                if TCB_TABLE[i].tid == 0 { found = i; break; }
            }
            found
        };
        // main 线程 tid 必须唯一（NEXT_TID 已分配 smp_worker/worker/shell；
        // 用 fetch_add 取下一个值，避免与已有 tid 冲突导致 position() 错位）
        let main_tid = NEXT_TID.fetch_add(1, Ordering::Relaxed) as u32;
        TCB_TABLE[idx] = Tcb {
            tid: main_tid, name: "main", state: ThreadState::Running,
            saved_rsp: rsp, stack_base: 0, stack_pages: 0,
            entry: 0, caps: CapSet::all(), runtime_ticks: 0,
            pml4: 0, as_slot: usize::MAX, home_cpu: u32::MAX,
            aff_cpu: u32::MAX,
        };
        CURRENT_TID_PER_CPU[0].store(main_tid as usize, Ordering::Relaxed);
        serial::log("   sched: registered main as tid=");
        serial::log_u(main_tid as u64);
        serial::log(" (boot stack, no canary)\r\n");
    }
    serial::log("   sched: main entering selftest\r\n");
    // [REDACTED: ring3 native-app selftest (exec loader, preinstalled apps)]
    unsafe { core::arch::asm!("sti"); }

    // 多核 BSOD 取证（可选 feature）：等 SMP dump（5s 后）完成，再在
    // main 线程（非中断上下文）故意触发 #PF，验证蓝屏 handler 在多核下
    // 仍能打印有意义的 CR2/CR3/RSP 现场。默认构建不触发。
    #[cfg(feature = "smp_bsod_test")]
    {
        while !smp_dump_done() {
            unsafe { core::arch::asm!("hlt", options(nostack, preserves_flags)); }
        }
        crate::serial::log(">> smp_bsod_test: deliberately triggering #PF (unmapped addr)\r\n");
        unsafe {
            let bad = core::ptr::read_volatile(0xFFFF_FFFF_FFFF_FFFFusize as *const u32);
            core::hint::black_box(bad);
        }
        crate::serial::log(">> smp_bsod_test: read returned (PF did not fire?)\r\n");
        // 防御：若 #PF 未触发，停在这里等待人工干预
        loop { unsafe { core::arch::asm!("hlt", options(nostack, preserves_flags)); } }
    }

    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

pub fn ly_sched_stats() {
    let cur_cpu = cpu_local::cpu_id();
        serial::log("   sched: cpu");
        serial::log_u(cur_cpu as u64);
        serial::log(" tid=");
        serial::log_u(current_tid() as u64);
        serial::log(" switches=");
        serial::log_u(TOTAL_SWITCHES.load(Ordering::Relaxed) as u64);
        serial::log(" ticks=");
        serial::log_u(pit::ly_pit_ticks());
        serial::log("\r\n");
}

pub fn ly_sched_current_tid() -> u32 { current_tid() }

pub fn ly_sched_thread_caps(tid: u32) -> capability::CapSet {
    if tid == 0 {
        return capability::CapSet::kernel_default();
    }
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid && TCB_TABLE[i].state != ThreadState::Zombie {
                return TCB_TABLE[i].caps;
            }
        }
    }
    capability::CapSet::empty()
}

pub fn ly_sched_revoke_cap(tid: u32, cap: u8) -> bool {
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                TCB_TABLE[i].caps.revoke(cap);
                return true;
            }
        }
    }
    false
}

pub fn ly_sched_kill(tid: u32) -> bool {
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid && TCB_TABLE[i].state != ThreadState::Zombie {
                TCB_TABLE[i].state = ThreadState::Zombie;
                return true;
            }
        }
    }
    false
}

pub fn ly_sched_stack_top(tid: u32) -> u64 {
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                return TCB_TABLE[i].stack_base + (TCB_TABLE[i].stack_pages as u64) * 4096;
            }
        }
    }
    0
}

pub fn ly_sched_thread_usage(tid: u32) -> u64 {
    let _g = TCB_LOCK.lock();
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                return TCB_TABLE[i].runtime_ticks;
            }
        }
    }
    0
}

pub fn ly_sched_list_count() -> usize {
    let _g = TCB_LOCK.lock();
    unsafe {
        let mut n = 0;
        for t in TCB_TABLE.iter() {
            if t.tid != 0 { n += 1; }
        }
        n
    }
}

pub fn ly_sched_list(out: &mut [(u32, &'static str, u8, u64); 16]) -> usize {
    let _g = TCB_LOCK.lock();
    unsafe {
        let mut n = 0usize;
        for i in 0..MAX_THREADS {
            let t = &TCB_TABLE[i];
            if t.tid != 0 {
                let st = match t.state {
                    ThreadState::Ready => 0u8,
                    ThreadState::Running => 1u8,
                    ThreadState::Zombie => 2u8,
                };
                out[n] = (t.tid, t.name, st, t.runtime_ticks);
                n += 1;
                if n >= out.len() { break; }
            }
        }
        n
    }
}
