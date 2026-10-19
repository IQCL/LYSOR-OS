//! IDT (Interrupt Descriptor Table) for LYSOR OS.

use core::arch::asm;
use core::mem;

use crate::pic;
use crate::serial;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0, selector: 0, ist: 0, type_attr: 0,
            offset_mid: 0, offset_high: 0, zero: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

#[repr(C, align(16))]
struct IdtTable {
    entries: [IdtEntry; 256],
}

static mut IDT: IdtTable = IdtTable {
    entries: [IdtEntry::missing(); 256],
};

extern "C" {
    static isr_stub_table: [usize; 256];
}

unsafe fn set_gate(entry: &mut IdtEntry, handler: usize, ist: u8) {
    entry.offset_low = (handler & 0xFFFF) as u16;
    entry.selector = crate::gdt::KCODE_SEL;
    entry.ist = ist & 0x7;
    entry.type_attr = 0x8E;
    entry.offset_mid = ((handler >> 16) & 0xFFFF) as u16;
    entry.offset_high = (handler >> 32) as u32;
    entry.zero = 0;
}

pub fn init_idt() {
    unsafe {
        for i in 0..256 {
            let handler = core::ptr::read_volatile(isr_stub_table.as_ptr().add(i));
            set_gate(&mut IDT.entries[i], handler, 0);
        }
        let ptr = IdtPtr {
            limit: (mem::size_of::<IdtTable>() - 1) as u16,
            base: &raw const IDT as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack));
    }
    serial::log("   IDT: 256 gates installed\r\n");
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapFrame {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9: u64,  pub r8: u64,
    pub rbp: u64, pub rdi: u64, pub rsi: u64, pub rdx: u64,
    pub rcx: u64, pub rbx: u64, pub rax: u64,
    pub vector: u64, pub error_code: u64,
    pub rip: u64, pub cs: u64, pub rflags: u64,
    pub rsp: u64, pub ss: u64,
}

/// Unified ISR dispatcher. Returns a new rsp to switch to (0 = no switch).
/// Vector 128 (INT 0x80) is the syscall entry: dispatched to the syscall
/// layer with the full TrapFrame so handlers can patch rax and (for yield)
/// swap to another thread's saved frame.
extern "C" fn ly_isr_handler(frame: &mut TrapFrame) -> u64 {
    let vec = frame.vector;

    if vec == 128 {
        return crate::syscall::ly_syscall_entry(frame as *mut TrapFrame);
    }

    serial::log("\r\n*** vector=");
    serial::log_u(vec);
    if frame.error_code != 0 {
        serial::log(" err=0x");
        serial::log_hex(frame.error_code);
    }
    serial::log(" rip=0x");
    serial::log_hex(frame.rip);
    serial::log("\r\n");

    if vec == 0x0E {
        let cr2: u64;
        unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
        serial::log("   CR2=0x");
        serial::log_hex(cr2);
        serial::log("\r\n");
    }

    if vec >= 32 && vec < 48 {
        unsafe { pic::pic_eoi((vec - 32) as u8) };
    }

    if vec < 32 {
        // Public microkernel build: no kernel components (KC) exist, so every
        // CPU exception is a fatal kernel fault. The full (private) build
        // attributes exceptions inside a KC fault domain to that component
        // and resumes at its recovery point instead of halting.
        serial::log("   fatal CPU exception - halting, tid=");
        serial::log_u(crate::sched::current_tid() as u64);
        serial::log("\r\n");
        loop {
            unsafe { asm!("cli; hlt") };
        }
    }
    0
}

