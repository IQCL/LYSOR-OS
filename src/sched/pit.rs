//! 8253/8254 PIT timer (channel 0) for preemptive scheduling ticks.

use crate::pic;

const PIT_CH0_DATA: u16 = 0x40;
const PIT_CMD: u16 = 0x43;

/// Timer frequency: 100 Hz = 10ms tick.
const TICK_HZ: u32 = 100;
const BASE_FREQ: u32 = 1193182;
const DIVISOR: u32 = BASE_FREQ / TICK_HZ;

static mut TICKS: u64 = 0;

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("al") val, in("dx") port, options(nomem, nostack));
}

/// Initialize PIT channel 0 in rate-generator mode.
pub fn ly_pit_init() {
    unsafe {
        outb(PIT_CMD, 0x36); // channel 0, lsb+msb, mode 3 (square wave)
        outb(PIT_CH0_DATA, (DIVISOR & 0xFF) as u8);
        outb(PIT_CH0_DATA, ((DIVISOR >> 8) & 0xFF) as u8);
        pic::pic_unmask(0); // unmask IRQ0
    }
}

/// Called from IRQ0 handler. Returns true each call; increments tick counter.
pub fn ly_pit_tick() -> u64 {
    unsafe {
        TICKS += 1;
        TICKS
    }
}

pub fn ly_pit_ticks() -> u64 {
    unsafe { TICKS }
}
