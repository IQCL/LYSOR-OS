//! cgroup 鈥?杩涚▼璧勬簮闄愬埗.
//! CPU/鍐呭瓨閰嶉, OOM 妗?

use crate::serial;

const MAX_CGROUPS: usize = 8;

#[derive(Clone, Copy)]
pub struct Cgroup {
    pub name: [u8; 16],
    pub name_len: usize,
    pub mem_limit_kb: u32,
    pub cpu_weight: u32,
    pub procs: u32,
}

impl Cgroup {
    const fn new() -> Self {
        Cgroup { name: [0u8;16], name_len: 0, mem_limit_kb: 0, cpu_weight: 1024, procs: 0 }
    }
}

static mut CGROUPS: [Cgroup; MAX_CGROUPS] = [Cgroup::new(); MAX_CGROUPS];

pub fn cgroup_create(name: &str, mem_kb: u32) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_CGROUPS {
            if CGROUPS[i].name_len == 0 {
                let b = name.as_bytes();
                let n = b.len().min(16);
                CGROUPS[i].name[..n].copy_from_slice(&b[..n]);
                CGROUPS[i].name_len = n;
                CGROUPS[i].mem_limit_kb = mem_kb;
                return Ok(i);
            }
        }
        Err("no cgroup slot")
    }
}

pub fn cgroup_attach(idx: usize) {
    unsafe { if idx < MAX_CGROUPS { CGROUPS[idx].procs += 1; } }
}

pub fn cgroup_init() {
    serial::log("   cgroup: resource limits ready (8 groups)\r\n");
}

