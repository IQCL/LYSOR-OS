//! LYSOR OS Terminal (`ly_terminal`).
//!
//! Interactive shell over the serial console. Real commands backed by the
//! VFS / scheduler / exec loader (no stubs):
//!   help, ls, cd, pwd, cat, mkdir, rm, run, ps, kcmd, echo, date, clear,
//!   uname, uptime, free, df, whoami, hostname, exit

use crate::serial;
use crate::vfs;
use crate::sched;

const CMD_BUF_SIZE: usize = 128;
static mut CMD_BUF: [u8; CMD_BUF_SIZE] = [0u8; CMD_BUF_SIZE];
static mut CMD_LEN: usize = 0;
static mut CWD: [u8; 64] = [0u8; 64];
static mut CWD_LEN: usize = 1;

/// Kernel-global root device name, set by the `root=` cmdline parameter.
pub static mut K_ROOT_DEV: [u8; 16] = [0u8; 16];
pub static mut K_ROOT_SET: bool = false;

/// Return the parsed root device name (e.g. "sda"), default "ram0".
pub fn ly_root_device() -> &'static str {
    unsafe {
        if !K_ROOT_SET {
            return "ram0";
        }
        let n = K_ROOT_DEV.iter().position(|&b| b == 0).unwrap_or(K_ROOT_DEV.len());
        let s = core::str::from_utf8(&K_ROOT_DEV[..n]).unwrap_or("ram0");
        s
    }
}

/// Join the current working directory with a relative path.
fn resolve_cwd(path: &str) -> alloc_path {
    unsafe {
        let mut out = [0u8; 64];
        if path.starts_with('/') {
            let n = path.len().min(63);
            out[..n].copy_from_slice(&path.as_bytes()[..n]);
            return alloc_path { buf: out, len: n };
        }
        let mut n = CWD_LEN;
        out[..n].copy_from_slice(&CWD[..n]);
        if !path.is_empty() {
            if out[n - 1] != b'/' { out[n] = b'/'; n += 1; }
            let pb = path.as_bytes();
            let m = pb.len().min(63 - n);
            out[n..n + m].copy_from_slice(&pb[..m]);
            n += m;
        }
        alloc_path { buf: out, len: n }
    }
}

struct alloc_path { buf: [u8; 64], len: usize }

