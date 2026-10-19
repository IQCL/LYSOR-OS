//! cron 鈥?瀹氭椂浠诲姟璋冨害.
//! 鍛ㄦ湡浠诲姟, 绉掔骇绮惧害.

use crate::serial;

const MAX_JOBS: usize = 16;

#[derive(Clone, Copy)]
pub struct CronJob {
    pub name: [u8; 24],
    pub name_len: usize,
    pub interval_ticks: u32,
    pub last_tick: u32,
    pub enabled: bool,
}

impl CronJob {
    const fn new() -> Self {
        CronJob { name: [0u8;24], name_len: 0, interval_ticks: 0, last_tick: 0, enabled: false }
    }
}

static mut JOBS: [CronJob; MAX_JOBS] = [CronJob::new(); MAX_JOBS];
static mut TICK: u32 = 0;

pub fn cron_add(name: &str, interval_hz: u32) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_JOBS {
            if JOBS[i].name_len == 0 {
                let b = name.as_bytes();
                let n = b.len().min(24);
                JOBS[i].name[..n].copy_from_slice(&b[..n]);
                JOBS[i].name_len = n;
                JOBS[i].interval_ticks = 100 / interval_hz.max(1);
                JOBS[i].enabled = true;
                return Ok(i);
            }
        }
        Err("no cron slot")
    }
}

pub fn cron_tick() {
    unsafe {
        TICK += 1;
        for i in 0..MAX_JOBS {
            if JOBS[i].enabled && TICK - JOBS[i].last_tick >= JOBS[i].interval_ticks {
                JOBS[i].last_tick = TICK;
            }
        }
    }
}

pub fn cron_init() {
    serial::log("   cron: scheduler ready (16 jobs, 100Hz tick)\r\n");
}

