# DRIVER_SPEC — LYSOR OS .lykc 开放端子驱动规范（蓝皮书）

文档版本：ds-v1.0.0
对应系统版本：LYSOR OS v0.2.0
权威来源：`src/kcomponent/lykc.rs`、`src/kcomponent/driver.rs`（私有仓真实现，本文件为公开契约）

> **边界声明**：本文是 `.lykc` 第三方驱动的唯一权威接口契约。内核实现细节（寄存器操作、故障域汇编等）不在本文范围；一切以本文 + 公开仓基座源码为准。

---

## 1. 总则

LYSOR OS v0.2 开放"驱动端子"（Open Driver Endpoint）：第三方硬件驱动以 `.lykc` 容器发布，由内核装载器强制验签后，在 KC 故障域内运行。**驱动不引用任何内核外部符号**——一切内核能力经入口参数传入的 `LyKernelExports` 导出表获得。

**无签名 / 验签失败 / ABI 不符 → 装载即报错，无任何旁路、无对话框。** `.lykc` 路径不存在 `allow_unsigned` 开关（区别于 `.lyx` 应用路径的开发者门）。

---

## 2. .lykc 容器格式

容器与 `.lyx` 尾部 trailer 完全同构（签名服务器脚本零改动即可签）。

### 2.1 头（64 字节，小端）

| 偏移 | 大小 | 字段 | 说明 |
|------|------|------|------|
| 0 | 4 | magic | `0x4C594B31`（"LYK1"） |
| 4 | 4 | version | 当前 `1`，大于内核 `LYKC_VERSION` 拒载 |
| 8 | 4 | text_off | 代码段在镜像内偏移 |
| 12 | 4 | text_size | 代码段字节数 |
| 16 | 4 | rodata_off | 只读数据段偏移 |
| 20 | 4 | rodata_size | 只读数据段字节数 |
| 24 | 4 | data_off | 数据段（含 BSS 清零）偏移 |
| 28 | 4 | data_size | 数据段字节数 |
| 32 | 8 | entry | 入口在镜像内偏移（`base + entry` 为调用地址） |
| 40 | 4 | driver_class | `DRV_CLASS_*`（见 §4） |
| 44 | 4 | driver_flags | `DRV_FLAG_*`，`.lykc` 恒含 `DRV_FLAG_LOADABLE(1<<1)` |
| 48 | 16 | reserved | 置零 |

段顺序固定：`[0..64) 头` → `text` → `rodata` → `data`（`data` 尾部追加清零 BSS）。装载器要求 `entry < max(段尾)`、各段 `off >= 64 && off+size <= payload_len`。

### 2.2 trailer（65 字节）

- `algo`（1B）：`0=SM2`、`1=Ed25519`（`0xFF=无签名`，但 `.lykc` 无签名即拒载）；
- `sig`（64B）：raw 签名（SM2：`r‖s` 各 32B 大端；Ed25519：raw 64B）；
- **被签范围 = 整个文件去尾 65 字节**，与 `.lyx` 完全同构。

---

## 3. LyKernelExports（唯一内核调用面）

`repr(C)`，入口参数 `entry(exports: *const LyKernelExports) -> *mut LyDriverDesc`。字段顺序固定，增改必须同步升 ABI 版本：

```rust
#[repr(C)]
pub struct LyKernelExports {
    pub abi_major: u16,   // 当前 1
    pub abi_minor: u16,   // 当前 0
    pub alloc_frame: extern "C" fn() -> u64,                 // 物理帧 | 0
    pub free_frame: extern "C" fn(phys: u64),
    pub map_page: extern "C" fn(phys: u64, virt: u64, flags: u64) -> i32,
    pub unmap_page: extern "C" fn(virt: u64) -> i32,
    pub mmio_map: extern "C" fn(phys: u64, size: u64) -> u64, // 内核 VA | 0
    pub irq_register: extern "C" fn(vector: u32) -> i32,
    pub pci_cfg_read: extern "C" fn(bus: u32, dev: u32, func: u32, reg: u32) -> u32,
    pub pci_cfg_find: extern "C" fn(vendor: u32, device: u32) -> u64, // bus<<32|dev<<8|func；!0 无
    pub klog: extern "C" fn(ptr: *const u8, len: u32),
    pub udelay: extern "C" fn(us: u32),
    pub dma_alloc: extern "C" fn(pages: u32) -> u64,         // 连续物理帧（identity 区）| 0
    pub dma_free: extern "C" fn(phys: u64, pages: u32),
}
```

---

## 4. 驱动描述符与 ABI

