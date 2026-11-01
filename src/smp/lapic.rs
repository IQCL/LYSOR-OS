//! 本地 APIC（Local APIC）。
//!
//! MMIO 基址：0xFEE0_0000（默认，QEMU 固定写死）。
//!
//! 寄存器偏移：
//!   0x00  APIC_ID
//!   0x01  APIC_VERSION
//!   0x02  TPR（Task Priority Register，用于中断屏蔽）
//!   0x05  EOI（写 0 清除最高优先级的 ISR 位）
//!   0x06  LDR（Logical Destination Register）
//!   0x07  DFR（Destination Format Register）
//!   0x08  SVR（Spurious Vector Register，bit 8 = Enable）
//!   0x09-0x0F  ISR（In-Service Register，8 个 32-bit 字）
//!   0x10  TMR（Trigger Mode Register，8 个 32-bit 字）
//!   0x14  IIR（Interrupt In-Service Register，8 个 32-bit 字）
//!   0x30  ICR_LO（Interrupt Command Register，低 32-bit）
//!   0x31  ICR_HI（Interrupt Command Register，高 32-bit）
//!   0x32  LVT Timer
//!   0x34  LVT Local Int 1
//!   0x35  LVT Error
//!   0x36  LVT Local Int 2 (POWER)
//!   0x38  Timer Initial Count
//!   0x39  Timer Current Count
//!   0x3E  Timer Divide Configuration
//!
//! 设计：PIT 保留给单核兼容；BSP 启动 AP 用 INIT-SIPI-SIPI；
//! AP 自己关掉 PIT（或用同一个 PIT 也可以，只是 tick 重复）。
//! 简化起见：**暂时不启 LAPIC Timer**，保留 PIT，AP 上也能收 PIT tick。

/// LAPIC MMIO 基址（higher-half 映射，由 boot 期 map_page 建立；
/// 低区 identity 0xFEE00000 在 QEMU TCG 下 walk 异常）。
pub const LAPIC_BASE: u64 = 0xFFFF_8000_FEE0_0000;

/// 已保留在 IDT 里的向量号。PIT 用 vector 32，LAPIC 保留：
pub const LAPIC_SPURIOUS: u8 = 0xFF;
pub const LAPIC_ERROR: u8 = 0xFE;
pub const LAPIC_TIMER: u8 = 0xFD;

// 寄存器偏移（*4，因为 APIC 寄存器在 [0xFEE00000..] 是 dword 间距 0x10）
// 标准 APIC：每个寄存器占 0x10 字节间隔（256 字节一个寄存器块），
// 地址 = LAPIC_BASE + (offset * 0x10)，offset 单位 dword index。
const REG_ID: u32 = 0x02;          // offset 0x00 / 0x10
const REG_VERSION: u32 = 0x03;
#[allow(dead_code)] // LAPIC 寄存器索引，中断/优先级编程预留
const REG_TPR: u32 = 0x08;
const REG_EOI: u32 = 0x0B;
#[allow(dead_code)]
const REG_LDR: u32 = 0x0D;
#[allow(dead_code)]
const REG_DFR: u32 = 0x0E;
const REG_SVR: u32 = 0x0F;
#[allow(dead_code)]
const REG_ISR_BASE: u32 = 0x10;
#[allow(dead_code)]
const REG_TMR_BASE: u32 = 0x18;
#[allow(dead_code)]
const REG_IIR_BASE: u32 = 0x20;
const REG_ESR: u32 = 0x28;        // Error Status Register（读了 ESR 才能清 ESR）
#[allow(dead_code)]
const REG_CMCI: u32 = 0x2F;
const REG_ICR_LO: u32 = 0x30;
const REG_ICR_HI: u32 = 0x31;
const REG_LVT_TIMER: u32 = 0x32;
// 标准 LAPIC 寄存器索引（地址 = LAPIC_BASE + index*0x10）：
//   Timer=0x320, Thermal=0x330, PerfMon=0x340, LINT0=0x350,
//   LINT1=0x360, Error=0x370。
#[allow(dead_code)] // LVT 寄存器索引，电源管理/外部中断路由预留
const REG_LVT_LINT0: u32 = 0x35;
#[allow(dead_code)]
const REG_LVT_LINT1: u32 = 0x36;
const REG_LVT_ERROR: u32 = 0x37;
#[allow(dead_code)]
const REG_LVT_PM: u32 = 0x34;
const REG_TIC: u32 = 0x38;        // Timer Initial Count
const REG_TCC: u32 = 0x39;        // Timer Current Count
const REG_TDCR: u32 = 0x3E;       // Timer Divide

#[inline(always)]
unsafe fn mmio_read(base: u64, reg: u32) -> u32 {
    // 注意优先级：base + (reg << 4)，不是 (base + reg) << 4。
    core::ptr::read_volatile((base + ((reg as u64) << 4)) as *const u32)
}

#[inline(always)]
unsafe fn mmio_write(base: u64, reg: u32, val: u32) {
    core::ptr::write_volatile((base + ((reg as u64) << 4)) as *mut u32, val);
}

