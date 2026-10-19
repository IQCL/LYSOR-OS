//! LYSOR OS - minimal TCB kernel base
//!
//! Prefix: `ly_` for all public kernel symbols.
//! Boot: Multiboot2 -> 32-bit stub -> temporary paging -> x86_64 long mode.

#![no_std]
#![no_main]

// Public microkernel build: this crate contains ONLY the minimal TCB
// (scheduler / memory / IPC / IDT / syscalls / VFS / drivers). The kernel
// component manager (kcomponent), graphics (gfx/wm/ui/xmlui/desktop),
// crypto (SM2/SM3/SM4 + SHA-512/Ed25519), apps, user services, snap apps,
// LYAP packages, the vector font engine and graphical assets are private
// subsystems shipped in the full (private) repository only.
pub mod serial;
pub mod vga;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod memory;
pub mod tcb;
pub mod sched;
pub mod ipc;
pub mod vfs;
pub mod syscall;
pub mod exec;

/// `ram=` override in MiB (parsed from the kernel cmdline), 0 = unset.
pub fn ly_kcmdline_ram_mib() -> u32 {
    tty::ly_kcmdline_ram_mib()
}
pub mod sig;
pub mod tty;
pub mod fs;
pub mod drivers;
pub mod security;
pub mod net;
pub mod syslog;
pub mod init;
pub mod coreutils;
pub mod auth;
pub mod cgroup;
pub mod cron;
pub mod ntp;
pub mod dbus;
pub mod pm;
pub mod udev;

// ---------------------------------------------------------------------------
// Boot assembly (Multiboot2 header + 32-bit stub + 64-bit entry)
// ---------------------------------------------------------------------------

core::arch::global_asm! {
    ".intel_syntax noprefix",

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

    // ---- BSS: stack + boot page tables ----
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

    // ---- rodata: GDT + mb_info_ptr ----
    ".section .rodata",
    ".align 8",
    "boot_gdt:",
    ".quad 0x0000000000000000",
    ".quad 0x00AF9A000000FFFF",
    ".quad 0x00CF92000000FFFF",
    "boot_gdt_end:",
    "boot_gdt_ptr:",
    ".word boot_gdt_end - boot_gdt - 1",
    ".long boot_gdt",
    "mb_info_ptr:",
    ".long 0",
}

/// Called from the 64-bit entry point.
#[no_mangle]
pub extern "C" fn kernel_main(mb_info: u32) -> ! {
    serial::init_serial();
    serial::serial_writestr(b"\r\n");
    serial::serial_writestr(b"===============================================\r\n");
    serial::serial_writestr(b"  LYSOR OS - kernel base v0.1.0\r\n");
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
    serial::log("DBG: virtual memory done\r\n");

    // Scheduler must be initialized before the boot thread does any
    // schedulable work (yield syscalls, timer ticks): the boot thread gets
    // its real stack bounds + canary here, so early selftests are protected.
    serial::log("DBG: sched init\r\n");
    sched::ly_sched_init();
    serial::log("DBG: sched init done\r\n");

    // Kernel command line parsed early: it decides the root device.
    serial::log("DBG: cmdline parse\r\n");
    tty::ly_kcmdline_parse("console=ttyS0 root=/dev/sda");
    serial::log("DBG: cmdline parse done\r\n");
    serial::log(">> root device: /dev/");
    serial::log(tty::ly_root_device());
    serial::log("\r\n");

    serial::log(">> TCB boundary established (real grant table)\r\n");
    tcb::ly_tcb_init();

    // --- IPC subsystem self-test ---
    ipc::ly_ipc_init();
    vfs::ly_vfs_init();
    vfs::memfs::ly_memfs_init();
    syscall::ly_syscall_init();
    exec::ly_exec_init();
    // Signature verification delegates to the optional crypto KC (private
    // subsystem); with the KC absent the trust module rejects all requests.
    sig::ly_pkg_sig_init();
    tty::ly_term_init();
    drivers::ps2kbd::ps2_init();
    drivers::ps2mouse::ps2mouse_init();
    drivers::pci::pci_enumerate();
    security::security_init();
    drivers::ramdisk::ramdisk_init();
    drivers::ata::ata_init();
    drivers::blockdev::ly_blk_dump();
    // Mount the real root device (ATA disk, or ramdisk fallback).
    match vfs::ly_vfs_mount_root(tty::ly_root_device()) {
        Ok(()) => {
            // Headless public build: the kernel component manager, graphics
            // KC, window manager, desktop, apps, crypto and snap runtime are
            // private subsystems (full build in the private repository).
            // The public microkernel boots headless: mount -> boot tests.
            disk_boot_test();
            syscall_selftest();
        }
        Err(e) => {
            serial::log(">> mount failed: ");
            serial::log(e);
            serial::log("\r\n");
        }
    }
    net::net_init();
    init::init_start();
    auth::auth_init();
    cgroup::cgroup_init();
    cron::cron_init();
    ntp::ntp_init();
    dbus::dbus_init();
    pm::pm_init();
    udev::udev_init();
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
    serial::log(">> spawning test threads...\r\n");
    sched::ly_sched_create("worker_a", worker_a as extern "C" fn());
    sched::ly_sched_create("worker_b", worker_b as extern "C" fn());
    sched::ly_sched_create("worker_c", worker_c as extern "C" fn());
    sched::ly_sched_create("idle", headless_idle as extern "C" fn());

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

// ---------------------------------------------------------------------------
// Test worker threads
// ---------------------------------------------------------------------------

#[no_mangle]
/// Headless public build: the graphical desktop shell is a private subsystem
/// (full build only); the public microkernel runs a simple idle loop.
extern "C" fn headless_idle() {
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

extern "C" fn worker_a() {
    serial::log("   [A] thread started\r\n");
    let mut count: u64 = 0;
    loop {
        count += 1;
        if count % 200000 == 0 {
            serial::log("   [A] tick ");
            serial::log_u(count);
            serial::log("\r\n");
        }
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



























































