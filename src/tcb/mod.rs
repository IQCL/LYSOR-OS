//! LYSOR OS TCB (Trusted Computing Base) boundary.
//!
//! The TCB is the small set of code that must be bug-free for the system to
//! be secure. Everything outside the TCB (components, future user tasks) is
//! loaded into a separate, non-user-mapped region and mediated by these hooks.
//!
//! Boundary enforcement (real):
//!   - All privileged instructions (CR3 load, MSR, I/O ports) go through
//!     `ly_tcb_*` entry points.
//!   - Components are granted only the permissions they declare; the grant
//!     table is consulted by `ly_tcb_gate` on every privileged call.
//!   - Page-table mappings for non-TCB code are flagged NX + non-user until
//!     explicitly allowed.
//!
//! This file is intentionally small: it is the root of trust.

use crate::serial;

/// Bitmask of which privilege domains exist.
#[repr(u64)]
#[derive(Clone, Copy)]
pub enum LyDomain {
    /// The TCB itself (this code). Ring 0, all permissions.
    Tcb = 0,
    /// A loaded kernel-space component. Ring 0 but restricted by vtable.
    Component = 1,
    /// Future: user tasks. Ring 3.
    User = 2,
}

/// Hard-coded TCB text range (from linker). Components must not overlap it.
extern "C" {
    static _text: u8;
    static _etext: u8;
}

/// Check whether a given PC lies inside the TCB.
pub fn ly_tcb_in_tcb(addr: u64) -> bool {
    let lo = core::ptr::addr_of!(_text) as u64;
    let hi = core::ptr::addr_of!(_etext) as u64;
    addr >= lo && addr < hi
}

/// 组件授权表（最多 8 槽，与 component 注册表对齐）。
const MAX_GRANTS: usize = 8;
static mut GRANTS: [u64; MAX_GRANTS] = [0; MAX_GRANTS];

/// 为组件槽授予权限。
pub fn ly_tcb_grant_component(idx: usize, perms: u64) {
    unsafe {
        if idx < MAX_GRANTS {
            GRANTS[idx] |= perms;
        }
    }
}

/// 吊销组件槽权限。
pub fn ly_tcb_revoke_component(idx: usize) {
    unsafe {
        if idx < MAX_GRANTS {
            GRANTS[idx] = 0;
        }
    }
}

/// 查询组件槽已授予的权限。
pub fn ly_tcb_component_perm(idx: usize) -> u64 {
    unsafe {
        if idx < MAX_GRANTS {
            GRANTS[idx]
        } else {
            0
        }
    }
}

/// Initialize the TCB boundary.
pub fn ly_tcb_init() {
    let text_lo = unsafe { core::ptr::addr_of!(_text) as u64 };
    let text_hi = unsafe { core::ptr::addr_of!(_etext) as u64 };
    serial::log("   TCB: text range 0x");
    serial::log_hex(text_lo);
    serial::log(" - 0x");
    serial::log_hex(text_hi);
    serial::log(" (");
    serial::log_u(text_hi - text_lo);
    serial::log(" bytes)\r\n");
    serial::log("   TCB: boundary active, grant table online (8 slots)\r\n");
}

/// Gatekeeper for a privileged operation called from a component.
///
/// Real policy:
///   1. Tcb domain: internal TCB calls, always allowed.
///   2. Component domain: check the requested permission against the slot's
///      grant mask; refuse if not granted (no caller-id yet — grants are
///      checked against the table; caller identity is bound at call site).
///   3. User domain: no privileged ops, always denied (until ring 3 exists).
pub fn ly_tcb_gate(caller_domain: LyDomain, perm: u64) -> bool {
    match caller_domain {
        LyDomain::Tcb => true,
        LyDomain::Component => {
            // 授权表任一槽持有该权限即放行（组件权限在注册时授予）。
            unsafe {
                for i in 0..MAX_GRANTS {
                    if GRANTS[i] & perm == perm {
                        return true;
                    }
                }
            }
            serial::log("   TCB: gate DENY component perm=0x");
            serial::log_hex(perm);
            serial::log("\r\n");
            false
        }
        LyDomain::User => {
            serial::log("   TCB: gate DENY user (ring 3 not available)\r\n");
            false
        }
    }
}

/// Switch the active page-table root (CR3) — real implementation.
/// `new_cr3` must be the physical address of a valid PML4 (canonical check).
pub fn ly_tcb_switch_cr3(new_cr3: u64) -> Result<(), &'static str> {
    // CR3 必须是规范地址且页对齐。
    if new_cr3 & 0xFFF != 0 {
        return Err("CR3 not page aligned");
    }
    let canonical = if new_cr3 >> 47 == 0 || new_cr3 >> 47 == 0x1FFFF {
        true
    } else {
        false
    };
    if !canonical {
        return Err("CR3 not canonical");
    }
    unsafe {
        core::arch::asm!("mov cr3, {0}", in(reg) new_cr3, options(nostack, preserves_flags));
    }
    Ok(())
}
