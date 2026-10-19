# LYSOR OS — 组件化混合内核操作系统（no_std, x86_64, Rust）

自研 **组件化混合内核（Componentized Hybrid Kernel）** 操作系统：Rust `no_std` 单 crate，QEMU x86_64 启动。符号前缀 `ly_`。许可证 **MPL-2.0**。

> 本仓库为公开审查版，仅包含**最小可信计算基（TCB）**：中断/异常、调度器、虚拟内存、IPC、syscall、块设备 HAL、VFS/LFS。图形子系统、窗口管理、桌面、应用、密码学组件、用户态服务等均属私有子系统，不在本仓库中。**审查者可对 TCB 部分做形式化验证**。

## 架构准则（红线）

- **TCB 最小化、可形式化验证**：调度、内存、IPC、IDT/中断、系统调用仅保留最小集合；图形/桌面/应用一律不进内核基座。
- **组件（KC）＝可选内核组件**：以独立故障域运行的可选扩展，通过稳定 ABI 与内核基座交互；本公开版本不含任何 KC 实现，仅保留接口契约。
- **密码学双轨（可选组件）**：国密系列（SM2/SM3/SM4，GB/T 32905/32918/32907）优先 + SHA-512/Ed25519（RFC 8032）备用；公开版不含实现，签名验证显式拒绝。

## 快速开始

```bash
# 构建（release，产出 target/x86_64-unknown-none/release/lysor-os）
cargo build --release

# QEMU 启动（无头模式，串口输出）
qemu-system-x86_64 -kernel target/x86_64-unknown-none/release/lysor-os \
    -m 256M -nographic -serial stdio -no-reboot
```

按 `Ctrl+A` 然后 `X` 退出 QEMU。

## 启动链

```
PVH boot（multiboot2 兼容头）
  → 32 位 stub（cli, 建栈, 填充启动页表）
  → CR3 ← boot_p4, EFER.LME, CR0.PG → 长模式
  → kernel_main()（Rust）
```

## 仓库结构

```
├── src/
│   ├── main.rs            # 启动序列、root 挂载、内核自测
│   ├── memory/            # 物理帧分配器 + PML4 分页（自适应 RAM 探测）
│   ├── sched/             # 调度器（PIT 抢占 + TCB + 栈金丝雀）
│   ├── ipc/               # IPC 双通道（消息端口 + 共享内存白名单）
│   ├── syscall/           # INT 0x80 系统调用
│   ├── tcb/               # 可信计算基边界
│   ├── vfs/  fs/          # 虚拟文件系统 + LFS 文件系统
│   ├── drivers/           # ATA/PCI/PS2/RTC 驱动
│   ├── exec/              # LYX 可执行加载器
│   └── sig/               # 签名验证接口契约
├── docs/                  # 公开开发文档（应用开发者从这里开始）
└── Cargo.toml
```

## 开发者文档索引

| 文档 | 内容 |
|------|------|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | 组件化混合内核分层架构 |
| [WHITEPAPER.md](docs/WHITEPAPER.md) | 官方白皮书（定位 / 架构思想 / 新旧对比 / 取舍） |
| [APP_DEV_GUIDE.md](docs/APP_DEV_GUIDE.md) | 应用开发入门（原生应用 / 瞬应用） |
| [SYSCALL_API.md](docs/SYSCALL_API.md) | 系统调用 ABI 完整参考 |
| [IPC_API.md](docs/IPC_API.md) | IPC 消息端口与共享内存 |
| [GFX_API.md](docs/GFX_API.md) | 图形 API（应用唯一绘图出口） |
| [WM_API.md](docs/WM_API.md) | 窗口管理 API |
| [XMLUI_SPEC.md](docs/XMLUI_SPEC.md) | 声明式 XML 界面规范 |
| [LYAP_SPEC.md](docs/LYAP_SPEC.md) | 瞬应用包（.lyap）格式规范 |
| [LFS_SPEC.md](docs/LFS_SPEC.md) | LFS 文件系统磁盘格式 |
| [TESTING.md](docs/TESTING.md) | 构建与验证指南 |
| [ROADMAP.md](docs/ROADMAP.md) | 开发路线图 |

## 公开版与完整版的区别

| 模块 | 公开版 | 完整版 |
|------|--------|--------|
| 内核基座（TCB） | ✅ 完整（审查重点） | ✅ 完整 |
| 组件框架（KC） | 接口契约 | 完整实现 |
| 图形/窗口/桌面/应用 | ❌ | ✅ |
| 密码学组件 | ❌（恒拒绝） | ✅ |
| 矢量字体引擎 | ❌ | ✅ |