/// Parse and execute a command line (real implementations).
pub fn ly_term_exec(cmdline: &str, caller_tid: u32) {
    let mut parts = cmdline.trim().split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return,
    };

    match cmd {
        "help" => {
            serial::log("LYSOR OS terminal (real commands):\r\n");
            serial::log("  ls [path]    list directory\r\n");
            serial::log("  cd <path>    change directory\r\n");
            serial::log("  pwd          print working directory\r\n");
            serial::log("  cat <file>   print file contents\r\n");
            serial::log("  mkdir <dir>  create directory\r\n");
            serial::log("  rm <file>    delete file\r\n");
            serial::log("  run <app>    run /bin/<app>.lyx (dual-track sig)\r\n");
            serial::log("  ps           list threads\r\n");
            serial::log("  kcmd <sub>   kernel debug commands (privileged)\r\n");
            serial::log("  echo/date/clear/uname/uptime/free/df/whoami/hostname\r\n");
            serial::log("  exit         close terminal\r\n");
        }
        "ls" => {
            let arg = parts.next().unwrap_or("");
            let p = if arg.is_empty() { unsafe { core::str::from_utf8(&CWD[..CWD_LEN]).unwrap_or("/") } } else { arg };
            crate::coreutils::cmd_ls(p);
        }
        "cd" => {
            let arg = parts.next().unwrap_or("/");
            let ap = resolve_cwd(arg);
            let s = core::str::from_utf8(&ap.buf[..ap.len]).unwrap_or("/");
            // validate the directory exists and is a directory
            if let Ok((_, mode)) = vfs::ly_vfs_stat(s) {
                if mode & 0o170000 == 0o040000 {
                    unsafe {
                        CWD[..ap.len].copy_from_slice(&ap.buf[..ap.len]);
                        CWD_LEN = ap.len;
                    }
                    serial::log("  cd -> ");
                    serial::log(s);
                    serial::log("\r\n");
                } else {
                    serial::log("  cd: not a directory\r\n");
                }
            } else {
                serial::log("  cd: no such directory\r\n");
            }
        }
        "pwd" => {
            serial::log("  ");
            serial::log(unsafe { core::str::from_utf8(&CWD[..CWD_LEN]).unwrap_or("/") });
            serial::log("\r\n");
        }
        "cat" => {
            let arg = parts.next().unwrap_or("");
            let ap = resolve_cwd(arg);
            crate::coreutils::cmd_cat(core::str::from_utf8(&ap.buf[..ap.len]).unwrap_or(""));
        }
        "mkdir" => {
            let arg = parts.next().unwrap_or("");
            let ap = resolve_cwd(arg);
            let s = core::str::from_utf8(&ap.buf[..ap.len]).unwrap_or("");
            let fd = vfs::ly_vfs_open(s, vfs::O_CREAT | vfs::O_DIRECTORY | vfs::O_RDWR, 0);
            if fd >= 0 {
                vfs::ly_vfs_close(fd);
                serial::log("  mkdir: created ");
                serial::log(s);
                serial::log("\r\n");
            } else {
                serial::log("  mkdir: failed (exists or denied)\r\n");
            }
        }
        "rm" => {
            let arg = parts.next().unwrap_or("");
            let ap = resolve_cwd(arg);
            let s = core::str::from_utf8(&ap.buf[..ap.len]).unwrap_or("");
            match vfs::ly_vfs_unlink(s) {
                Ok(()) => { serial::log("  rm: removed "); serial::log(s); serial::log("\r\n"); }
                Err(e) => { serial::log("  rm: "); serial::log(e); serial::log("\r\n"); }
            }
        }
        "run" => {
            let name = parts.next().unwrap_or("");
            if name.is_empty() { serial::log("  run: usage: run <app>\r\n"); return; }
            // load /bin/<name>.lyx and exec through the signed loader
            let mut path = [0u8; 64];
            path[..5].copy_from_slice(b"/bin/");
            let mut n = 5;
            let nb = name.as_bytes();
            let m = nb.len().min(52);
            path[n..n + m].copy_from_slice(&nb[..m]);
            n += m;
            if n + 4 > path.len() { serial::log("  run: name too long\r\n"); return; }
            path[n..n + 4].copy_from_slice(b".lyx");
            n += 4;
            let p = core::str::from_utf8(&path[..n]).unwrap_or("");
            let fd = vfs::ly_vfs_open(p, vfs::O_RDWR, 0);
            if fd < 0 { serial::log("  run: "); serial::log(p); serial::log(" not found\r\n"); return; }
            unsafe {
                static mut RUN_BUF: [u8; 65536] = [0u8; 65536];
                let len = vfs::ly_vfs_read(fd, RUN_BUF.as_mut_ptr(), RUN_BUF.len());
                vfs::ly_vfs_close(fd);
                if len <= 0 { serial::log("  run: empty app\r\n"); return; }
                serial::log("  run: loading ");
                serial::log(p);
                serial::log(" (");
                serial::log_u(len as u64);
                serial::log(" bytes)\r\n");
                match crate::exec::ly_exec_lyx(RUN_BUF.as_ptr(), len as usize) {
                Ok(tid) => { serial::log("  run: started tid="); serial::log_u(tid as u64); serial::log("\r\n"); }
                    Err(e) => { serial::log("  run: rejected: "); serial::log(e); serial::log("\r\n"); }
                }
            }
        }
        "ps" => crate::coreutils::cmd_ps_table(),
        "kcmd" => {
            // Kernel debug command: requires root capability (tid 1 = boot shell)
            if caller_tid != 1 {
                serial::log("  kcmd: permission denied (need root)\r\n");
                return;
            }
            let sub = parts.next().unwrap_or("");
            match sub {
                "sync" => { crate::fs::lfs::lfs_sync(); serial::log("  kcmd: lfs synced (dirty flag cleared)\r\n"); }
                "fsck" => { crate::fs::lfs::lfs_fsck(); serial::log("  kcmd: fsck done\r\n"); }
                "frames" => {
                    serial::log("  kcmd: frames used=");
                    serial::log_u(crate::memory::physical::frame_used_count() as u64);
                    serial::log("/");
                    serial::log_u(crate::memory::physical::frame_total() as u64);
                    serial::log("\r\n");
                }
                "stats" => sched::ly_sched_stats(),
                "audit" => crate::security::audit_dump(),
                "caps" => {
                    let t = parts.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
                    let caps = sched::ly_sched_thread_caps(t);
                    serial::log("  kcmd: caps(tid");
                    serial::log_u(t as u64);
                    serial::log(") = 0x");
                    serial::log_hex(caps.bits() as u64);
                    serial::log("\r\n");
                }
                "poweroff" => { crate::pm::pm_poweroff(); }
                "canary" => {
                    // verify our own stack canary live
                    let tid = sched::ly_sched_current_tid();
                    let top = sched::ly_sched_stack_top(tid);
                    serial::log("  kcmd: canary(tid");
                    serial::log_u(tid as u64);
                    serial::log(") = ");
                    serial::log(if crate::security::stack_canary_check(top, tid) { "OK (armed, verified live)\r\n" } else { "VIOLATION\r\n" });
                }
                _ => { serial::log("  kcmd: subs = sync|fsck|frames|stats|audit|caps|poweroff|canary\r\n"); }
            }
        }
        "echo" => {
            let rest = cmdline.trim().strip_prefix("echo").unwrap_or("").trim_start();
            serial::log("  ");
            serial::log(rest);
            serial::log("\r\n");
        }
        "date" => {
            // coarse: PIT-based relative clock; real RTC sync arrives with ntp
            let secs = sched::pit::ly_pit_ticks() / 100;
            serial::log("  uptime ");
            serial::log_u(secs);
            serial::log("s (RTC time pending ntp sync)\r\n");
        }
        "clear" => {
            for _ in 0..40 { serial::log("\r\n"); }
        }
        "uname" => crate::coreutils::cmd_uname(),
        "uptime" => crate::coreutils::cmd_uptime(),
        "free" => crate::coreutils::cmd_free(),
        "df" => crate::coreutils::cmd_df(),
        "whoami" => crate::coreutils::cmd_whoami(),
        "hostname" => crate::coreutils::cmd_hostname(),
        "exit" => {
            serial::log("  closing terminal...\r\n");
        }
        _ => {
            serial::log("  unknown command: ");
            serial::log(cmd);
            serial::log(" (try 'help')\r\n");
        }
    }
}

