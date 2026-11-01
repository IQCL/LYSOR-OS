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
            Err(_e) => serial::log("   map_page failed: "),
        }
    } else {
        serial::log("   no free frame for scratch map\r\n");
    }

    serial::log("   virtual memory ready (identity 1 GiB + higher-half scratch)\r\n");
}

// ---------------------------------------------------------------------------
// Per-process address spaces (real CR3 isolation)
// ---------------------------------------------------------------------------

/// User-space virtual base: PML4 slot 128 (0x0000_4000_0000_0000, canonical
/// low half). The kernel low region (identity) and higher-half mappings are
/// shared into every address space WITHOUT the USER flag, so ring 3 code
/// can never touch kernel memory while ring 0 keeps full visibility.
pub const USER_BASE: u64 = 0x0000_4000_0000_0000;
/// User stack region: 16 pages below this top.
pub const USER_STACK_TOP: u64 = 0x0000_4000_1000_0000;

/// Physical address of the boot (kernel) PML4 — it is identity-mapped in BSS.
pub fn kernel_pml4() -> u64 {
    core::ptr::addr_of!(boot_p4) as u64
}

/// Read CR3.
pub fn current_cr3() -> u64 {
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack)) };
    cr3
}

/// Switch CR3 (full TLB flush).
pub fn switch_cr3(pml4_phys: u64) {
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) pml4_phys, options(nomem, nostack)) };
}

/// Create a fresh address space: a new PML4 that shares the kernel low
/// region and higher-half mappings (entries 0 and 256..512 of the boot
/// PML4). All other slots are empty — user pages are mapped per-process.
pub fn create_address_space() -> Result<u64, &'static str> {
    let frame = physical::alloc_frame().ok_or("no frame for PML4")?;
    if frame >= 0x4000_0000 {
        physical::free_frame(frame);
        return Err("PML4 frame out of identity range");
    }
    unsafe {
        let tbl = frame as *mut PageTable;
        for i in 0..512 { core::ptr::write_volatile(&mut (*tbl).entries[i], 0); }
        let boot = p4();
        // Kernel low region (entry 0, no USER flag) + higher half (256..512).
        (*tbl).entries[0] = boot.entries[0];
        for i in 256..512 {
            (*tbl).entries[i] = boot.entries[i];
        }
    }
    Ok(frame)
}

/// Map a page into a *specific* address space (by PML4 physical address).
/// Page tables below PML4 are private to that address space and allocated
/// below 1 GiB (identity-mapped, so direct phys deref is valid).
pub fn map_page_as(pml4_phys: u64, phys: u64, virt: u64, flags: u64) -> Result<(), &'static str> {
    if pml4_phys >= 0x4000_0000 { return Err("PML4 out of identity range"); }
    unsafe {
        // Walk/create explicitly so every level is created inside the right
        // address space (identity-mapped frames, phys == direct deref).
        let root = pml4_phys as *mut PageTable;
        let i4 = ((virt >> 39) & 0x1FF) as usize;
        let i3 = ((virt >> 30) & 0x1FF) as usize;
        let i2 = ((virt >> 21) & 0x1FF) as usize;
        let i1 = ((virt >> 12) & 0x1FF) as usize;

        let p3 = ensure_table(&mut (*root).entries[i4])?;
        let p2 = ensure_table(&mut (*p3).entries[i3])?;
        let p1 = ensure_table(&mut (*p2).entries[i2])?;
        (*p1).entries[i1] = (phys & 0x000F_FFFF_FFFF_F000) | flags | PTE_PRESENT;
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
    }
    Ok(())
}

/// Get-or-create a lower-level table for `entry` and return its pointer.
/// New tables are allocated below 1 GiB (identity-mapped, direct deref ok).
unsafe fn ensure_table(entry: &mut u64) -> Result<*mut PageTable, &'static str> {
    if *entry & PTE_PRESENT != 0 {
        let phys = *entry & 0x000F_FFFF_FFFF_F000;
        if phys >= 0x4000_0000 { return Err("page table out of identity range"); }
        return Ok(phys as *mut PageTable);
    }
    let frame = physical::alloc_frame().ok_or("no page-table frame")?;
    if frame >= 0x4000_0000 {
        physical::free_frame(frame);
        return Err("no page-table frame");
    }
    let tbl = frame as *mut PageTable;
    for e in (*tbl).entries.iter_mut() { *e = 0; }
    *entry = frame | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
    Ok(tbl)
}

/// Address-space registry for teardown: tracks every frame handed to a
/// process (user pages + private page tables) so exit can reclaim them.
const MAX_ADDR_SPACES: usize = 16;
const AS_FRAMES_CAP: usize = 2048;

#[derive(Clone, Copy)]
struct AsRecord {
    used: bool,
    pml4: u64,
    frames: [u64; AS_FRAMES_CAP],
    nframes: usize,
}

