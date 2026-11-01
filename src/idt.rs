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
        // The syscall gate (vector 128) must be callable from ring 3:
        // 0xEE = present | DPL3 | 64-bit interrupt gate.
        IDT.entries[128].type_attr = 0xEF;
    }
    serial::log("   IDT: 256 gates installed\r\n");
}

/// AP 调用：加载与 BSP 共享的同一张 IDT。
pub fn ap_load_idt() {
    unsafe {
        let ptr = IdtPtr {
            limit: (mem::size_of::<IdtTable>() - 1) as u16,
            base: &raw const IDT as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack));
    }
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
    // ring3 异常标记（cs 低 2 位 = 3 → 来自用户态）
    if frame.cs & 3 == 3 {
        serial::log(" RING3");
    }
    if frame.error_code != 0 {
        serial::log(" err=0x");
        serial::log_hex(frame.error_code);
    }
    serial::log(" rip=0x");
    serial::log_hex(frame.rip);
    serial::log(" cs=0x");
    serial::log_hex(frame.cs as u64);
    serial::log("\r\n");

    if vec == 0x0E {
        let cr2: u64;
        unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
        serial::log("   CR2=0x");
        serial::log_hex(cr2);
        serial::log("\r\n");
    }

    // EOI dispatch：PIC 32..47 走 8259，LAPIC 0xF0..0xFE 走 MMIO EOI，
    // 其他（CPU exception）不需要 EOI。
    if vec >= 32 && vec < 48 {
        unsafe { pic::pic_eoi((vec - 32) as u8) };
    } else if vec >= 0xF0 && vec != 0xFF {
        // LAPIC Error (0xFE) 读 ESR 记录下错误码
        if vec == 0xFE {
            let esr = crate::smp::lapic::read_esr();
            serial::log("   [LAPIC] Error Status Register=0x");
            serial::log_hex(esr as u64);
            serial::log("\r\n");
        }
        crate::smp::lapic::eoi();
    }

    if vec < 32 {
        // [REDACTED: KC fault dispatch (kcomponent) — non-TCB component model]
        serial::log("   fatal CPU exception - blue screen, tid=");
        serial::log_u(crate::sched::current_tid() as u64);
        serial::log("\r\n");
        unsafe { bsod(vec, frame.error_code, frame.rip) }
    }
    0
}

/// 8x8 bitmap glyphs for the blue screen (":( " + hex digits 0-9A-F).
#[allow(dead_code)] // 字形宽常量，蓝屏渲染排版预留
const BSOD_GLYPH_W: usize = 8;
const BSOD_GLYPHS: [u8; 19 * 8] = [
    // ':' 0
    0, 0, 24, 24, 0, 0, 24, 24,
    // '(' 1
    0, 24, 60, 66, 66, 66, 60, 24,
    // ')' 2
    0, 24, 60, 66, 66, 66, 60, 24,
    // ' ' 3
    0, 0, 0, 0, 0, 0, 0, 0,
    // '0' 4
    60, 66, 66, 66, 66, 66, 66, 60,
    // '1' 5
    24, 56, 24, 24, 24, 24, 24, 60,
    // '2' 6
    60, 66, 6, 12, 48, 96, 66, 126,
    // '3' 7
    126, 66, 4, 8, 4, 2, 66, 60,
    // '4' 8
    12, 28, 44, 76, 254, 12, 12, 12,
    // '5' 9
    254, 64, 64, 124, 2, 2, 66, 60,
    // '6' 10
    60, 66, 64, 124, 66, 66, 66, 60,
    // '7' 11
    126, 66, 4, 8, 16, 16, 16, 16,
    // '8' 12
    60, 66, 66, 60, 66, 66, 66, 60,
    // '9' 13
    60, 66, 66, 126, 2, 2, 66, 60,
    // 'A' 14
    60, 66, 66, 126, 66, 66, 66, 66,
    // 'B' 15
    124, 66, 66, 124, 66, 66, 66, 124,
    // 'C' 16
    60, 66, 64, 64, 64, 64, 66, 60,
    // 'D' 17
    124, 66, 66, 66, 66, 66, 66, 124,
    // 'E' 18
    126, 64, 64, 120, 64, 64, 64, 126,
];

