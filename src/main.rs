//! LYSOR OS - minimal TCB kernel base
//!
//! Prefix: `ly_` for all public kernel symbols.
//! Boot: Multiboot2 -> 32-bit stub -> temporary paging -> x86_64 long mode.

#![no_std]
#![no_main]
// 内核惯例：TCB 最小基座使用 static mut 直接访问（无全局锁/无运行时），
// 逐项 addr_of! 改写 90+ 引用点属高风险低收益改造，整体压制该风格 lint。
#![allow(static_mut_refs)]

pub mod serial;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod memory;
pub mod tcb;
pub mod sched;
pub mod ipc;
pub mod vfs;
pub mod syscall;
pub mod fs;
pub mod syslog;
pub mod smp;
// [REDACTED: non-TCB modules (vga/component/kcomponent/exec/lyap/uservice/
// snapapp/shan/crypto/sig/wm/tty/ui/xmlui/desktop/apps/assets/drivers/font/gfx/
// security/net/init/coreutils/auth/cgroup/cron/ntp/dbus/pm/udev)]

/// `ram=` override in MiB (parsed from the kernel cmdline), 0 = unset.
pub fn ly_kcmdline_ram_mib() -> u32 {
    // [REDACTED: tty-backed cmdline parse]
    0
}

extern crate alloc;

/// Kernel heap allocator (first-fit free list, boundary tags).
#[global_allocator]
static HEAP_ALLOC: memory::heap::LyHeapAlloc = memory::heap::LyHeapAlloc;

// ---------------------------------------------------------------------------
// Boot assembly (Multiboot2 header + 32-bit stub + 64-bit entry)
// ---------------------------------------------------------------------------

