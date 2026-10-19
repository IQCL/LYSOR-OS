//! PCI 总线枚举.
//! 扫描 bus 0 的所有设备，打印 vendor/device ID.

use crate::serial;

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

unsafe fn inl(port: u16) -> u32 {
    let r: u32;
    core::arch::asm!("in eax, dx", in("dx") port, out("eax") r);
    r
}

unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") val);
}

unsafe fn pci_read(bus: u32, dev: u32, func: u32, reg: u32) -> u32 {
    let addr = (bus << 16) | (dev << 11) | (func << 8) | (reg & 0xFC) | 0x80000000;
    outl(PCI_CONFIG_ADDRESS, addr);
    inl(PCI_CONFIG_DATA)
}

pub fn pci_enumerate() {
    unsafe {
        serial::log("   pci: scanning bus 0...\r\n");
        let mut count = 0;
        for dev in 0..32 {
            let vendor = pci_read(0, dev, 0, 0) & 0xFFFF;
            if vendor == 0xFFFF || vendor == 0 { continue; }
            let device = (pci_read(0, dev, 0, 0) >> 16) & 0xFFFF;
            serial::log("   pci: dev ");
            serial::log_u(dev as u64);
            serial::log(" vendor=0x");
            serial::log_hex(vendor as u64);
            serial::log(" device=0x");
            serial::log_hex(device as u64);
            serial::log("\r\n");
            count += 1;
        }
        serial::log("   pci: found ");
        serial::log_u(count as u64);
        serial::log(" devices\r\n");
    }
}
