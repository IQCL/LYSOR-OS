//! PS/2 鼠标驱动 (i8042 aux port).
//! 真实实现：aux 通道读 3 字节数据包 [buttons|dx|dy]，按下滚轮时第四个字节
//! 被跳过（QEMU 默认 2 键鼠标），组装为相对位移 + 按键状态。

use crate::serial;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;
const PS2_CMD: u16 = 0x64;
const PS2_AUX: u16 = 0x60;

unsafe fn inb(port: u16) -> u8 {
    let r: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") r);
    r
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val);
}

/// Mouse state: relative movement since last read + buttons.
#[derive(Clone, Copy)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left: bool,
    pub right: bool,
}

static mut MOUSE_ENABLED: bool = false;
static mut PKT: [u8; 3] = [0; 3];
static mut PKT_N: usize = 0;
static mut MOUSE_LEFT: bool = false;
static mut MOUSE_RIGHT: bool = false;
static mut ACC_DX: i32 = 0;
static mut ACC_DY: i32 = 0;

/// Initialize the PS/2 mouse: enable aux device, enable data reporting.
pub fn ps2mouse_init() {
    unsafe {
        // Wait for input buffer empty, then write to command port.
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_CMD, 0xA8); // enable aux device
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_CMD, 0x60); // write to controller command byte
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        // Enable IRQ12 + aux: 0x47 (IRQ12=0x02, aux clock=0x04, translation=0x40)
        outb(PS2_AUX, 0x47);
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        // Set default sample rate 100Hz, resolution 4, scaling 1:1
        outb(PS2_AUX, 0xF3); // set sample rate
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_AUX, 100);
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_AUX, 0xE8); // set resolution
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_AUX, 3);
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        outb(PS2_AUX, 0xF4); // enable data reporting
        for _ in 0..100 {
            if inb(PS2_STATUS) & 2 == 0 { break; }
        }
        // Flush stale bytes.
        for _ in 0..16 {
            if inb(PS2_STATUS) & 1 != 0 { let _ = inb(PS2_DATA); }
        }
        MOUSE_ENABLED = true;
    }
    serial::log("   ps2mouse: mouse ready (3-byte packets, aux port)\r\n");
}

/// Accumulate raw bytes into packets; return true when a full packet arrived.
unsafe fn pump() -> bool {
    loop {
        let st = inb(PS2_STATUS);
        if st & 1 == 0 {
            break; // no output data at all
        }
        if st & 0x20 == 0 {
            // OBF is a KEYBOARD byte (status bit5=0): leave it for ps2kbd
            break;
        }
        let byte = inb(PS2_DATA);
        PKT[PKT_N] = byte;
        PKT_N += 1;
        if PKT_N == 3 {
            // Packet valid when bit 3 of byte 0 is set (always for standard mice).
            if PKT[0] & 0x08 != 0 {
                let mut dx = (PKT[1] as i8) as i32;
                let mut dy = (PKT[2] as i8) as i32;
                MOUSE_LEFT = PKT[0] & 0x01 != 0;
                MOUSE_RIGHT = PKT[0] & 0x02 != 0;
                // Sign-extend overflow bits (bit 4/5 of byte 0).
                if PKT[0] & 0x10 != 0 { dx += 256; }
                if PKT[0] & 0x20 != 0 { dy -= 256; }
                ACC_DX += dx;
                ACC_DY += dy;
                PKT_N = 0;
                return true;
            }
            PKT_N = 0; // bad sync, restart
        }
    }
    false
}

/// Read accumulated mouse state (relative delta since last poll).
pub fn ps2mouse_read() -> Option<MouseState> {
    unsafe {
        if !MOUSE_ENABLED { return None; }
        let got = pump();
        if ACC_DX == 0 && ACC_DY == 0 && !got {
            return None;
        }
        let st = MouseState { dx: ACC_DX, dy: ACC_DY, left: MOUSE_LEFT, right: MOUSE_RIGHT };
        ACC_DX = 0;
        ACC_DY = 0;
        Some(st)
    }
}
