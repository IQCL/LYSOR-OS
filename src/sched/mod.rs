//! LYSOR OS - Process / Thread Scheduler (`ly_sched`).
//!
//! Preemptive round-robin scheduling via PIT timer interrupt.
//! Context switch is done by swapping the saved TrapFrame on the kernel stack
//! before `iretq` 鈥?the ISR epilogue naturally restores the next thread.

pub mod capability;
pub mod pit;

use crate::memory::physical;
use crate::serial;
use crate::idt::TrapFrame;
use capability::CapSet;

const MAX_THREADS: usize = 16;
const STACK_PAGES: usize = 4; // 16 KiB per kernel stack
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
    entry: u64,
    caps: CapSet,
    runtime_ticks: u64,
}

impl Tcb {
    const fn empty() -> Self {
        Tcb {
            tid: 0, name: "", state: ThreadState::Zombie,
            saved_rsp: 0, stack_base: 0, stack_pages: 0,
            entry: 0, caps: CapSet::empty(), runtime_ticks: 0,
        }
    }
}

static mut TCB_TABLE: [Tcb; MAX_THREADS] = [Tcb::empty(); MAX_THREADS];
static mut CURRENT_TID: u32 = 0;
static mut NEXT_TID: u32 = 1;
static mut TOTAL_SWITCHES: u64 = 0;

// ---------------------------------------------------------------------------
// Thread creation
// ---------------------------------------------------------------------------

pub fn ly_sched_create(name: &'static str, entry: extern "C" fn()) -> Option<u32> {
    unsafe {
        let mut slot = None;
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == 0 { slot = Some(i); break; }
        }
        let idx = slot?;

        // Allocate contiguous-ish stack frames
        let mut base = 0u64;
        for f in 0..STACK_PAGES {
            let frame = physical::alloc_frame()?;
            if f == 0 { base = frame; }
        }
        let stack_top = base + (STACK_PAGES as u64) * 4096;

        // Build a fake TrapFrame on the new thread's stack so that
        // the first `iretq` lands at `entry` with interrupts enabled.
        let rsp = build_fake_trapframe(stack_top, entry as u64);

        // Real stack canary: write a per-thread magic word just above the
        // usable stack; every context switch verifies it (security.rs).
        crate::security::stack_canary_arm(stack_top, NEXT_TID);

        let tid = NEXT_TID;
        NEXT_TID += 1;

        TCB_TABLE[idx] = Tcb {
            tid, name, state: ThreadState::Ready,
            saved_rsp: rsp, stack_base: base, stack_pages: STACK_PAGES,
            entry: entry as u64, caps: CapSet::all(), runtime_ticks: 0,
        };

        serial::log("   sched: created tid=");
        serial::log_u(tid as u64);
        serial::log(" \"");
        serial::log(name);
        serial::log("\"\r\n");
        Some(tid)
    }
}

/// Build a fake TrapFrame at the top of a fresh kernel stack.
/// Layout matches the ISR stub push order (high -> low):
///   ss, rsp, rflags, cs, rip, err, vector, rax, rbx, rcx, rdx,
///   rsi, rdi, rbp, r8..r15
unsafe fn build_fake_trapframe(stack_top: u64, entry: u64) -> u64 {
    // Reserve the top 8 bytes for the stack canary (stack_top-8): the fake
    // frame must not occupy that slot or the canary arm would clobber the
    // iretq ss selector (GP#13 on first switch). The whole frame is shifted
    // down 8 bytes; the frame layout is relative to saved_rsp, so iretq
    // still pops every field from the correct slot.
    let mut sp = stack_top - 8 - 176;
    // iretq pops: rip, cs, rflags, rsp, ss (in that order)
    sp -= 8; core::ptr::write(sp as *mut u64, 0x10);          // ss
    sp -= 8; core::ptr::write(sp as *mut u64, sp + 16);       // rsp (just above frame)
    sp -= 8; core::ptr::write(sp as *mut u64, 0x202);         // rflags (IF=1)
    sp -= 8; core::ptr::write(sp as *mut u64, 0x08);          // cs
    sp -= 8; core::ptr::write(sp as *mut u64, entry);          // rip
    // ISR stub pushes: error_code, vector
    sp -= 8; core::ptr::write(sp as *mut u64, 0);             // error_code
    sp -= 8; core::ptr::write(sp as *mut u64, 32);            // vector (timer)
    // ISR stub pushes GP regs: rax, rbx, rcx, rdx, rsi, rdi, rbp, r8..r15
    // Order pushed: rax first ... r15 last. TrapFrame has r15 first.
    for _ in 0..15 {
        sp -= 8;
        core::ptr::write(sp as *mut u64, 0);
    }
    // sp now points to r15 on the fake frame
    sp
}

pub fn ly_sched_exit() -> ! {
    unsafe {
        let tid = CURRENT_TID;
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                TCB_TABLE[i].state = ThreadState::Zombie;
                break;
            }
        }
        // Pick next ready thread and swap in its stack
        ly_sched_pick_next();
        unreachable!();
    }
}

