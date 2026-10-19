//! 16550 UART serial driver (COM1, port 0x3F8)

const COM1: u16 = 0x3F8;

// Real output capture: while armed, serial bytes go into the buffer instead
// of the COM1 port. Used by the GUI Terminal app to show real command
// output. Single-armed at a time (terminal app only).
static mut CAP_BUF: *mut u8 = core::ptr::null_mut();
static mut CAP_LEN: usize = 0;
static mut CAP_MAX: usize = 0;

/// Arm output capture into `buf` (up to `max` bytes).
pub fn serial_capture_start(buf: *mut u8, max: usize) {
    unsafe {
        CAP_BUF = buf;
        CAP_LEN = 0;
        CAP_MAX = max;
    }
}

/// Stop capture; returns how many bytes were captured.
pub fn serial_capture_stop() -> usize {
    unsafe {
        CAP_BUF = core::ptr::null_mut();
        let n = CAP_LEN;
        CAP_LEN = 0;
        CAP_MAX = 0;
        n
    }
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("al") val, in("dx") port, options(nomem, nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack));
    v
}

pub fn init_serial() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

fn is_ready() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

pub fn putc(c: u8) {
    unsafe {
        if !CAP_BUF.is_null() {
            if CAP_LEN < CAP_MAX {
                *CAP_BUF.add(CAP_LEN) = c;
                CAP_LEN += 1;
            }
            return;
        }
    }
    while !is_ready() {}
    unsafe { outb(COM1, c) };
}

pub fn serial_writestr(s: &[u8]) {
    for &b in s {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}

pub fn log(s: &str) {
    serial_writestr(s.as_bytes());
}

pub fn log_u(n: u64) {
    if n == 0 {
        putc(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    let mut v = n;
    while v > 0 && len < 20 {
        buf[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        putc(buf[i]);
    }
}

#[inline(never)]
pub fn log_i(n: i64) {
    if n < 0 {
        putc(b'-');
        log_u((-n) as u64);
    } else {
        log_u(n as u64);
    }
}

#[inline(never)]
pub fn log_hex(n: u64) {
    // Mirror log_u's structure exactly: digit buffer, little-endian fill,
    // reverse print. Proven to work (log_u prints correctly).
    let mut buf = [0u8; 16];
    let mut len = 0usize;
    let mut v = n;
    while v > 0 && len < 16 {
        let nib = (v & 0xF) as u8;
        buf[len] = if nib < 10 { b'0' + nib } else { b'A' + nib - 10 };
        v >>= 4;
        len += 1;
    }
    if len == 0 {
        putc(b'0');
        return;
    }
    for i in (0..len).rev() {
        putc(buf[i]);
    }
}

#[allow(dead_code)]
pub fn serial_write_u32(n: u32) {
    log_u(n as u64);
}