/// 返回当前 CPU 的 APIC ID（0-255）。BSP 一定是 0。
pub fn apic_id() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_ID) >> 24 }
}

/// 读 APIC version。
pub fn apic_version() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_VERSION) }
}

/// 启用 LAPIC（写 SVR：vector = spurious + 0x100 使能位）。
/// Error/Spurious 向量 LVT 也一并挂上，AP 上一定要调。
pub fn lapic_enable() {
    unsafe {
        // 让 ESR 有内容：先写一次 0 清状态
        mmio_write(LAPIC_BASE, REG_ESR, 0);
        // Error LVT（vector 0xFE，masked bit=0 so 不 mask）
        mmio_write(LAPIC_BASE, REG_LVT_ERROR, LAPIC_ERROR as u32);
        // Spurious vector + enable bit (bit 8)
        mmio_write(LAPIC_BASE, REG_SVR, (LAPIC_SPURIOUS as u32) | (1 << 8));
    }
}

/// 写 EOI（End Of Interrupt）。**必须**在 ISR 末尾调用，否则 APIC 不再递交
/// 同级及以下中断。
#[inline(always)]
pub fn eoi() {
    unsafe { mmio_write(LAPIC_BASE, REG_EOI, 0) };
}

/// 读 LAPIC Error Status Register（vec 0xFE handler 用来 dump 错误码）。
pub fn read_esr() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_ESR) }
}

// ---- IPIs ----

/// 目的字段（ICR_HI 写 8-bit APIC ID）。广播则设成 `0xFF << 24`（所有）。
/// 我们只用 directed IPI 给 AP 发 INIT / SIPI。
pub const ICR_LEVEL_ASSERT: u32 = 1 << 14;   // INIT: 1
pub const ICR_TRIGGER_EDGE: u32 = 0;
pub const ICR_TRIGGER_LEVEL: u32 = 1 << 15;
pub const ICR_DESTMOD_PHYS: u32 = 0;
pub const ICR_DEST_SHORTHAND: u32 = 0 << 18; // directed

/// ICR Delivery Mode（bits 10:8）。
/// 之前漏了这两个位：INIT 被编成 fixed edge vector 0（AP 忽略），SIPI 同理。
pub const ICR_DELIVERY_INIT: u32 = 5 << 8;   // 0101 = INIT
pub const ICR_DELIVERY_SIPI: u32 = 6 << 8;   // 0110 = Start-Up

/// INIT IPI：让目标 CPU 进入 INIT state（等价于复位，但保留 CPUID 状态）。
/// 编码：delivery=INIT(101), trigger=level, level=assert, vector=0。
pub fn send_init(target_apic_id: u32) {
    let hi = (target_apic_id & 0xFF) << 24;
    let lo = ICR_DELIVERY_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_ASSERT | ICR_DESTMOD_PHYS | 0x000;
    unsafe {
        mmio_write(LAPIC_BASE, REG_ICR_HI, hi);
        mmio_write(LAPIC_BASE, REG_ICR_LO, lo);
    }
    // 等 ICR 发送完成（Delivery Status bit 12 必须清）
    wait_icr_idle();
}

/// SIPI：让目标 CPU 从 `vector * 4KiB` 开始执行。QEMU 要求发两次。
/// 编码：delivery=Start-Up(110), trigger=edge, vector=启动向量。
pub fn send_sipi(target_apic_id: u32, vector: u8) {
    let hi = (target_apic_id & 0xFF) << 24;
    let lo = ICR_DELIVERY_SIPI | ICR_TRIGGER_EDGE | ICR_DESTMOD_PHYS | (vector as u32 & 0xFF);
    unsafe {
        mmio_write(LAPIC_BASE, REG_ICR_HI, hi);
        mmio_write(LAPIC_BASE, REG_ICR_LO, lo);
    }
    wait_icr_idle();
}

/// Fixed 模式定向 IPI：给目标 CPU 送一个 `vector` 中断（AP 调度驱动用）。
/// 编码：delivery=fixed(000), trigger=edge, vector=vector。
pub fn send_ipi_fixed(target_apic_id: u32, vector: u8) {
    let hi = (target_apic_id & 0xFF) << 24;
    let lo = ICR_TRIGGER_EDGE | ICR_DESTMOD_PHYS | (vector as u32 & 0xFF);
    unsafe {
        mmio_write(LAPIC_BASE, REG_ICR_HI, hi);
        mmio_write(LAPIC_BASE, REG_ICR_LO, lo);
    }
    wait_icr_idle();
}

fn wait_icr_idle() {
    unsafe {
        // 第一遍：纯读（QEMU TCG 下 ICR 发送几乎瞬时完成，busy 位立即清 0）。
        for _ in 0..100u32 {
            let v = mmio_read(LAPIC_BASE, REG_ICR_LO);
            if (v & (1 << 12)) == 0 {
                return;
            }
            core::hint::spin_loop();
        }
        // 第二遍：带 IO delay 再等，上限 2000 次（QEMU 下足以覆盖真实硬件时序）。
        for _ in 0..2000u32 {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x80u16,
                in("al") 0u8,
                options(nostack),
            );
            let v = mmio_read(LAPIC_BASE, REG_ICR_LO);
            if (v & (1 << 12)) == 0 {
                return;
            }
        }
    }
    crate::serial::log("   lapic: WARN ICR delivery-status timeout\r\n");
}

