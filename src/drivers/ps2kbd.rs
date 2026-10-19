//! PS/2 键盘驱动 (i8042).
//! 读取 scancode set 1, 翻译为 ASCII.

use crate::serial;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;

unsafe fn inb(port: u16) -> u8 {
    let r: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") r);
    r
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val);
}

/// Special key codes beyond ASCII (returned as event codes >= 0x80).
pub const K_ENTER: u8 = 0x80;
pub const K_ESC: u8 = 0x81;
pub const K_TAB: u8 = 0x82;
pub const K_BACKSPACE: u8 = 0x83;
pub const K_UP: u8 = 0x84;
pub const K_DOWN: u8 = 0x85;
pub const K_LEFT: u8 = 0x86;
pub const K_RIGHT: u8 = 0x87;
pub const K_CAPS: u8 = 0x88;
pub const K_WIN: u8 = 0x89;
pub const K_ALT: u8 = 0x8A;
pub const K_F4: u8 = 0x8B;
pub const K_F10: u8 = 0x8C; // Shift+F10: context-menu key (Windows)
pub const K_HOME: u8 = 0x8D;
pub const K_END: u8 = 0x8E;
pub const K_INS: u8 = 0x8F;
pub const K_DEL: u8 = 0x90;
pub const K_PGUP: u8 = 0x91;
pub const K_PGDN: u8 = 0x92;

/// Translate scancode set 1 to an ASCII char (lowercase, unshifted).
fn scancode_to_ascii(sc: u8) -> Option<u8> {
    match sc {
        0x02 => Some(b'1'), 0x03 => Some(b'2'), 0x04 => Some(b'3'),
        0x05 => Some(b'4'), 0x06 => Some(b'5'), 0x07 => Some(b'6'),
        0x08 => Some(b'7'), 0x09 => Some(b'8'), 0x0A => Some(b'9'),
        0x0B => Some(b'0'),
        0x0C => Some(b'-'), 0x0D => Some(b'='),
        0x10 => Some(b'q'), 0x11 => Some(b'w'), 0x12 => Some(b'e'),
        0x13 => Some(b'r'), 0x14 => Some(b't'), 0x15 => Some(b'y'),
        0x16 => Some(b'u'), 0x17 => Some(b'i'), 0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x1E => Some(b'a'), 0x1F => Some(b's'), 0x20 => Some(b'd'),
        0x21 => Some(b'f'), 0x22 => Some(b'g'), 0x23 => Some(b'h'),
        0x24 => Some(b'j'), 0x25 => Some(b'k'), 0x26 => Some(b'l'),
        0x2B => Some(b'z'), 0x2D => Some(b'x'), 0x2E => Some(b'c'),
        0x2F => Some(b'v'), 0x30 => Some(b'b'), 0x31 => Some(b'n'),
        0x32 => Some(b'm'), 0x39 => Some(b' '),
        _ => None,
    }
}

/// Shifted (uppercase / symbols) translation for the same scancodes.
fn scancode_to_ascii_shift(sc: u8) -> Option<u8> {
    match sc {
        0x02 => Some(b'!'), 0x03 => Some(b'@'), 0x04 => Some(b'#'),
        0x05 => Some(b'$'), 0x06 => Some(b'%'), 0x07 => Some(b'^'),
        0x08 => Some(b'&'), 0x09 => Some(b'*'), 0x0A => Some(b'('),
        0x0B => Some(b')'),
        0x0C => Some(b'_'), 0x0D => Some(b'+'),
        0x10 => Some(b'Q'), 0x11 => Some(b'W'), 0x12 => Some(b'E'),
        0x13 => Some(b'R'), 0x14 => Some(b'T'), 0x15 => Some(b'Y'),
        0x16 => Some(b'U'), 0x17 => Some(b'I'), 0x18 => Some(b'O'),
        0x19 => Some(b'P'),
        0x1E => Some(b'A'), 0x1F => Some(b'S'), 0x20 => Some(b'D'),
        0x21 => Some(b'F'), 0x22 => Some(b'G'), 0x23 => Some(b'H'),
        0x24 => Some(b'J'), 0x25 => Some(b'K'), 0x26 => Some(b'L'),
        0x2B => Some(b'Z'), 0x2D => Some(b'X'), 0x2E => Some(b'C'),
        0x2F => Some(b'V'), 0x30 => Some(b'B'), 0x31 => Some(b'N'),
        0x32 => Some(b'M'), 0x39 => Some(b' '),
        _ => None,
    }
}

