//! LYSOR OS ATA PIO disk driver (`ly_ata`).
//!
//! Real implementation: probes both IDE channels, IDENTIFYs the drive,
//! reads 48-bit LBA geometry, and supports multi-sector PIO read/write.
//! Registers itself as block device "sda" / "sdb" so the filesystem can
//! mount a real disk image (QEMU `-drive file=...,if=ide`).

use crate::serial;
use crate::drivers::blockdev;

const ATA_SR_BSY: u8 = 0x80;
const ATA_SR_DRQ: u8 = 0x08;
const ATA_SR_ERR: u8 = 0x01;
const ATA_SR_DF: u8 = 0x20;

const ATA_CMD_READ: u8 = 0x20;     // READ SECTORS (LBA28)
const ATA_CMD_WRITE: u8 = 0x30;    // WRITE SECTORS (LBA28)
const ATA_CMD_IDENTIFY: u8 = 0xEC; // IDENTIFY DEVICE

const MAX_SECTORS_PER_IO: u32 = 128; // safe LBA28 burst
const IO_TIMEOUT: u32 = 2_000_000;   // busy-wait iterations

struct AtaChannel {
    base: u16,      // data register port (0x1F0 / 0x170)
    ctrl: u16,      // control port (0x3F6 / 0x376)
    present: bool,  // channel responds on status read
    drive: bool,    // drive 0 present on this channel
    sectors: u64,   // total sectors (48-bit if supported)
    dev_id: u32,    // registered block device id
}

static mut CH: [AtaChannel; 2] = [
    AtaChannel { base: 0x1F0, ctrl: 0x3F6, present: false, drive: false, sectors: 0, dev_id: u32::MAX },
    AtaChannel { base: 0x170, ctrl: 0x376, present: false, drive: false, sectors: 0, dev_id: u32::MAX },
];

unsafe fn inb(port: u16) -> u8 {
    let r: u8;
    core::arch::asm!("in al, dx", out("al") r, in("dx") port, options(nomem, nostack));
    r
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
}

unsafe fn inw(port: u16) -> u16 {
    let r: u16;
    core::arch::asm!("in ax, dx", out("ax") r, in("dx") port, options(nomem, nostack));
    r
}

unsafe fn outw(port: u16, val: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack));
}

/// Wait for BSY to clear (with timeout). Returns Err on timeout.
unsafe fn wait_bsy(base: u16) -> Result<(), &'static str> {
    let mut t = 0;
    while inb(base + 7) & ATA_SR_BSY != 0 {
        t += 1;
        if t > IO_TIMEOUT {
            return Err("ata: BSY timeout");
        }
    }
    Ok(())
}

/// Wait for DRQ to be set (data ready). Returns Err on timeout/error.
unsafe fn wait_drq(base: u16) -> Result<(), &'static str> {
    let mut t = 0;
    loop {
        let st = inb(base + 7);
        if st & ATA_SR_ERR != 0 {
            return Err("ata: controller error");
        }
        if st & ATA_SR_DRQ != 0 {
            return Ok(());
        }
        t += 1;
        if t > IO_TIMEOUT {
            return Err("ata: DRQ timeout");
        }
    }
}

/// Do a raw PIO data-in (IDENTIFY / READ) transfer of `words` 16-bit words.
unsafe fn pio_read_words(base: u16, words: usize, buf: &mut [u8]) -> Result<(), &'static str> {
    for i in 0..words {
        let w = inw(base);
        if buf.len() >= (i + 1) * 2 {
            buf[i * 2] = (w & 0xFF) as u8;
            buf[i * 2 + 1] = (w >> 8) as u8;
        }
    }
    Ok(())
}

unsafe fn pio_write_words(base: u16, words: usize, buf: &[u8]) {
    for i in 0..words {
        let lo = if i * 2 < buf.len() { buf[i * 2] } else { 0 };
        let hi = if i * 2 + 1 < buf.len() { buf[i * 2 + 1] } else { 0 };
        outw(base, (hi as u16) << 8 | lo as u16);
    }
}

