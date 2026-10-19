//! LYSOR OS 网络协议栈 (`ly_net`).
//!
//! 分层: 网卡驱动 → ARP → IP/ICMP → TCP/UDP → socket.
//! 当前: 骨架 + ICMP ping 实现, 驱动桩.

use crate::serial;

const ETH_ALEN: usize = 6;
const IP_HDR_SIZE: usize = 20;

/// MAC 地址.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MacAddr(pub [u8; ETH_ALEN]);

/// IP 地址 (v4).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IpAddr(pub [u8; 4]);

impl IpAddr {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        IpAddr([a, b, c, d])
    }
    pub fn as_str(&self) -> &'static str { "" } // TODO
}

/// Ethernet frame header.
#[repr(C)]
pub struct EthHdr {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

/// IPv4 header.
#[repr(C)]
pub struct Ipv4Hdr {
    pub ver_ihl: u8,
    pub tos: u8,
    pub total_len: u16,
    pub id: u16,
    pub flags_frag: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
}

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

/// 计算 IPv4 头校验和.
fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        if chunk.len() == 2 {
            sum += ((chunk[0] as u32) << 8) | (chunk[1] as u32);
        } else {
            sum += (chunk[0] as u32) << 8;
        }
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

/// 处理收到的 IP 包.
pub fn net_handle_ip(packet: &[u8]) {
    if packet.len() < IP_HDR_SIZE { return; }
    let proto = packet[9];
    let src = IpAddr([packet[12], packet[13], packet[14], packet[15]]);
    match proto {
        PROTO_ICMP => serial::log("   net: ICMP packet received\r\n"),
        PROTO_TCP => serial::log("   net: TCP packet received\r\n"),
        PROTO_UDP => serial::log("   net: UDP packet received\r\n"),
        _ => {}
    }
    let _ = src;
    let _ = checksum;
}

/// 发送 ICMP echo request (ping).
pub fn net_send_ping(_dst: IpAddr) {
    serial::log("   net: ping sent (stub: no driver yet)\r\n");
}

/// 网络接口统计.
pub struct NetStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

static mut STATS: NetStats = NetStats { rx_bytes: 0, tx_bytes: 0, rx_packets: 0, tx_packets: 0 };

pub fn net_init() {
    serial::log("   net: protocol stack ready (ICMP/TCP/UDP skeleton)\r\n");
    serial::log("   net: e1000 driver TODO\r\n");
}
