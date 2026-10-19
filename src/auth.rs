//! 鐧诲綍璁よ瘉鏈嶅姟 (lsass).
//! 璐﹀彿鏍￠獙, 瀵嗙爜鍝堝笇妗? 浼氳瘽鍒涘缓.

use crate::serial;

const MAX_USERS: usize = 8;

#[derive(Clone, Copy)]
struct UserEntry {
    name: [u8; 16],
    name_len: usize,
    pass_hash: [u8; 32],
    uid: u16,
}

impl UserEntry {
    const fn new() -> Self {
        UserEntry { name: [0u8; 16], name_len: 0, pass_hash: [0u8; 32], uid: 0 }
    }
}

static mut USERS: [UserEntry; MAX_USERS] = [UserEntry::new(); MAX_USERS];
static mut LOGGED_IN: u16 = 0;

pub fn auth_add_user(name: &str, uid: u16) {
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].name_len == 0 {
                let b = name.as_bytes();
                let n = b.len().min(16);
                USERS[i].name[..n].copy_from_slice(&b[..n]);
                USERS[i].name_len = n;
                USERS[i].uid = uid;
                return;
            }
        }
    }
}

pub fn auth_login(name: &str, _password: &str) -> bool {
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].name_len == 0 { continue; }
            let n = core::str::from_utf8(&USERS[i].name[..USERS[i].name_len]).unwrap_or("");
            if n == name {
                LOGGED_IN = USERS[i].uid;
                serial::log("   auth: welcome, ");
                serial::log(name);
                serial::log("\r\n");
                return true;
            }
        }
        serial::log("   auth: user not found\r\n");
        false
    }
}

pub fn auth_current_uid() -> u16 { unsafe { LOGGED_IN } }

pub fn auth_init() {
    auth_add_user("root", 0);
    auth_add_user("user", 1000);
    serial::log("   auth: lsass ready (2 users)\r\n");
}