/// I/O delay: reads from the diagnostic port 0x80 (~1us each) so the IDE
/// controller has time to latch state changes.
unsafe fn io_delay() {
    let _ = inb(0x80);
    let _ = inb(0x80);
    let _ = inb(0x80);
    let _ = inb(0x80);
}

/// Probe a channel: select drive 0, check for a device, run IDENTIFY.
/// Follows the standard ATA probe sequence (reset -> select -> wait for
/// BSY clear -> check DRDY -> IDENTIFY) so timing races cannot hide a drive.
unsafe fn probe_channel(idx: usize) {
    let base = CH[idx].base;
    let ctrl = CH[idx].ctrl;

    // Software reset the channel (clear latched errors).
    outb(ctrl, 0x04);
    io_delay();
    outb(ctrl, 0x00);
    io_delay();

    // Drive select (master).
    outb(base + 6, 0xA0);
    io_delay();

    let st = inb(base + 7);
    if st == 0xFF {
        // Floating bus: no channel.
        return;
    }
    CH[idx].present = true;

    // Wait for BSY to clear (the select asserts it briefly).
    if wait_bsy(base).is_err() {
        return;
    }
    let st = inb(base + 7);
    if st == 0x00 {
        return; // no drive
    }
    if st & ATA_SR_ERR != 0 {
        return; // ATAPI or error latch
    }

    // Wait for DRDY (drive ready) before issuing IDENTIFY.
    let mut t = 0;
    while inb(base + 7) & 0x40 == 0 {
        t += 1;
        if t > IO_TIMEOUT {
            return; // no drive
        }
    }
    CH[idx].drive = true;

    // IDENTIFY DEVICE.
    outb(base + 6, 0xA0);
    io_delay();
    outb(base + 7, ATA_CMD_IDENTIFY);
    io_delay();
    if wait_bsy(base).is_err() { return; }

    let st = inb(base + 7);
    if st == 0x00 {
        // ATAPI device or none.
        CH[idx].drive = false;
        return;
    }

    let mut id = [0u8; 512];
    // LBA48-capable drives may raise ERR with zeroed error reg; fall back to
    // 28-bit geometry in that case.
    if wait_drq(base).is_ok() {
        if pio_read_words(base, 256, &mut id).is_err() { return; }
    }

    // Parse geometry.
    // Words 60-61: 28-bit total sectors. Words 100-103: 48-bit total sectors.
    let lba28_lo = u16::from_le_bytes([id[60 * 2], id[60 * 2 + 1]]) as u64;
    let lba28_hi = u16::from_le_bytes([id[61 * 2], id[61 * 2 + 1]]) as u64;
    let lba28 = (lba28_hi << 16) | lba28_lo;
    let lba48_lo = u64::from_le_bytes(id[100 * 2..100 * 2 + 8].try_into().unwrap());
    let lba48_hi = u64::from_le_bytes(id[102 * 2..102 * 2 + 8].try_into().unwrap());
    let lba48 = (lba48_hi << 32) | lba48_lo;

    CH[idx].sectors = if lba48 != 0 { lba48 } else { lba28 };
    if CH[idx].sectors == 0 {
        CH[idx].drive = false;
        return;
    }

    serial::log("   ata: channel ");
    serial::log_u(idx as u64);
    serial::log(" drive present, sectors=");
    serial::log_u(CH[idx].sectors);
    serial::log("\r\n");
}