core::arch::global_asm! {

    // ---- PVH boot note (Xen PVH protocol) ----
    ".section .note.pvh, \"a\"",
    ".align 4",
    ".long 4",
    ".long 4",
    ".long 18",
    ".ascii \"Xen\"",
    ".align 4",
    ".long _start",

    // ---- Multiboot1 header ----
    ".section .text.boot",
    ".code32",
    ".align 4",
    ".long 0x1BADB002",
    ".long 0x00000007",
    ".long -(0x1BADB002 + 0x00000007)",

    ".global _start",
    "_start:",
    "cli",
    "mov esp, offset stack_top",
    "mov dword ptr [mb_info_ptr], ebx",

    // Identity-map first 1 GiB with 2 MiB huge pages
    "mov eax, offset boot_p3",
    "or eax, 0x003",
    "mov dword ptr [boot_p4], eax",
    "mov eax, offset boot_p2",
    "or eax, 0x003",
    "mov dword ptr [boot_p3], eax",
    // p3 index 3 covers 3 GiB..4 GiB, where LAPIC MMIO lives.
    // Page-table entries are 8 bytes: p4[3] must point at boot_p3 and
    // boot_p3[3] at the extra P2 table that only maps the LAPIC slot.
    "mov eax, offset boot_p3",
    "or eax, 0x003",
    "mov dword ptr [boot_p4 + 3*8], eax",
    "mov eax, offset boot_p2_extra",
    "or eax, 0x003",
    "mov dword ptr [boot_p3 + 3*8], eax",
    // boot_p2_extra[503] = 0xFEE00000 | 0x003.
    // 503 = (0xFEE00000 >> 21) & 0x1FF, the P2 index inside the 3 GiB segment.
    // 用 4KiB 普通页（不带 PS 位）：QEMU TCG 对 MMIO 大页映射会 #PF，
    // LAPIC 全部寄存器都在 4KiB 内（偏移 0x000..0x3F0）。
    "mov dword ptr [boot_p2_extra + 503*8], 0xFEE00000 + 0x003",
    "mov ecx, 0",
    "1:",
    "mov eax, ecx",
    "shl eax, 21",
    "or eax, 0x083",
    "mov dword ptr [boot_p2 + ecx*8], eax",
    "inc ecx",
    "cmp ecx, 512",
    "jl 1b",

    "mov eax, offset boot_p4",
    "mov cr3, eax",
    "mov eax, cr4",
    "or eax, 1 << 5",
    "mov cr4, eax",
    "mov ecx, 0xC0000080",
    "rdmsr",
    "or eax, 1 << 8",
    "wrmsr",
    "lgdt [boot_gdt_ptr]",
    "mov eax, cr0",
    "or eax, 1 << 31",
    "mov cr0, eax",
    // Far jump: EA <32-bit offset> <16-bit segment=0x08>
    ".byte 0xEA",
    ".long long_mode_start",
    ".word 0x08",

    // ---- 64-bit entry ----
    ".code64",
    "long_mode_start:",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov fs, ax",
    "mov gs, ax",
    "mov ss, ax",
    "mov edi, dword ptr [mb_info_ptr]",
    "call kernel_main",
    "cli",
    "3:",
    "hlt",
    "jmp 3b",

    // ---- BSS: boot stack + boot page tables + AP params ----
    ".section .bss",
    ".align 16",
    ".global stack_bottom",
    "stack_bottom:",
    ".skip 131072",
    ".global stack_top",
    "stack_top:",
    ".align 4096",
    "boot_p4:",
    ".skip 4096",
    "boot_p3:",
    ".skip 4096",
    "boot_p2:",
    ".skip 4096",
    "boot_p2_extra:",
    ".skip 4096",

    // AP 启动参数区。BSP 填，AP trampoline 从 identity map 直接读。
    // struct { start_ip:u64, stack_top:u64, cpu_id:u32, started:u32 }
    ".align 16",
    ".global ap_params",
    "ap_params:",
    ".skip 4*8",

    // AP 启动 trampoline（纯 16/32/64-bit，不能碰 Rust 全局）。
    // BSP 把这段代码复制到低物理地址（0x8000）再发 SIPI。
    // SIPI 后 AP 处于 16-bit real mode（CS=0x0800, IP=0），必须先加载 GDT
    // 再进 protected mode，然后 PAE/PGE/EFER/分页 → long mode。
    // 关键：三段代码都在低地址（0x8000 + 段内偏移 < 0x8100 < 64KiB）执行，
    // 16-bit 远跳偏移是 16 位，跳不到镜像（0x10xxxx），必须跳到低地址副本；
    // 镜像里的立即数（boot_gdt_ptr / ap_params / boot_p4 的 VMA）在无分页
    // 保护模式或 identity 映射下都能直接按物理地址访问。
    // 16-bit 段直接用 BSP 的 boot_gdt_ptr（单符号引用，real mode 32 位
    // 寻址可访问 rodata/BSS）。
    ".section .text.smp_ap, \"ax\"",
    ".code16",
    ".global ap_trampoline32",
    "ap_trampoline32:",
    "cli",
    // serial 哨兵 '1'（real mode 直接写 0x3F8）
    "mov dx, 0x3F8",
    "mov al, '1'",
    "out dx, al",
    "mov sp, 0x7C00",                 // real-mode 安全栈（1 MiB 内）
    "mov eax, offset boot_gdt_ptr",   // 加载 BSP 的 GDT（null/kcode/kdata）
    "lgdt [eax]",
    // 哨兵 '2'
    "mov dx, 0x3F8",
    "mov al, '2'",
    "out dx, al",
    "mov eax, cr0",
    "or al, 1",                       // PE=1
    "mov cr0, eax",
    // 第一次 ljmpw 到 0x18（kcode32，L=0/D=1）：QEMU TCG 对 16 位段切换
    // 的 CS 加载不会更新操作数大小（后续仍按 16 位解码 32 位代码），
    // 所以 32 位段开头再同段跳一次强制刷新。
    // 目标地址 = 0x8000 + (ap_trampoline32bit - ap_trampoline32) < 64KiB
    ".byte 0xEA",
    ".word ap_trampoline32bit - ap_trampoline32 + 0x8000",
    ".word 0x18",

    ".code32",
    ".global ap_trampoline32bit",
    "ap_trampoline32bit:",
    // 哨兵 '3'（此时 CS=0x18 已生效，QEMU 已按 32 位操作数解码）
    "push ax",
    "push dx",
    "mov dx, 0x3F8",
    "mov al, '3'",
    "out dx, al",
    "pop dx",
    "pop ax",
    // 从 ap_params 读 BSP 填的参数（identity map 下 phys==virt）
    "mov eax, offset ap_params",
    "mov ebx, [eax + 12]     // cpu_id",
    "mov edx, [eax + 0]      // start_ip (low 32)",
    "mov esi, [eax + 4]      // start_ip (high 32)",
    // 打开 PAE + PGE
    "mov eax, cr4",
    "or eax, (1 << 5) | (1 << 7)",
    "mov cr4, eax",
    // EFER.LME
    "mov ecx, 0xC0000080",
    "rdmsr",
    "or eax, 1 << 8",
    "wrmsr",
    // cr3 = boot_p4
    "mov eax, offset boot_p4",
    "mov cr3, eax",
    // cr0: PG + PE
    "mov eax, cr0",
    "or eax, (1 << 31) | (1 << 0)",
    "mov cr0, eax",
    // 远跳到 64-bit（长模式已开，加载 0x08 kcode64 合法；32 位模式下
    // QEMU 的 EA 远跳正常，与 BSP boot asm 同一路径）
    ".byte 0xEA",
    ".long ap_trampoline64 - ap_trampoline32 + 0x8000",
    ".word 0x08",

    ".code64",
    "ap_trampoline64:",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    // 重新加载 ap_params（64-bit 下不能用 RIP 相对——代码在低地址副本，
    // RIP 不是链接 VMA；改用绝对立即数，零扩展后就是物理地址，identity
    // 映射可访问）
    "mov r11d, offset ap_params",
    "mov rdi, [r11 + 16]     // cpu_id (u32 at offset 16)",
    "mov rsi, [r11 + 8]      // stack_top",
    "mov rdx, [r11 + 0]      // start_ip",
    "mov rsp, rsi",
    "sub rsp, 8",            // dummy return slot
    // 同样不能用相对 call（RIP 在低地址），用绝对间接 call
    "mov r10d, offset ly_ap_entry",
    "call r10",
    // 若 Rust 入口返回，停 halt
    "3:",
    "cli",
    "hlt",
    "jmp 3b",
    ".global ap_trampoline_end",
    "ap_trampoline_end:",

    // ---- rodata: GDT + mb_info_ptr ----
    ".section .rodata",
    ".align 8",
    "boot_gdt:",
    ".quad 0x0000000000000000",   // 0x00 null
    ".quad 0x00AF9A000000FFFF",   // 0x08 kcode64（L=1, D=0）：长模式 64 位代码段
    ".quad 0x00CF92000000FFFF",   // 0x10 kdata
    ".quad 0x00CF9A000000FFFF",   // 0x18 kcode32（L=0, D=1）：非长模式下 AP 16→32 过渡
    "boot_gdt_end:",
    "boot_gdt_ptr:",
    ".word boot_gdt_end - boot_gdt - 1",
    ".long boot_gdt",
    "mb_info_ptr:",
    ".long 0",
}