core::arch::global_asm! {
    ".intel_syntax noprefix",

    ".macro STUB_NOERR vec",
    ".global isr_stub\\vec",
    "isr_stub\\vec:",
    "push 0",
    "push \\vec",
    "jmp isr_common",
    ".endm",

    ".macro STUB_ERR vec",
    ".global isr_stub\\vec",
    "isr_stub\\vec:",
    "push \\vec",
    "jmp isr_common",
    ".endm",

    "STUB_NOERR 0",
    "STUB_NOERR 1",
    "STUB_NOERR 2",
    "STUB_NOERR 3",
    "STUB_NOERR 4",
    "STUB_NOERR 5",
    "STUB_NOERR 6",
    "STUB_NOERR 7",
    "STUB_ERR   8",
    "STUB_NOERR 9",
    "STUB_ERR   10",
    "STUB_ERR   11",
    "STUB_ERR   12",
    "STUB_ERR   13",
    "STUB_ERR   14",
    "STUB_NOERR 15",
    "STUB_NOERR 16",
    "STUB_ERR   17",
    "STUB_NOERR 18",
    "STUB_NOERR 19",
    "STUB_NOERR 20",
    "STUB_ERR   21",
    "STUB_NOERR 22",
    "STUB_NOERR 23",
    "STUB_NOERR 24",
    "STUB_NOERR 25",
    "STUB_NOERR 26",
    "STUB_NOERR 27",
    "STUB_NOERR 28",
    "STUB_ERR   29",
    "STUB_ERR   30",
    "STUB_NOERR 31",

    "STUB_NOERR 33",
    "STUB_NOERR 34",
    "STUB_NOERR 35",
    "STUB_NOERR 36",
    "STUB_NOERR 37",
    "STUB_NOERR 38",
    "STUB_NOERR 39",
    "STUB_NOERR 40",
    "STUB_NOERR 41",
    "STUB_NOERR 42",
    "STUB_NOERR 43",
    "STUB_NOERR 44",
    "STUB_NOERR 45",
    "STUB_NOERR 46",
    "STUB_NOERR 47",

    // ---- Timer stub (vector 32): calls scheduler, swaps rsp, sends EOI ----
    ".global timer_stub32",
    "timer_stub32:",
    "push 0",
    "push 32",
    "push rax", "push rbx", "push rcx", "push rdx",
    "push rsi", "push rdi", "push rbp",
    "push r8", "push r9", "push r10", "push r11",
    "push r12", "push r13", "push r14", "push r15",
    "mov rdi, rsp",
    "cld",
    "call {ly_sched_timer_tick}",
    // rax = new rsp to switch to (0 = no switch)
    "test rax, rax",
    "jz 1f",
    "mov rsp, rax",
    "1:",
    // EOI to PIC
    "mov al, 0x20",
    "out 0x20, al",
    "pop r15", "pop r14", "pop r13", "pop r12",
    "pop r11", "pop r10", "pop r9", "pop r8",
    "pop rbp", "pop rdi", "pop rsi",
    "pop rdx", "pop rcx", "pop rbx", "pop rax",
    "add rsp, 16",
    "iretq",

    "STUB_NOERR 128",
    "STUB_NOERR 129",
    "STUB_NOERR 130",

    "isr_common:",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rdi, rsp",
    "cld",
    "call {ly_isr_handler}",
    // rax = new rsp to switch to (0 = no switch, e.g. cooperative yield)
    "test rax, rax",
    "jz 1f",
    "mov rsp, rax",
    "1:",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "add rsp, 16",
    "iretq",

    ".section .rodata",
    ".align 16",
    ".global isr_stub_table",
    "isr_stub_table:",
    ".quad isr_stub0",
    ".quad isr_stub1",
    ".quad isr_stub2",
    ".quad isr_stub3",
    ".quad isr_stub4",
    ".quad isr_stub5",
    ".quad isr_stub6",
    ".quad isr_stub7",
    ".quad isr_stub8",
    ".quad isr_stub9",
    ".quad isr_stub10",
    ".quad isr_stub11",
    ".quad isr_stub12",
    ".quad isr_stub13",
    ".quad isr_stub14",
    ".quad isr_stub15",
    ".quad isr_stub16",
    ".quad isr_stub17",
    ".quad isr_stub18",
    ".quad isr_stub19",
    ".quad isr_stub20",
    ".quad isr_stub21",
    ".quad isr_stub22",
    ".quad isr_stub23",
    ".quad isr_stub24",
    ".quad isr_stub25",
    ".quad isr_stub26",
    ".quad isr_stub27",
    ".quad isr_stub28",
    ".quad isr_stub29",
    ".quad isr_stub30",
    ".quad isr_stub31",
    ".quad timer_stub32",
    ".quad isr_stub33",
    ".quad isr_stub34",
    ".quad isr_stub35",
    ".quad isr_stub36",
    ".quad isr_stub37",
    ".quad isr_stub38",
    ".quad isr_stub39",
    ".quad isr_stub40",
    ".quad isr_stub41",
    ".quad isr_stub42",
    ".quad isr_stub43",
    ".quad isr_stub44",
    ".quad isr_stub45",
    ".quad isr_stub46",
    ".quad isr_stub47",
    ".rept 80",
    ".quad isr_stub128",
    ".endr",
    ".quad isr_stub128",
    ".quad isr_stub129",
    ".quad isr_stub130",
    ".rept 125",
    ".quad isr_stub128",
    ".endr",

    ly_isr_handler = sym ly_isr_handler,
    ly_sched_timer_tick = sym crate::sched::ly_sched_timer_tick,
}
