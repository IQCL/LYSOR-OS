//! Virtual memory / paging.
//!
//! We keep the boot-time PML4 (identity-maps first 1 GiB via 2 MiB huge pages).
//! Page tables live in BSS and are identity-mapped, so we can dereference them
//! directly by their virtual==physical address.
//!
//! Primitives:
//!   - `map_page(phys, virt, flags)`: create a 4 KiB mapping
//!   - `unmap_page(virt)`: remove a mapping
//!   - `walk(addr)`: return the PTE for a virtual address
//!
//! Security note: page-table flags enforce the TCB boundary: user-accessible
//! pages are flagged `USER`; kernel pages are not. Future component loader
//! will use these flags to carve out per-component address spaces.

use crate::memory::physical;
use crate::serial;

// Page table entry flags (x86_64)
pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE: u64 = 1 << 7;
pub const PTE_GLOBAL: u64 = 1 << 8;
pub const PTE_NX: u64 = 1 << 63;

// The boot-time PML4 (phys==virt, in BSS).
// Defined in main.rs boot assembly.
extern "C" {
    static mut boot_p4: u64;
}

#[repr(align(4096))]
struct PageTable {
    entries: [u64; 512],
}

fn p4() -> &'static mut PageTable {
    unsafe { &mut *(core::ptr::addr_of_mut!(boot_p4) as *mut PageTable) }
}

/// Get (creating if needed) the next-level table for a given entry.
fn next_level(entry: &mut u64, create: bool) -> Option<&'static mut PageTable> {
    if *entry & PTE_PRESENT != 0 {
        let phys = *entry & 0x000F_FFFF_FFFF_F000;
        return Some(unsafe { &mut *(phys as *mut PageTable) });
    }
    if !create {
        return None;
    }
    let frame = physical::alloc_frame()?;
    // Only frames < 1 GiB are identity-mapped; reject higher allocs.
    if frame >= 0x4000_0000 {
        physical::free_frame(frame);
        return None;
    }
    let tbl = unsafe { &mut *(frame as *mut PageTable) };
    for e in tbl.entries.iter_mut() {
        *e = 0;
    }
    *entry = frame | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
    Some(tbl)
}

/// Map a single 4 KiB page.
pub fn map_page(phys: u64, virt: u64, flags: u64) -> Result<(), &'static str> {
    let p4 = p4();
    let i4 = ((virt >> 39) & 0x1FF) as usize;
    let p3 = next_level(&mut p4.entries[i4], true).ok_or("no P3 frame")?;

    let i3 = ((virt >> 30) & 0x1FF) as usize;
    let p2 = next_level(&mut p3.entries[i3], true).ok_or("no P2 frame")?;

    let i2 = ((virt >> 21) & 0x1FF) as usize;
    let p1 = next_level(&mut p2.entries[i2], true).ok_or("no P1 frame")?;

    let i1 = ((virt >> 12) & 0x1FF) as usize;
    p1.entries[i1] = (phys & 0x000F_FFFF_FFFF_F000) | flags | PTE_PRESENT;

    // Invalidate TLB
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
    }
    Ok(())
}

/// Unmap a single page.
#[allow(dead_code)]
pub fn unmap_page(virt: u64) -> Result<(), &'static str> {
    let p4 = p4();
    let i4 = ((virt >> 39) & 0x1FF) as usize;
    let p3 = next_level(&mut p4.entries[i4], false).ok_or("not mapped")?;

    let i3 = ((virt >> 30) & 0x1FF) as usize;
    let p2 = next_level(&mut p3.entries[i3], false).ok_or("not mapped")?;

    let i2 = ((virt >> 21) & 0x1FF) as usize;
    let p1 = next_level(&mut p2.entries[i2], false).ok_or("not mapped")?;

    let i1 = ((virt >> 12) & 0x1FF) as usize;
    p1.entries[i1] = 0;
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
    }
    Ok(())
}

/// Initialize virtual memory subsystem.
pub fn init_virtual() {
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack)) };
    serial::log("   CR3 (PML4 phys): 0x");
    serial::log_hex(cr3);
    serial::log("\r\n");

    // Prove map_page works: allocate a scratch frame and map it at a scratch
    // virtual address (0xFFFF_8000_0000_1000 - unused higher-half).
    if let Some(frame) = physical::alloc_frame() {
        let vaddr = 0xFFFF_8000_0000_1000u64;
        match map_page(frame, vaddr, PTE_WRITABLE | PTE_USER) {
            Ok(_) => {
                serial::log("   mapped scratch frame 0x");
                serial::log_hex(frame);
                serial::log(" -> 0x");
                serial::log_hex(vaddr);
                serial::log("\r\n");
            }
            Err(e) => serial::log("   map_page failed: "),
        }
    } else {
        serial::log("   no free frame for scratch map\r\n");
    }

    serial::log("   virtual memory ready (identity 1 GiB + higher-half scratch)\r\n");
}
