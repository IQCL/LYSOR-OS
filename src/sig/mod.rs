//! LYSOR OS 包签名验证 (`ly_pkg_sig`) — 微内核信任模型。
//!
//! 信任模型：内置证书表（8 槽），每张证书带算法标识——
//!   - ALGO_SM2 (0)：SM3 摘要 + SM2 (GB/T 32918) 验签（国密轨，优先）
//!   - ALGO_ED25519 (1)：SHA-512 摘要 + Ed25519 (RFC 8032) 验签（现代轨）
//!
//! 密码能力由可选 kcomponent "lycomp_crypto" 承载。**该组件属于私有
//! 完整版仓库（full build），不含在本公开微内核构建中**；因此本模块
//! 始终处于"密码组件未加载"状态：显式拒绝一切验证请求，绝不静默放行
//! （无保护桩）。完整版内核加载 crypto KC 后，同样的接口会走真实
//! SM2/Ed25519 验签链。

use crate::serial;

/// 国密轨算法标识（SM2/SM3，GB/T 32918/32905）。
pub const ALGO_SM2: u8 = 0;
/// 现代轨算法标识（Ed25519 + SHA-512，RFC 8032）。
pub const ALGO_ED25519: u8 = 1;

/// 信任的开发者公钥槽。
#[derive(Clone, Copy)]
pub struct LyDevCert {
    pub dev_id: [u8; 16],
    pub algo: u8,
    pub key: [u8; 64], // SM2: x||y 各 32B；Ed25519: 32B 公钥放 key[..32]
    pub revoked: bool,
}

impl LyDevCert {
    const fn empty() -> Self {
        LyDevCert { dev_id: [0u8; 16], algo: 0xFF, key: [0u8; 64], revoked: false }
    }
}

const MAX_CERTS: usize = 8;
static mut CERTS: [LyDevCert; MAX_CERTS] = [LyDevCert::empty(); MAX_CERTS];

/// 注册开发者证书（可信根）。key64 按 algo 解释。
/// 证书表属于微内核信任根：即使没有 crypto KC，策略仍可先行注册，
/// 完整版加载 KC 后即生效。
pub fn ly_cert_add(dev_id: &[u8], algo: u8, key64: &[u8; 64]) -> Result<usize, &'static str> {
    unsafe {
        for i in 0..MAX_CERTS {
            if CERTS[i].key == [0u8; 64] {
                let n = dev_id.len().min(16);
                CERTS[i].dev_id[..n].copy_from_slice(&dev_id[..n]);
                CERTS[i].algo = algo;
                CERTS[i].key = *key64;
                CERTS[i].revoked = false;
                return Ok(i);
            }
        }
        Err("cert store full")
    }
}

/// 吊销证书。
pub fn ly_cert_revoke(idx: usize) {
    unsafe { if idx < MAX_CERTS { CERTS[idx].revoked = true; } }
}

/// 统一验签：algo 指定算法，msg 为被签名内容，sig64 为 64 字节签名。
/// 本公开构建不含 crypto KC：恒返回 false（显式拒绝，绝不静默放行）。
fn verify_with_certs(algo: u8, _msg: &[u8], _sig64: &[u8; 64]) -> bool {
    let _ = algo;
    serial::log("   sig: crypto KC absent in public build, verification refuses\r\n");
    false
}

/// 验证 .lyx 可执行文件签名。
/// 布局：`image[0..len]` 为被签内容（整个二进制体），`algo` 指定算法，
/// `sig64` 为 64 字节签名。
pub fn ly_verify_lyx(image: *const u8, len: usize, algo: u8, sig64: &[u8; 64]) -> bool {
    if len == 0 || image.is_null() {
        serial::log("   sig: lyx verify rejected (empty image)\r\n");
        return false;
    }
    unsafe {
        let msg = core::slice::from_raw_parts(image, len);
        verify_with_certs(algo, msg, sig64)
    }
}

/// 验证 .lyap 应用包签名（与 lyx 同一验签链，供 exec 加载时调用）。
pub fn ly_verify_lyap(image: *const u8, len: usize, algo: u8, sig64: &[u8; 64]) -> bool {
    ly_verify_lyx(image, len, algo, sig64)
}

/// 初始化签名子系统。公开构建无 crypto KC：仅声明拒绝语义；
/// 完整版（私有仓库）在此注册内置开发者证书并跑真实签名/验签/篡改自检。
pub fn ly_pkg_sig_init() {
    serial::log("   sig: verifier init (");
    serial::log_u(MAX_CERTS as u64);
    serial::log(" cert slots, dual-track contract)\r\n");
    serial::log("   sig: crypto KC absent (private build); all verification will refuse\r\n");
}
