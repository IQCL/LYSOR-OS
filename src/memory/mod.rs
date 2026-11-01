//! Physical and virtual memory management for LYSOR OS.
//!
//! - `physical`: bitmap-based frame allocator, driven by multiboot2 memory map.
//! - `paging`:   PML4 mapping, page fault hook, map/unmap primitives,
//!               per-process address spaces (real CR3 isolation).
//! - `heap`:     kernel heap (first-fit free list, boundary tags) backing
//!               `alloc::Box` / `alloc::vec`.

pub mod physical;
pub mod paging;
pub mod heap;

pub use physical::{alloc_frame, free_frame, init_physical, frame_used_count};
pub use paging::init_virtual;

/// Page size: 4 KiB.
pub const PAGE_SIZE: usize = 4096;

/// Align an address up to the next 4K boundary.
pub const fn align_up(addr: usize) -> usize {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Align an address down to a 4K boundary.
pub const fn align_down(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}
