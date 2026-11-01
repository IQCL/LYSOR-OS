//! Low-level context switching for x86_64.
//!
//! `switch_context(old_rsp: *mut u64, new_rsp: u64)`:
//!   - Pushes callee-saved regs (rbx, rbp, r12-r15) onto old stack
//!   - Saves old rsp into *old_rsp
//!   - Loads new rsp
//!   - Pops callee-saved regs
//!   - Returns to caller of the new context

core::arch::global_asm! {
    ".intel_syntax noprefix",

    // extern "C" fn switch_context(old_rsp: *mut u64, new_rsp: u64)
    ".global ly_switch_context",
    "ly_switch_context:",
    // Save callee-saved registers on current stack
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    // Save old stack pointer
    "mov [rdi], rsp",
    // Load new stack pointer
    "mov rsp, rsi",
    // Restore callee-saved registers
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",
}

/// Create an initial stack for a new kernel thread.
/// The fake stack looks like we just called into the thread from switch_context.
///
/// Layout (high -> low, growing down):
///   [ret addr -> thread_entry]
///   [r15=0, r14=0, r13=0, r12=0, rbx=0, rbp=0]
///
/// When switch_context pops these and `ret`s, it jumps to thread_entry.
pub fn ly_init_stack(stack_top: u64, entry: u64) -> u64 {
    let mut sp = stack_top;
    // Return address: when switch_context does `ret`, it jumps here
    sp -= 8;
    unsafe { core::ptr::write(sp as *mut u64, entry) };
    // Fake callee-saved regs (popped by switch_context)
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // rbp
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // rbx
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // r12
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // r13
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // r14
    sp -= 8; unsafe { core::ptr::write(sp as *mut u64, 0u64) }; // r15
    sp
}