/// Blue screen of death: fill blue, draw ":(" + vector/error hex, run a
/// recovery progress bar for ~4 s, then reset via the 8042 keyboard controller.
unsafe fn bsod(vec: u64, err: u64, rip: u64) -> ! {
    // Forensic dump on serial before painting the screen: CR2 (faulting
    // address), CR3 (address space), RSP and a window of stack words. This
    // is a permanent diagnostic aid — every fatal fault leaves a serial
    // record of where it happened and in which address space.
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
    let cr2: u64;
    core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
    let rsp: u64;
    core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack));
    serial::log("\r\n[BSOD] vec=");
    serial::log_u(vec);
    serial::log(" err=");
    serial::log_u(err);
    serial::log(" rip=");
    serial::log_hex(rip);
    serial::log(" cr2=");
    serial::log_hex(cr2);
    serial::log(" cr3=");
    serial::log_hex(cr3);
    serial::log(" rsp=");
    serial::log_hex(rsp);
    serial::log("\r\n");
    // Find the thread's kernel stack bounds from the faulting rsp.
    for i in 0..14u64 {
        let v = *((rsp + i * 8) as *const u64);
        serial::log("  stk+");
        serial::log_u(i * 8);
        serial::log("=0x");
        serial::log_hex(v);
        serial::log("\r\n");
    }
    // [REDACTED: framebuffer window base/size (gfx KC, non-TCB)]
    let fb: *mut u32 = core::ptr::null_mut();
    let fw: usize = 0;
    let fh: usize = 0;
    let blue = 0xFF0078D7u32;
    let white = 0xFFFFFFFFu32;
    for i in 0..fw.saturating_mul(fh).min(4_000_000) {
        fb.add(i).write_volatile(blue);
    }
    // Draw a glyph row at (gx, gy) in white, glyph index list.
    let draw_row = |glyphs: &[usize], gx: usize, gy: usize| {
        for (gi, &g) in glyphs.iter().enumerate() {
            let bx = gx + gi * 10;
            for row in 0..8 {
                let bits = BSOD_GLYPHS[g * 8 + row];
                for col in 0..8 {
                    if bits & (0x80 >> col) != 0 {
                        let px = bx + col;
                        let py = gy + row;
                        if px < fw && py < fh {
                            fb.add(py * fw + px).write_volatile(white);
                        }
                    }
                }
            }
        }
    };
    let cx = fw / 2 - 20;
    let cy = fh / 3;
    draw_row(&[0, 1, 3], cx, cy); // ":("
    // Hex digits for vector / error / rip.
    let hex = |mut v: u64| -> [usize; 16] {
        let mut out = [3usize; 16]; // spaces
        for i in (0..16).rev() {
            let d = (v & 0xF) as usize;
            out[i] = if d < 10 { 4 + d } else { 10 + d - 10 };
            v >>= 4;
        }
        out
    };
    let hv = hex(vec);
    let mut head = [3usize; 8];
    for i in 0..8 { head[i] = hv[i]; }
    draw_row(&head, fw / 2 - 60, cy + 24);
    let _ = (err, rip);
    // Recovery progress bar: animate 0 -> full, ~4 s, then reset.
    let bar_w = (fw / 2) as i64;
    let bar_x = (fw / 4) as i64;
    let bar_y = (fh * 2 / 3) as i64;
    for step in 0..40 {
        let fill = bar_w * step / 40;
        for y in 0..14i64 {
            for x in 0..bar_w {
                let px = bar_x + x;
                let py = bar_y + y;
                if px < fw as i64 && py < fh as i64 {
                    let c = if x < fill { 0xFFF7F7F7u32 } else { 0xFF1F5FA8u32 };
                    fb.add(py as usize * fw + px as usize).write_volatile(c);
                }
            }
        }
        for _ in 0..40_000_000u32 {
            core::hint::spin_loop();
        }
    }
    // 8042 reset (0x64 / 0xFE); fall back to halt if the controller is dead.
    let p64 = 0x64u16;
    let p60 = 0x60u16;
    for _ in 0..4 {
        core::arch::asm!("out dx, al", in("dx") p64, in("al") 0xFEu8, options(nomem, nostack, preserves_flags));
        core::arch::asm!("out dx, al", in("dx") p60, in("al") 0xFEu8, options(nomem, nostack, preserves_flags));
    }
    loop {
        core::arch::asm!("cli; hlt");
    }
}

core::arch::global_asm! {

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

    // ---- Timer 路径（PIT vector 32 + LAPIC Timer vector 0xFD + BSP→AP
    //      调度 IPI vector 0xF0 共用 timer_common 主体）----
    ".global timer_stub32",
    "timer_stub32:",
    "push 0",
    "push 32",
    "jmp timer_common",

    // BSP 的 PIT tick 用固定 IPI 唤醒 AP（vector 0xF0）。
    // QEMU TCG 下 AP 的 LAPIC timer 在 hlt 时冻结不触发，IPI 唤醒可靠。
    ".global ap_ipi_stub_F0",
    "ap_ipi_stub_F0:",
    "push 0",
    "push 0xF0",
    "jmp timer_common",

    ".global lapic_timer_stub_FD",
    "lapic_timer_stub_FD:",
    "push 0",
    "push 0xFD",
    "jmp timer_common",

    "timer_common:",
    "push rax", "push rbx", "push rcx", "push rdx",
    "push rsi", "push rdi", "push rbp",
    "push r8", "push r9", "push r10", "push r11",
    "push r12", "push r13", "push r14", "push r15",
    "mov rdi, rsp",
    "cld",
    // EOI 在 Rust 侧完成（ly_sched_timer_tick 用 frame.vector 判定
    // LAPIC/PIC，切换前 ACK，避免切换栈后读错 vector）
    "call {ly_sched_timer_tick}",
    // rax = new rsp to switch to (0 = no switch)
    "test rax, rax",
    "jz 1f",
    "mov rsp, rax",
    "1:",
    "pop r15", "pop r14", "pop r13", "pop r12",
    "pop r11", "pop r10", "pop r9", "pop r8",
    "pop rbp", "pop rdi", "pop rsi",
    "pop rdx", "pop rcx", "pop rbx", "pop rax",
    "add rsp, 16",
    "iretq",

    // LAPIC Error (0xFE)：走 isr_common，Rust handler 里 EOI 发给 LAPIC
    ".global lapic_error_stub_FE",
    "lapic_error_stub_FE:",
    "push 0",
    "push 0xFE",
    "jmp isr_common",

    // LAPIC Spurious (0xFF)：不调 handler，不发 EOI
    ".global lapic_spurious",
    "lapic_spurious:",
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
    ".rept 109",
    ".quad isr_stub128",
    ".endr",
    ".quad ap_ipi_stub_F0",     // 240 (0xF0) BSP→AP 调度 IPI
    ".rept 12",
    ".quad isr_stub128",
    ".endr",
    ".quad lapic_timer_stub_FD",    // 253 (0xFD) LAPIC Timer
    ".quad lapic_error_stub_FE",    // 254 (0xFE) LAPIC Error
    ".quad lapic_spurious",         // 255 (0xFF) LAPIC Spurious

    ly_isr_handler = sym ly_isr_handler,
    ly_sched_timer_tick = sym crate::sched::ly_sched_timer_tick,
}