```rust
#[repr(C)]
pub struct LyDriverOps {
    pub probe: extern "C" fn() -> u64,          // 非 0 = 接管成功（驱动上下文）
    pub start: extern "C" fn(ctx: u64),
    pub stop: extern "C" fn(ctx: u64),
    pub ioctl: extern "C" fn(ctx: u64, cmd: u64, a0: u64, a1: u64) -> i64,
}

#[repr(C)]
pub struct LyDriverDesc {
    pub name: [u8; 24],
    pub bus_id: u32,       // PCI bus<<16|dev<<8|func；非 PCI = 0xFFFFFFFF
    pub class: u32,        // DRV_CLASS_*
    pub flags: u32,        // DRV_FLAG_*
    pub abi_major: u16,    // 必须 == LY_DRV_ABI_MAJOR(1)，否则拒载
    pub abi_minor: u16,
    pub ops: LyDriverOps,
}
```

权限 class 表（装载时按类授予 KC 权限）：

| class | 常量 | 用途 |
|-------|------|------|
| 0 | `DRV_CLASS_GPU` | 显示后端（帧缓冲/GTT） |
| 1 | `DRV_CLASS_INPUT` | 输入设备 |
| 2 | `DRV_CLASS_NET` | 网卡 |
| 3 | `DRV_CLASS_BLOCK` | 块设备 |
| 4 | `DRV_CLASS_USB` | USB 控制器/HID |
| 5 | `DRV_CLASS_MISC` | 其他 |

所有 `.lykc` 驱动装载时授予 `KCOMP_PERM_PHYS | KCOMP_PERM_VMAP | KCOMP_PERM_IRQ`；`class > 5` 拒载。

---

## 5. 装载与验签流程

```
ly_kc_load(image, len)
 ├─ 尺寸/头校验（magic、version、段范围、entry、class）
 ├─ 强制验签 sig::ly_verify_lyx(去尾65B, algo, sig)   ← 失败 Err，审计
 ├─ 注册 KC 槽（私有栈/故障域/策略）
 ├─ 段映射到 KC 代码窗口（base = 0xFFFF_9000_0000_0000 + slot*4MiB，页粒度）
 ├─ 故障域内调 entry(exports) → 返回 *mut LyDriverDesc（内核立即拷贝副本）
 ├─ ABI major 校验 → 注册驱动管理器 → 绑定 KC↔驱动槽
 └─ 故障域内 probe/start；probe=0 → 注销驱动（设备不存在）
```

`entry` 返回的描述符指针：内核在入口返回后立即 `read_volatile` 拷贝，模块栈窗口在拷贝前保持映射；描述符以内核副本为准，模块段永久映射（驱动生命周期与内核一致）。

---

## 6. 示例驱动走读（examples/lykc_hello）

最小 MISC 驱动：入口打印 `exports ok`，probe 恒 1，ioctl 回显命令字。

```
examples/lykc_hello/
├── Cargo.toml        # no_std、panic=abort、独立 .cargo/config（large code model）
├── linker.ld         # 固定基址 KERNEL_BASE+64，段 SUBALIGN(1) 无间隙
├── Makefile          # build → out/lykc_hello.lykc（--algo none / sign KEY=）
└── src/main.rs       # 类型定义 + entry + ops
```

关键约定：

- **固定基址**：`KERNEL_BASE = 0xFFFF_9000_0000_0000`，代码从 `+64` 起（头占 64B），段间无对齐间隙（与镜像平铺布局一致）；
- **code model**：基址超出 32 位寻址范围，必须 `-C code-model=large`（local `.cargo/config.toml` 覆盖仓库根配置）；
- **无外部符号**：只依赖本文 §3 类型定义，不链接内核任何符号。

---

## 7. mklykc 用法

```bash
# 打包（默认 ed25519 签名；cryptography 未装或调试期用 --algo none）
python tools/mklykc.py --elf out/lykc_hello.elf --out out/lykc_hello.lykc \
    [--base 0xFFFF900000000000] [--class 5] [--flags 2] \
    [--algo none|ed25519] [--key sk.pem]
```

工具职责：解析 ELF64 段（`.text/.rodata/.data/.bss`，Rust 每符号独立段 `.ltext.*` 等全捕获）→ 平铺为镜像（头 + 连续段）→ 按 ELF entry VMA 换算镜像内偏移 → 可选 ed25519 签名。签名服务器侧复用 `sign_lyx.py`（payload = .lykc 去尾 65B；完整签名操作手册在私有仓 `SIGNING_GUIDE.md`，公开版不包含服务器操作细节与密钥管理流程）。

---

## 8. 兼容与吊销

- ABI 版本不符（`abi_major != 1`）→ 拒载；`.lykc` 头 version > 1 → 拒载；
- 证书吊销（`sig::ly_cert_revoke`）后装载即拒绝，无需改动驱动镜像；
- 验签失败永不降级放行——**无签名驱动装载即报错，无任何旁路**。