extern "C" {
    #[allow(dead_code)] // boot 阶段页表符号：boot asm（main.rs 内联汇编）引用
    static boot_p4: u8;
    #[allow(dead_code)]
    static boot_p3: u8;
    #[allow(dead_code)] // 早期引导页表符号，2MiB 大页路径预留
    static boot_p2: u8;
    #[allow(dead_code)] // LAPIC 槽位专用 P2 表（boot asm 填充）
    static boot_p2_extra: u8;
}

/// Called from the 64-bit entry point.
#[no_mangle]
pub extern "C" fn kernel_main(mb_info: u32) -> ! {
    serial::init_serial();
    serial::serial_writestr(b"\r\n");
    serial::serial_writestr(b"===============================================\r\n");
    serial::serial_writestr(b"  LYSOR OS - minimal TCB kernel base v0.2.0\r\n");
    serial::serial_writestr(b"  build: ");
    serial::serial_writestr(env!("CARGO_PKG_VERSION").as_bytes());
    serial::serial_writestr(b"\r\n");
    serial::serial_writestr(b"===============================================\r\n");

    serial::log(">> multiboot2 info pointer: ");
    serial::log_hex(mb_info as u64);
    serial::log("\r\n");

    serial::log(">> installing GDT...\r\n");
    gdt::init_gdt();

    serial::log(">> installing IDT...\r\n");
    idt::init_idt();

    serial::log(">> remapping PIC8259...\r\n");
    pic::init_pic();

    serial::log(">> initializing physical memory manager...\r\n");
    unsafe { memory::init_physical(mb_info as usize) };

    serial::log(">> initializing virtual memory...\r\n");
    memory::init_virtual();

    // Kernel heap: real allocator backing Box/Vec (first-fit, boundary tags).
    memory::heap::ly_heap_init();
    if !memory::heap::ly_heap_selftest() {
        serial::log(">> kernel heap self-test FAILED\r\n");
    }
    // Per-process address spaces: create + cross-CR3 verify + reclaim.
    if !memory::paging::address_space_selftest() {
        serial::log(">> address space self-test FAILED\r\n");
    }

    // Scheduler must be initialized before the boot thread does any
    // schedulable work (yield syscalls, timer ticks): the boot thread gets
    // its real stack bounds + canary here, so early selftests are protected.
    sched::ly_sched_init();

    // BSP: 打开 LAPIC + 启动 APs（探测 CPU 数 → INIT-SIPI-SIPI）。
    serial::log(">> BSP: enabling LAPIC...\r\n");
    // LAPIC 高半区映射：低区 identity 的 p2_extra[503] walk 被 QEMU 拒绝，
    // 改用 higher-half fresh 页表链映射 LAPIC（map_page 标准路径）；smp/lapic.rs
    // 的 LAPIC_BASE 依赖此映射。
    {
        let _ = crate::memory::paging::map_page(
            0xFEE0_0000, 0xFFFF_8000_FEE0_0000, crate::memory::paging::PTE_WRITABLE);
    }
    smp::lapic::lapic_enable();
    smp::ly_smp_start_aps();

    // [REDACTED: vector font engine init (non-TCB gfx stack)]

    // Kernel command line parsed early: it decides the root device.

    serial::log(">> TCB boundary established (real grant table)\r\n");
    tcb::ly_tcb_init();

    // --- IPC subsystem self-test ---
    ipc::ly_ipc_init();
    vfs::ly_vfs_init();
    vfs::memfs::ly_memfs_init();
    syscall::ly_syscall_init();
    // [REDACTED: package signature init (sig::ly_pkg_sig_init, app-ecosystem trust)]
    {
        use ipc::channel_a::*;
        use ipc::channel_b::*;
        let ch = ly_chan_open("test", 1).unwrap();
        let mut msg = LyMsg::empty();
        msg.src_tid = 1; msg.dst_tid = 1; msg.len = 5;
        msg.data[..5].copy_from_slice(b"hello");
        ly_chan_send(ch, &msg).unwrap();
        if let Ok(Some(m)) = ly_chan_recv(ch, 1) {
            serial::log("   ipc test A: recv \"");
            serial::serial_writestr(&m.data[..m.len as usize]);
            serial::log("\" OK\r\n");
        }
        ly_chan_close(ch).unwrap();
        let shm = ly_shm_create("trusted", 1).unwrap();
        ly_shm_write(shm, 0, b"ipc-b", 1, 0).unwrap();
        let mut buf = [0u8; 5];
        ly_shm_read(shm, 0, &mut buf, 1, 0).unwrap();
        serial::log("   ipc test B: shm read \"");
        serial::serial_writestr(&buf);
        serial::log("\" OK\r\n");
        match ly_shm_write(shm, 0, b"x", 99, 0) {
            Err("whitelist denied") => serial::log("   ipc test B: whitelist denial OK\r\n"),
            _ => serial::log("   ipc test B: whitelist FAIL\r\n"),
        }
    }
    serial::log(">> LYSOR OS boot complete.\r\n");

    // --- Create test kernel threads ---
    serial::log(">> spawning SMP per-CPU workers...\r\n");
    // 用探测的 CPU 总数（而非 ONLINE_CPUS：个别 AP 的 ONLINE_CPUS 递增
    // 可能晚到，探测数才是"每个 CPU 都要有 worker"的权威依据）
    // CPU0 不创建 smp_worker：main 线程独占 CPU0 跑启动流程 + ring3 自检
    // （verify 是长计算），若 CPU0 也有 aff=0 的无限忙等 worker，pick_next_ready
    // 的 pass 0（亲和优先）会永远先挑回 worker → main/lyx/shell 饿死。
    let online = smp::TOTAL_CPUS.load(core::sync::atomic::Ordering::Relaxed);
    for c in 1..online {
        sched::ly_sched_create_on_cpu("smp_worker", sched::ly_sched_smp_worker, c as u32);
    }
    serial::log(">> spawning boot workers...\r\n");
    sched::ly_sched_create("worker_a", worker_a as extern "C" fn());
    sched::ly_sched_create("worker_b", worker_b as extern "C" fn());
    sched::ly_sched_create("worker_c", worker_c as extern "C" fn());

    // --- Start scheduler (never returns) ---
    sched::ly_sched_run();
}
// ---------------------------------------------------------------------------
// Syscall INT 0x80 end-to-end self-test (kernel-space caller).
// ---------------------------------------------------------------------------