static mut SHIFT: bool = false;
static mut CAPS: bool = false;
static mut E0: bool = false;
static mut ALT: bool = false;

/// Is Alt currently held (for Alt+F4 etc.)?
pub fn ly_kbd_alt() -> bool {
    unsafe { ALT }
}

/// Current Caps Lock state (toggled by the real PS/2 key).
pub fn ly_kbd_caps() -> bool {
    unsafe { CAPS }
}

/// Try to read a key event from PS/2 keyboard.
/// Returns a key code: ASCII char, or a K_* special code (>= 0x80),
/// or None when no key is pending. Tracks Shift for uppercase.
pub fn ps2_read_key() -> Option<u8> {
    unsafe {
        let st = inb(PS2_STATUS);
        if st & 1 == 0 { return None; } // no data
        if st & 0x20 != 0 { return None; } // AUX (mouse) byte: leave for ps2mouse
        let sc = inb(PS2_DATA);
        if sc == 0xE0 {
            E0 = true;
            return None;
        }
        if E0 {
            E0 = false;
            match sc {
                0x5B => return Some(K_WIN),  // left Win press
                0xDB => return None,         // left Win release
                0x5C => return Some(K_WIN),  // right Win press
                0xDC => return None,         // right Win release
                // Extended navigation keys (E0-prefixed, scancode set 1).
                0x48 => return Some(K_UP),
                0x50 => return Some(K_DOWN),
                0x4B => return Some(K_LEFT),
                0x4D => return Some(K_RIGHT),
                0x47 => return Some(K_HOME),
                0x4F => return Some(K_END),
                0x52 => return Some(K_INS),
                0x53 => return Some(K_DEL),
                0x49 => return Some(K_PGUP),
                0x51 => return Some(K_PGDN),
                _ => return None,
            }
        }
        match sc {
            0x2A => { SHIFT = true; return None; }      // left shift press
            0xAA => { SHIFT = false; return None; }     // left shift release
            0x36 => { SHIFT = true; return None; }      // right shift press
            0xB6 => { SHIFT = false; return None; }     // right shift release
            0x38 => { ALT = true; return None; }       // left alt press
            0xB8 => { ALT = false; return None; }      // left alt release
            0x3E => return Some(K_F4),                 // F4 (Alt+F4 = shutdown)
            0x44 => {
                if SHIFT {
                    return Some(K_F10);                 // Shift+F10 = context menu
                }
                return None;
            }
            0x3A => {                                   // caps lock press (toggle)
                CAPS = !CAPS;
                serial::log("   kbd: CAPS LOCK ");
                serial::log(if CAPS { "ON" } else { "OFF" });
                serial::log("\r\n");
                return None;
            }
            0xBA => return None,                         // caps lock release
            0x1C => return Some(K_ENTER),
            0x01 => return Some(K_ESC),
            0x0F => return Some(K_TAB),
            0x0E => return Some(K_BACKSPACE),
            0x48 => return Some(K_UP),
            0x50 => return Some(K_DOWN),
            0x4B => return Some(K_LEFT),
            0x4D => return Some(K_RIGHT),
            _ => {}
        }
        if sc & 0x80 != 0 { return None; } // key release, ignore
        let ch = if SHIFT {
            scancode_to_ascii_shift(sc).or_else(|| scancode_to_ascii(sc))
        } else {
            scancode_to_ascii(sc)
        };
        // Caps Lock inverts the case of letters only (symbols unaffected).
        if let Some(c) = ch {
            if CAPS && c.is_ascii_alphabetic() {
                return Some(if c.is_ascii_lowercase() { c - b'a' + b'A' } else { c - b'A' + b'a' });
            }
        }
        ch
    }
}

/// Raw i8042 status register (0x64): bit0 = output buffer full.
pub fn ps2_status() -> u8 {
    unsafe { inb(PS2_STATUS) }
}

/// Initialize PS/2 keyboard.
pub fn ps2_init() {
    unsafe {
        // Flush any pending data
        for _ in 0..16 {
            if inb(PS2_STATUS) & 1 != 0 { let _ = inb(PS2_DATA); }
        }
    }
    serial::log("   ps2: keyboard ready (scancode set 1)\r\n");
}