/// Kernel command line: parse boot-time arguments.
/// Example: "console=ttyS0 root=/dev/sda quiet"
/// `ram=NNN` override (MiB) from the kernel command line, 0 = not set.
static mut KCMDLINE_RAM_MIB: u32 = 0;

/// Return the `ram=` override in MiB (0 = unset).
pub fn ly_kcmdline_ram_mib() -> u32 {
    unsafe { KCMDLINE_RAM_MIB }
}

pub fn ly_kcmdline_parse(cmdline: &str) {
    serial::log("   kcmdline: \"");
    serial::log(cmdline);
    serial::log("\"\r\n");
    for part in cmdline.split_whitespace() {
        if part.starts_with("console=") {
            serial::log("   kcmdline: console = ");
            serial::log(&part[8..]);
            serial::log("\r\n");
        } else if part.starts_with("ram=") {
            let mut mib: u32 = 0;
            for ch in part[4..].bytes() {
                if ch < b'0' || ch > b'9' { break; }
                mib = mib * 10 + (ch - b'0') as u32;
            }
            if mib > 0 && mib <= 4096 {
                unsafe { KCMDLINE_RAM_MIB = mib; }
                serial::log("   kcmdline: ram = ");
                serial::log_u(mib as u64);
                serial::log(" MiB (override)\r\n");
            }
        } else if part.starts_with("root=") {
            let dev = &part[5..];
            serial::log("   kcmdline: root = ");
            serial::log(dev);
            serial::log("\r\n");
            // store device name (strip /dev/ prefix)
            let dev = dev.strip_prefix("/dev/").unwrap_or(dev);
            unsafe {
                for (i, &b) in dev.as_bytes().iter().enumerate().take(15) {
                    K_ROOT_DEV[i] = b;
                }
                K_ROOT_DEV[dev.as_bytes().len().min(15)] = 0;
                K_ROOT_SET = true;
            }
        } else if part == "quiet" {
            serial::log("   kcmdline: quiet mode\r\n");
        }
    }
}

/// Initialize terminal subsystem.
pub fn ly_term_init() {
    unsafe { CWD[0] = b'/'; CWD_LEN = 1; }
    serial::log("   tty: terminal ready (real commands: ls/cd/cat/mkdir/rm/run/ps/kcmd + coreutils)\r\n");
}