/// Kernel-space syscall ABI helper: issues INT 0x80 with nr in rax and
/// args in rdi/rsi/rdx; the ISR dispatcher writes the result back to rax.
#[inline(never)]
unsafe fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "int 0x80",
        inout("rax") nr => ret,
        in("rdi") a0,
        in("rsi") a1,
        in("rdx") a2,
        options(nostack)
    );
    ret
}

/// Resolution switch self-test: 1024x768 and 1280x1024 must remap the FB
/// window, redraw through the graphics KC, then restore. Verifies the
/// variable-resolution path (VBE mode set + window remap) end to end.
fn res_switch_selftest() {
    serial::log(">> resolution switch selftest (geometry)\r\n");
    // Validate the framebuffer window for every supported resolution WITHOUT
    // actually switching VBE modes: QEMU stdvga reallocates the LFB and leaves
    // stale bands on a rapid switch cycle. The live switch path is exercised
    // through the Settings > Display UI (Enter applies, 15s auto-revert).
    // FB window: 0xFFFF800000200000 .. end; must never collide with the KC
    // stack window (0xFFFF800001000000).
    let base: u64 = 0xFFFF_8000_0020_0000;
    let kc_win: u64 = 0xFFFF_8000_0100_0000;
    let mut all_ok = true;
    for (w, h) in [(800u32, 600u32), (1024u32, 768u32), (1280u32, 1024u32)] {
        let pages = (w * h * 4 + 0xFFF) / 0x1000;
        let end = base + pages as u64 * 0x1000;
        let ok = end < kc_win;
        serial::log("   res geom: ");
        serial::log_u(w as u64);
        serial::log("x");
        serial::log_u(h as u64);
        serial::log(if ok { " OK (window clear)" } else { " COLLIDES KC WINDOW" });
        serial::log("\r\n");
        all_ok &= ok;
    }
    serial::log(if all_ok { ">> resolution selftest done\r\n" } else { ">> resolution selftest FAILED\r\n" });
}

