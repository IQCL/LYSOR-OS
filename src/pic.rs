//! PIC8259 interrupt controller remap.

use core::arch::asm;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

pub const OFFSET1: u8 = 32;
pub const OFFSET2: u8 = 40;

unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("al") val, in("dx") port, options(nomem, nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack));
    v
}

pub fn init_pic() {
    unsafe {
        outb(PIC1_CMD, 0x11);
        outb(PIC2_CMD, 0x11);
        outb(PIC1_DATA, OFFSET1);
        outb(PIC2_DATA, OFFSET2);
        outb(PIC1_DATA, 0x04);
        outb(PIC2_DATA, 0x02);
        outb(PIC1_DATA, 0x01);
        outb(PIC2_DATA, 0x01);
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }
}

pub unsafe fn pic_eoi(irq: u8) {
    if irq >= 8 {
        outb(PIC2_CMD, 0x20);
    }
    outb(PIC1_CMD, 0x20);
}

#[allow(dead_code)]
pub unsafe fn pic_unmask(irq: u8) {
    let (port, bit) = if irq < 8 {
        (PIC1_DATA, irq)
    } else {
        (PIC2_DATA, irq - 8)
    };
    let mask = inb(port) & !(1 << bit);
    outb(port, mask);
}