static mut AS_TABLE: [AsRecord; MAX_ADDR_SPACES] = [AsRecord {
    used: false, pml4: 0, frames: [0; AS_FRAMES_CAP], nframes: 0,
}; MAX_ADDR_SPACES];

/// Register a newly created address space and start tracking its frames.
pub fn register_address_space(pml4: u64) -> Result<usize, &'static str> {
    unsafe {
        for r in AS_TABLE.iter_mut() {
            if !r.used {
                r.used = true;
                r.pml4 = pml4;
                r.nframes = 0;
                return Ok(r as *mut AsRecord as usize - AS_TABLE.as_mut_ptr() as usize)
                    .map(|off| off / core::mem::size_of::<AsRecord>());
            }
        }
    }
    Err("address space table full")
}

/// Track a frame belonging to an address space (user page or private table).
pub fn track_frame(as_slot: usize, frame: u64) {
    unsafe {
        if as_slot >= MAX_ADDR_SPACES { return; }
        let r = &mut AS_TABLE[as_slot];
        if r.nframes < AS_FRAMES_CAP {
            r.frames[r.nframes] = frame;
            r.nframes += 1;
        }
    }
}

/// Tear down an address space: free every tracked frame + the PML4.
/// Returns the number of frames reclaimed.
pub fn release_address_space(as_slot: usize) -> usize {
    unsafe {
        if as_slot >= MAX_ADDR_SPACES { return 0; }
        let r = &mut AS_TABLE[as_slot];
        if !r.used { return 0; }
        let mut freed = 0usize;
        for i in 0..r.nframes {
            physical::free_frame(r.frames[i]);
            freed += 1;
        }
        // Free any private page-table frames still mapped under this PML4
        // that were not tracked explicitly (defensive: track_frame should
        // have covered them via map_page_as callers).
        free_subtables(r.pml4, 0, &mut freed);
        physical::free_frame(r.pml4);
        freed += 1;
        r.used = false;
        r.pml4 = 0;
        r.nframes = 0;
        freed
    }
}

/// Recursively free private page-table frames (levels 3..1) of a PML4.
/// Skips entries shared with the boot PML4 (kernel low + higher half).
unsafe fn free_subtables(pml4: u64, level: usize, freed: &mut usize) {
    if pml4 == 0 || pml4 >= 0x4000_0000 { return; }
    let tbl = pml4 as *mut PageTable;
    let start = if level == 0 { 1 } else { 0 }; // slot 0 of the root is shared
    let end = if level == 0 { 256 } else { 512 };
    for i in start..end {
        let e = (*tbl).entries[i];
        if e & PTE_PRESENT == 0 || e & PTE_HUGE != 0 { continue; }
        let phys = e & 0x000F_FFFF_FFFF_F000;
        if level < 3 {
            free_subtables(phys, level + 1, freed);
        }
        physical::free_frame(phys);
        *freed += 1;
    }
}

/// Address-space self-test: create a space, map a user page into it, verify
/// the mapping reads back, then switch CR3 there and back on the boot AS.
pub fn address_space_selftest() -> bool {
    let boot = kernel_pml4();
    let pml4 = match create_address_space() {
        Ok(p) => p,
        Err(e) => {
            serial::log("   as: create FAILED: ");
            serial::log(e);
            serial::log("\r\n");
            return false;
        }
    };
    let slot = match register_address_space(pml4) {
        Ok(s) => s,
        Err(_) => { physical::free_frame(pml4); return false; }
    };
    let upage = match physical::alloc_frame() {
        Some(f) if f < 0x4000_0000 => f,
        Some(f) => { physical::free_frame(f); release_address_space(slot); return false; }
        None => { release_address_space(slot); return false; }
    };
    track_frame(slot, upage);

    // Fill the frame through the identity mapping, then read through the
    // user mapping after switching CR3.
    unsafe {
        let ident = upage as *mut u64;
        core::ptr::write_volatile(ident, 0x1DEA_2026);
        let va = USER_BASE + 0x8000;
        if map_page_as(pml4, upage, va, PTE_USER | PTE_WRITABLE).is_err() {
            physical::free_frame(upage);
            release_address_space(slot);
            return false;
        }
        switch_cr3(pml4);
        let uptr = va as *const u64;
        let ok_read = core::ptr::read_volatile(uptr) == 0x1DEA_2026;
        // Kernel must still be visible in the new address space (identity).
        let kptr = 0x1000u64 as *const u64; // low kernel region (present in every AS)
        let _kernel_visible = core::ptr::read_volatile(kptr) != u64::MAX;
        switch_cr3(boot);
        let freed = release_address_space(slot);
        serial::log("   as: self-test ");
        serial::log(if ok_read { "passed" } else { "FAILED (user read mismatch)" });
        serial::log(" (CR3 0x");
        serial::log_hex(pml4);
        serial::log(", ");
        serial::log_u(freed as u64);
        serial::log(" frames reclaimed)\r\n");
        ok_read
    }
}
