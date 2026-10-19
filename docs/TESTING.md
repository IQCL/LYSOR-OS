# LYSOR OS - Minimal TCB Kernel Base - Test Document

## 1. Build prerequisites

- Rust stable toolchain (`rustup`) with `x86_64-unknown-none` target support.
- QEMU (`qemu-system-x86_64`).
- The `rust-lld` linker (bundled with Rust).

```powershell
# Add Rust to PATH (one-time per session)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

## 2. Build

```powershell
cargo build --release
```

Output ELF kernel:
```
target\x86_64-unknown-none\release\lysor-os
```

## 3. Run

```powershell
qemu-system-x86_64.exe -kernel target\x86_64-unknown-none\release\lysor-os -m 256M -nographic -serial stdio -no-reboot
```

Press `Ctrl+A` then `X` to exit QEMU.

## 4. Boot method

The kernel uses the **Xen PVH boot protocol** (ELF note: name="Xen", type=18 / XEN_ELFNOTE_PHYS32_ENTRY).
QEMU loads the 64-bit ELF and jumps to the 32-bit entry point (`_start` at 0x100000).
The 32-bit stub sets up temporary 2 MiB huge-page tables, enables PAE + long mode, then jumps to the 64-bit `kernel_main`.

## 5. Expected serial output

```
===============================================
  LYSOR OS - minimal TCB kernel base v0.1.0
  build: 0.1.0
===============================================
>> installing GDT...
   GDT: loaded, CS=0x08 DS=0x10
>> installing IDT...
   IDT: 256 gates installed
>> remapping PIC8259...
>> initializing physical memory manager...
   PVH boot: using default memory map (256 MiB)
>> initializing virtual memory...
>> TCB boundary established
>> LYSOR OS boot complete.
>> spawning test threads...
   [A] thread started
   [B] thread started
   [C] thread started
```

The kernel then runs worker threads and an idle loop.

## 6. Test matrix

| # | Test | Expected | Pass criteria | Status |
|---|------|----------|---------------|--------|
| T1 | QEMU boots kernel | No triple-fault, serial output appears | "LYSOR OS" banner on serial | PASS |
| T2 | PVH note recognized | QEMU loads 64-bit ELF | Entry reached, no boot failure | PASS |
| T3 | Long-mode transition | 64-bit code executes | GDT/IDT logs printed | PASS |
| T4 | Serial output | Characters on stdio | Banner + boot logs visible | PASS |
| T5 | GDT load | CS/DS reload OK | "GDT: loaded" log | PASS |
| T6 | IDT install | 256 gates | "IDT: 256 gates installed" | PASS |
| T7 | PIC remap | No spurious interrupts | No flood of IRQ messages | PASS |
| T8 | Physical allocator | Frames allocated | frame alloc logs | PASS |
| T9 | Virtual memory | Scratch page mapped | virtual memory ready | PASS |
| T10 | TCB boundary | Text range recorded | "TCB boundary established" | PASS |
| T11 | IPC self-test | Channel A/B work | ipc test A/B OK | PASS |
| T12 | Syscall self-test | INT 0x80 roundtrip | syscall OK | PASS |
| T13 | Disk persistence | bootcount increments | DISK PERSISTENCE VERIFIED | PASS |
| T14 | Idle loop stability | No crash | QEMU process still running | PASS |

## 7. Scheduler (`ly_sched`)

### Architecture

- **Context switch**: TrapFrame swap on timer ISR return.
- **Algorithm**: Round-robin, 10ms quantum (PIT channel 0 at 100Hz).
- **Threads**: Up to 16 kernel threads, 16 KiB stack each.
- **Lifecycle**: Ready → Running → Zombie → Reaped (stack freed).

## 8. Project structure

```
├── Cargo.toml
├── .cargo/config.toml
├── linker.ld
├── docs/
└── src/
    ├── main.rs             # Entry + boot asm + test worker threads
    ├── serial.rs           # COM1 UART driver
    ├── gdt.rs              # 64-bit GDT
    ├── idt.rs              # 256-entry IDT
    ├── pic.rs              # 8259 PIC remap
    ├── memory/
    │   ├── mod.rs
    │   ├── physical.rs     # Bitmap frame allocator
    │   └── paging.rs       # PML4 page table walker
    ├── sched/
    │   ├── mod.rs          # ly_sched: round-robin, lifecycle
    │   ├── pit.rs          # 8254 PIT timer (100Hz)
    │   └── capability.rs   # Capability model
    ├── ipc/                # Channel A (msg) + Channel B (shm whitelist)
    ├── syscall/            # INT 0x80 syscall dispatcher
    ├── vfs/                # Virtual filesystem
    ├── fs/                 # LFS filesystem
    ├── drivers/            # ATA/PCI/PS2/RTC drivers
    ├── tcb/                # TCB boundary
    ├── exec/               # LYX executable loader
    └── sig/                # Signature verification contract
```
