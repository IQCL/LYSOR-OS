//! CMOS RTC (real-time clock) driver.
//! Real implementation: reads the RTC registers through ports 0x70/0x71,
//! waits for update-in-progress to clear, disables NMI during reads, and
//! returns BCD-decoded wall-clock time. Powers the Clock app and taskbar.

use crate::serial;

const RTC_ADDR: u16 = 0x70;
const RTC_DATA: u16 = 0x71;

// Register addresses
const RTC_SECONDS: u8 = 0x00;
const RTC_MINUTES: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_CENTURY: u8 = 0x32;
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

unsafe fn inb(port: u16) -> u8 {
    let r: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") r);
    r
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val);
}

/// Read a single RTC register (BCD unless binary mode set).
unsafe fn rtc_read(reg: u8) -> u8 {
    // Disable NMI while selecting the register (bit 7 = 1).
    outb(RTC_ADDR, reg | 0x80);
    inb(RTC_DATA)
}

/// Wall-clock time.
#[derive(Clone, Copy)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn bcd(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0F)
}

/// Read the current wall-clock time from the CMOS RTC.
pub fn ly_rtc_read() -> RtcTime {
    unsafe {
        // Wait until the RTC is not updating.
        loop {
            outb(RTC_ADDR, RTC_STATUS_A | 0x80);
            if inb(RTC_DATA) & 0x80 == 0 { break; }
        }
        // Register B bit 2: 0 = BCD, 1 = binary.
        outb(RTC_ADDR, RTC_STATUS_B | 0x80);
        let bin_mode = inb(RTC_DATA) & 0x04 != 0;

        let mut sec = rtc_read(RTC_SECONDS);
        let mut min = rtc_read(RTC_MINUTES);
        let mut hr = rtc_read(RTC_HOURS);
        let mut day = rtc_read(RTC_DAY);
        let mut mon = rtc_read(RTC_MONTH);
        let mut yr = rtc_read(RTC_YEAR);
        let cen = rtc_read(RTC_CENTURY);

        if !bin_mode {
            sec = bcd(sec);
            min = bcd(min);
            hr = bcd(hr & 0x7F); // clear 12/24 flag bit
            day = bcd(day);
            mon = bcd(mon);
            yr = bcd(yr);
        }
        let century = if cen != 0 { bcd(cen) as u16 * 100 } else { 2000 };
        RtcTime {
            year: century + yr as u16,
            month: mon,
            day,
            hour: hr,
            minute: min,
            second: sec,
        }
    }
}

/// Format a two-digit number as ASCII into `out` at `pos`.
fn two_digits(v: u8, out: &mut [u8], pos: usize) {
    out[pos] = b'0' + (v / 10);
    out[pos + 1] = b'0' + (v % 10);
}

/// Human-readable "YYYY/MM/DD HH:MM" into a 17-byte buffer.
pub fn ly_rtc_format(out: &mut [u8]) -> usize {
    let t = ly_rtc_read();
    let y = t.year;
    out[0] = b'0' + ((y / 1000) % 10) as u8;
    out[1] = b'0' + ((y / 100) % 10) as u8;
    out[2] = b'0' + ((y / 10) % 10) as u8;
    out[3] = b'0' + (y % 10) as u8;
    out[4] = b'/';
    two_digits(t.month, out, 5);
    out[7] = b'/';
    two_digits(t.day, out, 8);
    out[10] = b' ';
    two_digits(t.hour, out, 11);
    out[13] = b':';
    two_digits(t.minute, out, 14);
    16
}
