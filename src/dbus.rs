//! DBus 鈥?绯荤粺娑堟伅鎬荤嚎.
//! 鏈嶅姟鍙戠幇, 璺ㄨ繘绋婻PC妗?

use crate::serial;

const MAX_SERVICES: usize = 16;
const MAX_MSGS: usize = 64;

#[derive(Clone, Copy)]
pub struct ServiceEntry {
    pub name: [u8; 24],
    pub name_len: usize,
    pub pid: u32,
}

impl ServiceEntry {
    const fn new() -> Self {
        ServiceEntry { name: [0u8;24], name_len: 0, pid: 0 }
    }
}

static mut SERVICES: [ServiceEntry; MAX_SERVICES] = [ServiceEntry::new(); MAX_SERVICES];

pub fn dbus_register(name: &str, pid: u32) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_SERVICES {
            if SERVICES[i].name_len == 0 {
                let b = name.as_bytes();
                let n = b.len().min(24);
                SERVICES[i].name[..n].copy_from_slice(&b[..n]);
                SERVICES[i].name_len = n;
                SERVICES[i].pid = pid;
                return Ok(i);
            }
        }
        Err("no dbus slot")
    }
}

pub fn dbus_lookup(name: &str) -> Option<u32> {
    unsafe {
        for i in 0..MAX_SERVICES {
            if SERVICES[i].name_len == 0 { continue; }
            let n = core::str::from_utf8(&SERVICES[i].name[..SERVICES[i].name_len]).unwrap_or("");
            if n == name { return Some(SERVICES[i].pid); }
        }
        None
    }
}

pub fn dbus_init() {
    serial::log("   dbus: system bus ready (16 services)\r\n");
}