pub fn ly_sched_reap() {
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].state == ThreadState::Zombie && TCB_TABLE[i].tid != 0 {
                for f in 0..TCB_TABLE[i].stack_pages {
                    physical::free_frame(TCB_TABLE[i].stack_base + (f as u64) * 4096);
                }
                TCB_TABLE[i].tid = 0;
                TCB_TABLE[i].name = "";
                TCB_TABLE[i].stack_base = 0;
                TCB_TABLE[i].stack_pages = 0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Context switch via TrapFrame swap
// ---------------------------------------------------------------------------

/// Called from the timer ISR. Returns the new rsp (pointing to a TrapFrame)
/// for the next thread, or 0 if no switch. The ISR stub swaps rsp on return.
#[no_mangle]
pub extern "C" fn ly_sched_timer_tick(frame: &TrapFrame) -> u64 {
    let ticks = pit::ly_pit_tick();

    unsafe {
        let current_rsp = frame as *const TrapFrame as u64;
        let cur_idx = TCB_TABLE.iter().position(|t| t.tid == CURRENT_TID).unwrap_or(0);

        // Stack canary check: a corrupted stack (overflow / clobber) must be
        // caught here, before the thread gets scheduled again.
        let cur_top = TCB_TABLE[cur_idx].stack_base
            + (TCB_TABLE[cur_idx].stack_pages as u64) * 4096;
        if !crate::security::stack_canary_check(cur_top, CURRENT_TID) {
            serial::log("   security: STACK CANARY VIOLATION tid=");
            serial::log_u(CURRENT_TID as u64);
            serial::log(", thread killed
\n");
            TCB_TABLE[cur_idx].state = ThreadState::Zombie;
            for step in 1..=MAX_THREADS {
                let idx = (cur_idx + step) % MAX_THREADS;
                if TCB_TABLE[idx].state == ThreadState::Ready {
                    TCB_TABLE[idx].state = ThreadState::Running;
                    CURRENT_TID = TCB_TABLE[idx].tid;
                    TOTAL_SWITCHES += 1;
                    return TCB_TABLE[idx].saved_rsp;
                }
            }
            return 0;
        }

        TCB_TABLE[cur_idx].saved_rsp = current_rsp;
        TCB_TABLE[cur_idx].runtime_ticks += 1;

        for step in 1..=MAX_THREADS {
            let idx = (cur_idx + step) % MAX_THREADS;
            if TCB_TABLE[idx].state == ThreadState::Ready {
                TCB_TABLE[cur_idx].state = ThreadState::Ready;
                TCB_TABLE[idx].state = ThreadState::Running;
                CURRENT_TID = TCB_TABLE[idx].tid;
                TOTAL_SWITCHES += 1;
                return TCB_TABLE[idx].saved_rsp;
            }
        }
    }

    if ticks % 50 == 0 {
        ly_sched_reap();
    }
    0
}

unsafe fn ly_sched_pick_next() {
    let cur_idx = TCB_TABLE.iter().position(|t| t.tid == CURRENT_TID).unwrap_or(0);
    for step in 1..=MAX_THREADS {
        let idx = (cur_idx + step) % MAX_THREADS;
        if TCB_TABLE[idx].state == ThreadState::Ready {
            TCB_TABLE[idx].state = ThreadState::Running;
            CURRENT_TID = TCB_TABLE[idx].tid;
            core::arch::asm!(
                "mov rsp, {}",
                in(reg) TCB_TABLE[idx].saved_rsp,
                options(nostack)
            );
            unreachable!();
        }
    }
}

/// Cooperative yield (SYS_SCHED_YIELD): save the current thread's TrapFrame
/// rsp as its resume point, mark it Ready, and switch to the next Ready
/// thread. Returns the next thread's saved rsp (0 = no other thread ready).
pub unsafe fn ly_sched_yield_switch(cur_frame: u64) -> u64 {
    let cur_idx = TCB_TABLE.iter().position(|t| t.tid == CURRENT_TID).unwrap_or(0);

    // Real stack canary verification on the cooperative switch path too.
    let cur_top = TCB_TABLE[cur_idx].stack_base
        + (TCB_TABLE[cur_idx].stack_pages as u64) * 4096;
    if !crate::security::stack_canary_check(cur_top, CURRENT_TID) {
        serial::log("   security: STACK CANARY VIOLATION tid=");
        serial::log_u(CURRENT_TID as u64);
        serial::log(", thread killed\r\n");
        TCB_TABLE[cur_idx].state = ThreadState::Zombie;
        for step in 1..=MAX_THREADS {
            let idx = (cur_idx + step) % MAX_THREADS;
            if TCB_TABLE[idx].state == ThreadState::Ready {
                TCB_TABLE[idx].state = ThreadState::Running;
                CURRENT_TID = TCB_TABLE[idx].tid;
                TOTAL_SWITCHES += 1;
                return TCB_TABLE[idx].saved_rsp;
            }
        }
        return 0;
    }

    TCB_TABLE[cur_idx].saved_rsp = cur_frame;
    TCB_TABLE[cur_idx].state = ThreadState::Ready;
    for step in 1..=MAX_THREADS {
        let idx = (cur_idx + step) % MAX_THREADS;
        if TCB_TABLE[idx].state == ThreadState::Ready {
            TCB_TABLE[idx].state = ThreadState::Running;
            CURRENT_TID = TCB_TABLE[idx].tid;
            TOTAL_SWITCHES += 1;
            return TCB_TABLE[idx].saved_rsp;
        }
    }
    TCB_TABLE[cur_idx].state = ThreadState::Running;
    0
}

// ---------------------------------------------------------------------------
// Init + entry
// ---------------------------------------------------------------------------

/// The kernel boot stack (BSS in main.rs global asm, 128 KiB).
extern "C" {
    #[link_name = "stack_bottom"]
    static ly_boot_stack_bottom: u8;
    #[link_name = "stack_top"]
    static ly_boot_stack_top: u8;
}

pub fn current_tid() -> u32 {
    unsafe { CURRENT_TID }
}

pub fn ly_sched_init() {
    unsafe {
        CURRENT_TID = NEXT_TID;
        NEXT_TID += 1;
        // The boot thread runs on the kernel boot stack in BSS, not on a
        // scheduler-allocated stack; record the real bounds so the canary
        // check computes a valid stack_top instead of 0.
        let boot_top = &ly_boot_stack_top as *const u8 as u64;
        let boot_bottom = &ly_boot_stack_bottom as *const u8 as u64;
        TCB_TABLE[0] = Tcb {
            tid: CURRENT_TID,
            name: "idle",
            state: ThreadState::Running,
            saved_rsp: 0,
            stack_base: boot_bottom,
            stack_pages: ((boot_top - boot_bottom) / 4096) as usize,
            entry: 0,
            caps: CapSet::all(),
            runtime_ticks: 0,
        };
        crate::security::stack_canary_arm(boot_top, CURRENT_TID);

        capability::ly_cap_init();
        pit::ly_pit_init();

        serial::log("   sched: ready, threads=");
        serial::log_u(MAX_THREADS as u64);
        serial::log(" tick=100Hz\r\n");
    }
}

pub fn ly_sched_run() -> ! {
    serial::log("   sched: entering preemptive loop\r\n");
    unsafe {
        core::arch::asm!("sti");
    }
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

pub fn ly_sched_stats() {
    unsafe {
        serial::log("   sched: switches=");
        serial::log_u(TOTAL_SWITCHES);
        serial::log(" ticks=");
        serial::log_u(pit::ly_pit_ticks());
        serial::log(" cur=tid");
        serial::log_u(CURRENT_TID as u64);
        serial::log("\r\n");
    }
}



/// Get current thread tid.
pub fn ly_sched_current_tid() -> u32 { unsafe { CURRENT_TID } }

/// Get a thread's capability set (real per-thread caps).
/// Thread 0 is the boot/TCB context and holds kernel-default capabilities.
pub fn ly_sched_thread_caps(tid: u32) -> capability::CapSet {
    if tid == 0 {
        return capability::CapSet::kernel_default();
    }
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid && TCB_TABLE[i].state != ThreadState::Zombie {
                return TCB_TABLE[i].caps;
            }
        }
    }
    capability::CapSet::empty()
}

/// Revoke a capability from a live thread.
pub fn ly_sched_revoke_cap(tid: u32, cap: u8) -> bool {
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

/// Kill a thread: mark it Zombie so the scheduler skips it and reaper
/// reclaims its stacks. Real termination primitive for services/apps.
pub fn ly_sched_kill(tid: u32) -> bool {
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

/// Top of a thread's kernel stack (where the canary lives).
pub fn ly_sched_stack_top(tid: u32) -> u64 {
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                return TCB_TABLE[i].stack_base + (TCB_TABLE[i].stack_pages as u64) * 4096;
            }
        }
    }
    0
}

/// CPU time for a thread (ticks), for the task manager.
pub fn ly_sched_thread_usage(tid: u32) -> u64 {
    unsafe {
        for i in 0..MAX_THREADS {
            if TCB_TABLE[i].tid == tid {
                return TCB_TABLE[i].runtime_ticks;
            }
        }
    }
    0
}

/// Enumerate threads: (tid, name, state, runtime). Returns count.
/// Number of live threads (for list-style apps).
pub fn ly_sched_list_count() -> usize {
    unsafe {
        let mut n = 0;
        for t in TCB_TABLE.iter() {
            if t.tid != 0 { n += 1; }
        }
        n
    }
}

pub fn ly_sched_list(out: &mut [(u32, &'static str, u8, u64); 16]) -> usize {
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

