//! Minimal VGA text-mode stub (80x25, 0xB8000).
//! Currently unused - serial is the primary log channel. Kept as a placeholder
//! for future TCB debugging console.

#[allow(dead_code)]
pub fn vga_puts(_s: &str) {
    // placeholder: not implemented to keep TCB minimal
}