// ---- Timer ----

/// 停掉 LAPIC Timer（LVT Timer 的 mask bit 置 1）。我们保留 PIT 作为单核 tick，
/// 所以 AP 上 timer 用 LAPIC（或者干脆全部 AP 也收 PIT——但 PIT 只连到 CPU0）。
/// 为了简化，**LAPIC Timer 暂不启用**，AP 上只靠 INIT→SIPI 拉起来后停 halt 等
/// 调度（由 BSP 的 PIT tick 驱动 pick_next 在哪个 CPU 都行——per-CPU current_tid
/// 切换时顺便切 CR3 和 TSS.rsp0）。但这样 AP 永远不会被调度是个问题。
///
/// 最干净的方案：**每个 CPU 都收 PIT**——PIT 是 8254 芯片，只连到 LAPIC 的
/// CPU0 管脚。让所有 CPU 都能收：把 PIT 的 IRQ0 通过 IPI 转发给所有 CPU，
/// 或者直接用 LAPIC Timer 代替 PIT。
///
/// 我们选：**保留 PIT 只给 BSP**，AP 上用 LAPIC Timer 自己产生 tick，
/// 频率和 PIT 对齐（100 Hz）。这样每个 CPU 独立被抢占。

/// 配置 LAPIC Timer：periodic，vector = LAPIC_TIMER，每次 tick 间隔 ~10ms = 100Hz。
/// 初始计数值：TDCR 决定分频；默认 `TDCR = 0 → 1`，那么 Initial Count 直接
/// 就是多少个 CPU 周期。但 CPU 周期得校准——我们直接用 `TDCR=2`（除 2）
/// 然后 Initial Count = 一个“足够大”的数让它 ~10ms，实际上 QEMU 下 LAPIC
/// Timer 精度不重要，我们只需要它能把 AP 拉回调度器。
pub fn lapic_timer_start_periodic() {
    unsafe {
        // Divide by 2（寄存器值 0x01）
        mmio_write(LAPIC_BASE, REG_TDCR, 0x01);
        // Initial Count = 0xFFFFFFFF（很快倒计时完，让它快 tick；实际 QEMU 下约 1ms 一次）
        // 我们让它 ~2ms，太频繁 serial 刷屏严重。
        // 简化：用一次性模式 tick 一次后停，改为 periodic。
        // LVT Timer：vector=LAPIC_TIMER。QEMU 的 APIC 实现里
        // `APIC_LVT_TIMER_PERIODIC = (1 << 17)`——与 Intel SDM 的
        // bits[17:16]=0b01（periodic）位定义不同。实测 bit16（SDM 语义）
        // 在 QEMU 下不触发（AP ticks=0），bit17（QEMU 语义）才周期触发。
        mmio_write(
            LAPIC_BASE,
            REG_LVT_TIMER,
            LAPIC_TIMER as u32 | (1u32 << 17),
        );
        // Initial Count：0x800000（QEMU 下约 8ms/tick，125Hz）。
        // 原来 0x100000（~1ms）在 TCG 多核下 AP 中断风暴会拖垮 BSP 的
        // boot 测试（实测密码学测试 30s 未完成）。125Hz 对验收足够
        // （5s ≈ 625 tick，每个 tick 间隔 worker 至少 ++2 次即 >1000）。
        mmio_write(LAPIC_BASE, REG_TIC, 0x0080_0000);
    }
}

pub fn lapic_timer_stop() {
    unsafe {
        mmio_write(LAPIC_BASE, REG_LVT_TIMER, 1u32 << 16); // mask bit = 1
        mmio_write(LAPIC_BASE, REG_TIC, 0);
    }
}

/// Mask LAPIC Timer（不使能，也不配置向量）：AP 的调度由 BSP 的
/// 固定 IPI 驱动，LAPIC timer 保持静默，避免 boot 期随机触发与
/// BSP 的 spawn 抢 TCB_LOCK（QEMU TCG 下 hlt 冻结、非 hlt 窗口偶发
/// 触发，实测引发布局/时序敏感的随机卡死）。
pub fn lapic_timer_mask() {
    unsafe {
        // mask 位 = bit16 置 1；保留 vector 域以便读回诊断可见。
        mmio_write(LAPIC_BASE, REG_LVT_TIMER, 1u32 << 16 | LAPIC_TIMER as u32);
        mmio_write(LAPIC_BASE, REG_TIC, 0);
    }
}

/// 读 LVT Timer 寄存器（诊断用）。
pub fn read_lvtt_timer() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_LVT_TIMER) }
}

/// 读 Timer Initial Count（诊断用）。
pub fn read_tic() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_TIC) }
}

/// 读 Timer Current Count（诊断用）。
pub fn read_tcc() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, REG_TCC) }
}