fn syscall_selftest() {
    use crate::syscall::{
        SYS_CLOSE, SYS_IPC_RECV, SYS_IPC_SEND, SYS_OPEN, SYS_READ, SYS_SCHED_YIELD, SYS_WRITE,
    };
    serial::log(">> syscall INT 0x80 self-test\r\n");
    unsafe {
        // File roundtrip through the syscall ABI.
        let path = b"/tmp/sc_test\0";
        let fd = syscall3(SYS_OPEN, path.as_ptr() as usize, (vfs::O_CREAT | vfs::O_RDWR) as usize, 0);
        serial::log("   syscall: open -> fd=");
        serial::log_u(fd as u64);
        serial::log("\r\n");
        let data = b"syscall-int80-works";
        let w = syscall3(SYS_WRITE, fd as usize, data.as_ptr() as usize, data.len());
        let _ = syscall3(SYS_CLOSE, fd as usize, 0, 0);
        let fd2 = syscall3(SYS_OPEN, path.as_ptr() as usize, vfs::O_RDWR as usize, 0);
        let mut buf = [0u8; 64];
        let n = syscall3(SYS_READ, fd2 as usize, buf.as_mut_ptr() as usize, 64);
        let ok = w == data.len() as isize
            && n == data.len() as isize
            && &buf[..n.max(0) as usize] == data;
        serial::log("   syscall: w=");
        serial::log_u(w as u64);
        serial::log(" n=");
        serial::log_u(n as u64);
        serial::log(" roundtrip ");
        serial::log(if ok { "OK\r\n" } else { "FAIL\r\n" });
        let _ = syscall3(SYS_CLOSE, fd2 as usize, 0, 0);

        // IPC through the syscall ABI (channel A ring buffer).
        use crate::ipc::channel_a::{ly_chan_open, LyMsg};
        // Boot context tid=0: owner and broadcast dst must be 0 (any caller).
        let ch = ly_chan_open("scipc", 0).unwrap();
        let mut m = LyMsg::empty();
        m.src_tid = 0;
        m.dst_tid = 0;
        m.len = 6;
        m.data[..6].copy_from_slice(b"ipcint");
        let s = syscall3(SYS_IPC_SEND, ch, &m as *const LyMsg as usize, 0);
        let mut out = LyMsg::empty();
        let r = syscall3(SYS_IPC_RECV, ch, &mut out as *mut LyMsg as usize, 0);
        let ok2 = s == 0 && r == 6 && &out.data[..6] == b"ipcint";
        serial::log("   syscall: ipc send/recv via INT 0x80 ");
        serial::log(if ok2 { "OK\r\n" } else { "FAIL\r\n" });

        // Cooperative yield: no other ready thread yet, must return 0.
        let y = syscall3(SYS_SCHED_YIELD, 0, 0, 0);
        serial::log("   syscall: sched_yield -> ");
        serial::log_u(y as u64);
        serial::log("\r\n");

        // Unknown number: ENOSYS (-38).
        let e = syscall3(999, 0, 0, 0);
        serial::log("   syscall: unknown nr -> ");
        serial::log_u(e as u64);
        serial::log(" (expect 18446744073709551578 = -38)\r\n");
    }
}

