//! 电源管理 (PM).
//! 关机/休眠/CPU调频桩.

use crate::serial;

pub fn pm_poweroff() -> ! {
    serial::log("\r\n>> LYSOR OS powering off...\r\n");
    unsafe {
        core::arch::asm!("cli");
        core::arch::asm!("out dx, al", in("dx") 0x604u16, in("al") 0xCu8);
        loop { core::arch::asm!("hlt"); }
    }
}

pub fn pm_reboot() -> ! {
    serial::log("\r\n>> LYSOR OS rebooting...\r\n");
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x64u16, in("al") 0xFEu8);
        loop { core::arch::asm!("hlt"); }
    }
}

pub fn pm_init() {
    serial::log("   pm: power management ready (poweroff/reboot)\r\n");
}
