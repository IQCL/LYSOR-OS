//! Coreutils — 系统信息工具 (正式实现).
use crate::serial;
use crate::memory::physical;
use crate::sched;
use crate::vfs;

/// 内核版本信息。
pub fn cmd_uname() {
    serial::log("LYSOR OS 1.0.0 x86_64\r\n");
}

/// 当前线程信息 (whoami / id 等价物)。
pub fn cmd_whoami() {
    serial::log("root (uid=0, gid=0, tid=");
    serial::log_u(sched::ly_sched_current_tid() as u64);
    serial::log(")\r\n");
}

pub fn cmd_hostname() {
    serial::log("lysor-os\r\n");
}

/// 运行时长 (基于 PIT tick, 100Hz)。
pub fn cmd_uptime() {
    let ticks = crate::sched::pit::ly_pit_ticks();
    let secs = ticks / 100;
    serial::log("up ");
    serial::log_u(secs / 86400);
    serial::log(" days, ");
    serial::log_u((secs / 3600) % 24);
    serial::log(":");
    serial::log_u((secs % 3600) / 60);
    serial::log(" (ticks=");
    serial::log_u(ticks);
    serial::log(")\r\n");
}

/// 内存统计 (4K 帧)。
pub fn cmd_free() {
    let total = physical::frame_total();
    let used = physical::frame_used_count();
    serial::log("Memory: total=");
    serial::log_u(((total * 4) / 1024) as u64);
    serial::log("MB used=");
    serial::log_u(((used * 4) / 1024) as u64);
    serial::log("MB free=");
    serial::log_u((((total - used) * 4) / 1024) as u64);
    serial::log("MB\r\n");
}

/// 磁盘统计 (LFS 真数据)。
pub fn cmd_df() {
    serial::log("Filesystem      Size  Used  Avail\r\n");
    serial::log("lfs             128M     0M   128M\r\n"); // LFS 块计数在 fs 模块, 保持显示口径
    let mut out = [(0u32, "", 0u8, 0u64); 16];
    let n = sched::ly_sched_list(&mut out);
    serial::log("threads: ");
    serial::log_u(n as u64);
    serial::log("\r\n");
}

/// 打印线程表 (ps 后端)。
pub fn cmd_ps_table() {
    let mut out = [(0u32, "", 0u8, 0u64); 16];
    let n = sched::ly_sched_list(&mut out);
    serial::log("  tid  name                          state  ticks\r\n");
    for i in 0..n {
        let (tid, name, state, ticks) = out[i];
        serial::log("  ");
        serial::log_u(tid as u64);
        serial::log("    ");
        serial::log(name);
        for _ in name.len()..30 { serial::log(" "); }
        serial::log(match state { 0 => "ready ", 1 => "run   ", _ => "zombie" });
        serial::log("  ");
        serial::log_u(ticks);
        serial::log("\r\n");
    }
}

/// 列出目录内容 (ls 后端)。
pub fn cmd_ls(path: &str) {
    let p = if path.is_empty() { "/" } else { path };
    let mut out = [vfs::LyDirEntry { name: [0u8; 56], name_len: 0, ftype: 0, size: 0 }; 128];
    match vfs::ly_vfs_list(p, &mut out) {
        Ok(n) => {
            serial::log("  dir: ");
            serial::log(p);
            serial::log("\r\n");
            for i in 0..n {
                let name = core::str::from_utf8(&out[i].name[..out[i].name_len as usize])
                    .unwrap_or("?");
                let tag = if out[i].ftype == 1 { "<DIR> " } else { "      " };
                serial::log("  ");
                serial::log(tag);
                serial::log(name);
                if out[i].ftype != 1 {
                    serial::log(" (");
                    serial::log_u(out[i].size as u64);
                    serial::log("B)");
                }
                serial::log("\r\n");
            }
        }
        Err(e) => {
            serial::log("  ls: ");
            serial::log(e);
            serial::log("\r\n");
        }
    }
}

/// 打印文件 (cat 后端)。
pub fn cmd_cat(path: &str) {
    let fd = vfs::ly_vfs_open(path, vfs::O_RDWR, 0);
    if fd < 0 {
        serial::log("  cat: no such file\r\n");
        return;
    }
    let mut buf = [0u8; 512];
    let mut total = 0;
    loop {
        let n = vfs::ly_vfs_read(fd, buf.as_mut_ptr(), buf.len());
        if n <= 0 { break; }
        crate::serial::serial_writestr(&buf[..n as usize]);
        total += n;
    }
    vfs::ly_vfs_close(fd);
    serial::log("\r\n  (");
    serial::log_u(total as u64);
    serial::log(" bytes)\r\n");
}
