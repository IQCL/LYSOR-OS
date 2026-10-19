//! udevd 鈥?璁惧鐑彃鎷旀湇鍔?
//! 鎵弿纭欢, 鍔犺浇椹卞姩, 鍒涘缓璁惧鏂囦欢.

use crate::serial;

const MAX_DEVICES: usize = 16;

#[derive(Clone, Copy)]
pub struct DeviceEntry {
    pub name: [u8; 24],
    pub name_len: usize,
    pub dev_type: u8, // 0=block, 1=char, 2=net
}

impl DeviceEntry {
    const fn new() -> Self {
        DeviceEntry { name: [0u8;24], name_len: 0, dev_type: 0 }
    }
}

static mut DEVICES: [DeviceEntry; MAX_DEVICES] = [DeviceEntry::new(); MAX_DEVICES];

pub fn udev_add(name: &str, dev_type: u8) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_DEVICES {
            if DEVICES[i].name_len == 0 {
                let b = name.as_bytes();
                let n = b.len().min(24);
                DEVICES[i].name[..n].copy_from_slice(&b[..n]);
                DEVICES[i].name_len = n;
                DEVICES[i].dev_type = dev_type;
                return Ok(i);
            }
        }
        Err("no udev slot")
    }
}

pub fn udev_init() {
    // Register known devices
    udev_add("ttyS0", 1).ok();
    udev_add("fb0", 1).ok();
    serial::log("   udevd: device manager ready (16 slots)\r\n");
}