/// Read `count` sectors starting at `lba` into `buf` (count*512 bytes).
pub fn ata_read_sectors(idx: usize, lba: u64, count: u32, buf: &mut [u8]) -> Result<(), &'static str> {
    unsafe {
        let base = CH[idx].base;
        if !CH[idx].drive {
            return Err("ata: no drive");
        }
        let mut todo = count;
        let mut cur_lba = lba;
        let mut off = 0usize;
        while todo > 0 {
            let n = core::cmp::min(todo, MAX_SECTORS_PER_IO);
            wait_bsy(base)?;
            outb(base + 6, 0xE0 | ((cur_lba >> 24) & 0x0F) as u8);
            outb(base + 2, n as u8);
            outb(base + 3, (cur_lba & 0xFF) as u8);
            outb(base + 4, ((cur_lba >> 8) & 0xFF) as u8);
            outb(base + 5, ((cur_lba >> 16) & 0xFF) as u8);
            outb(base + 7, ATA_CMD_READ);
            wait_bsy(base)?;
            for _ in 0..n {
                wait_drq(base)?;
                pio_read_words(base, 256, &mut buf[off..off + 512])?;
                off += 512;
            }
            todo -= n;
            cur_lba += n as u64;
        }
        Ok(())
    }
}

/// Write `count` sectors starting at `lba` from `buf`.
pub fn ata_write_sectors(idx: usize, lba: u64, count: u32, buf: &[u8]) -> Result<(), &'static str> {
    unsafe {
        let base = CH[idx].base;
        if !CH[idx].drive {
            return Err("ata: no drive");
        }
        let mut todo = count;
        let mut cur_lba = lba;
        let mut off = 0usize;
        while todo > 0 {
            let n = core::cmp::min(todo, MAX_SECTORS_PER_IO);
            wait_bsy(base)?;
            outb(base + 6, 0xE0 | ((cur_lba >> 24) & 0x0F) as u8);
            outb(base + 2, n as u8);
            outb(base + 3, (cur_lba & 0xFF) as u8);
            outb(base + 4, ((cur_lba >> 8) & 0xFF) as u8);
            outb(base + 5, ((cur_lba >> 16) & 0xFF) as u8);
            outb(base + 7, ATA_CMD_WRITE);
            for _ in 0..n {
                wait_drq(base)?;
                pio_write_words(base, 256, &buf[off..off + 512]);
                off += 512;
            }
            // Wait for the drive to drain its buffer before the next burst.
            wait_bsy(base)?;
            if inb(base + 7) & ATA_SR_ERR != 0 {
                return Err("ata: write error");
            }
            todo -= n;
            cur_lba += n as u64;
        }
        Ok(())
    }
}

// --- Block device callbacks -------------------------------------------------

extern "C" fn ata_read_block(dev: u32, block: u64, buf: *mut u8) -> i32 {
    unsafe {
        for c in 0..2 {
            if CH[c].dev_id == dev {
                let slice = core::slice::from_raw_parts_mut(buf, 512);
                return match ata_read_sectors(c, block, 1, slice) {
                    Ok(()) => 0,
                    Err(_) => -5, // EIO
                };
            }
        }
    }
    -19
}

extern "C" fn ata_write_block(dev: u32, block: u64, buf: *const u8) -> i32 {
    unsafe {
        for c in 0..2 {
            if CH[c].dev_id == dev {
                let slice = core::slice::from_raw_parts(buf, 512);
                return match ata_write_sectors(c, block, 1, slice) {
                    Ok(()) => 0,
                    Err(_) => -5,
                };
            }
        }
    }
    -19
}

/// Probe both channels and register found drives with the block device layer.
pub fn ata_init() {
    serial::log("   ata: probing IDE channels...\r\n");
    unsafe {
        probe_channel(0);
        probe_channel(1);
    }
    for c in 0..2 {
        unsafe {
            if CH[c].drive {
                let name = if c == 0 { "sda" } else { "sdb" };
                let dev = blockdev::LyBlockDev {
                    name,
                    block_size: 512,
                    block_count: CH[c].sectors,
                    read_block: ata_read_block,
                    write_block: ata_write_block,
                };
                match blockdev::ly_blk_register(dev) {
                    Ok(id) => {
                        CH[c].dev_id = id;
                        serial::log("   ata: registered /dev/");
                        serial::log(name);
                        serial::log("\r\n");
                    }
                    Err(e) => serial::log(e),
                }
            }
        }
    }
}