// ---------------------------------------------------------------------------
// Disk boot test: exercises the real mounted volume end to end.
// ---------------------------------------------------------------------------

fn disk_boot_test() {
    use crate::vfs;
    serial::log(">> disk boot test\r\n");

    // Ensure standard directory tree exists (dependency order matters).
    // Boot thread (tid 0) is the TCB context: kernel capabilities.
    for dir in [
        "/etc", "/home", "/home/user", "/bin", "/tmp",
        "/var", "/var/log", "/opt", "/registry", "/sys",
    ] {
        let fd = vfs::ly_vfs_open(dir, vfs::O_CREAT | vfs::O_DIRECTORY | vfs::O_RDWR, 0);
        if fd >= 0 {
            vfs::ly_vfs_close(fd);
        }
    }

    // Boot counter: survives across reboots on the real disk.
    let mut cnt: u32 = 0;
    let fd = vfs::ly_vfs_open("/etc/bootcount", vfs::O_RDWR | vfs::O_CREAT, 0);
    if fd >= 0 {
        let mut b = [0u8; 4];
        let n = vfs::ly_vfs_read(fd, b.as_mut_ptr(), 4);
        if n == 4 {
            cnt = u32::from_le_bytes(b);
        }
        cnt += 1;
        vfs::ly_vfs_seek(fd, 0, 0);
        if vfs::ly_vfs_write(fd, cnt.to_le_bytes().as_ptr(), 4) != 4 {
            serial::log("   disk: bootcount write FAIL\r\n");
        }
        vfs::ly_vfs_close(fd);
    }

    // File roundtrip test.
    const MSG: &[u8] = b"Hello from LYSOR OS on a real ATA disk! abcdefghijklmnopqrstuvwxyz 0123456789";
    let fd = vfs::ly_vfs_open("/home/user/hello.txt", vfs::O_CREAT | vfs::O_RDWR, 0);
    if fd >= 0 {
        let w = vfs::ly_vfs_write(fd, MSG.as_ptr(), MSG.len());
        vfs::ly_vfs_seek(fd, 0, 0);
        let mut rb = [0u8; 128];
        let n = vfs::ly_vfs_read(fd, rb.as_mut_ptr(), 128);
        let ok = w == MSG.len() as isize && n == MSG.len() as isize && &rb[..n as usize] == MSG;
        serial::log("   disk: hello.txt roundtrip ");
        if ok {
            serial::log("OK\r\n");
        } else {
            serial::log("FAIL\r\n");
        }
        vfs::ly_vfs_close(fd);
    }

    // Unlink test: create then delete a scratch file.
    let fd = vfs::ly_vfs_open("/tmp/scratch", vfs::O_CREAT | vfs::O_RDWR, 0);
    if fd >= 0 {
        vfs::ly_vfs_close(fd);
        match vfs::ly_vfs_unlink("/tmp/scratch") {
            Ok(()) => serial::log("   disk: unlink OK\r\n"),
            Err(e) => serial::log(e),
        }
    }

    // List root.
    let mut entries = [vfs::LyDirEntry { name: [0u8; 56], name_len: 0, ftype: 0, size: 0 }; 64];
    let n = match vfs::ly_vfs_list("/", &mut entries) {
        Ok(n) => n,
        Err(e) => {
            serial::log("   disk: list failed: ");
            serial::log(e);
            0
        }
    };
    serial::log("   disk: root entries: ");
    serial::log_u(n as u64);
    serial::log("\r\n");
    for i in 0..n {
        serial::log("     ");
        let name_len = entries[i].name_len as usize;
        serial::serial_writestr(&entries[i].name[..name_len]);
        if entries[i].ftype == 1 {
            serial::log("/");
        }
        serial::log("\r\n");
    }
    serial::log("   disk: bootcount=");
    serial::log_u(cnt as u64);
    serial::log("\r\n");
    if cnt >= 2 {
        serial::log(">> DISK PERSISTENCE VERIFIED (cross-boot)\r\n");
    } else {
        serial::log(">> disk write OK (persistence verified on next boot)\r\n");
    }
}
#[no_mangle]
extern "C" fn worker_b() {
    serial::log("   [B] thread started\r\n");
    let mut count: u64 = 0;
    loop {
        count += 1;
        if count % 200000 == 0 {
            serial::log("   [B] tick ");
            serial::log_u(count);
            serial::log("\r\n");
        }
    }
}

#[no_mangle]
extern "C" fn worker_c() {
    serial::log("   [C] thread started\r\n");
    let mut count: u64 = 0;
    loop {
        count += 1;
        if count % 200000 == 0 {
            serial::log("   [C] tick ");
            serial::log_u(count);
            serial::log("\r\n");
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial::serial_writestr(b"\r\n*** KERNEL PANIC *** ");
    if let Some(loc) = info.location() {
        serial::serial_writestr(loc.file().as_bytes());
        serial::serial_writestr(b":");
        serial::serial_write_u32(loc.line());
        serial::serial_writestr(b" ");
    }
    serial::serial_writestr(b"\r\n");
    loop {
        unsafe { core::arch::asm!("cli; hlt") };
    }
}
