# LYSOR OS — 微内核操作系统（no_std, x86_64, Rust）

自研 **微内核 TCB** 操作系统：Rust `no_std` 单 crate 单二进制，QEMU x86_64 启动。前缀 `ly_`。许可证 **MPL-2.0**。

> 本仓库为公开审查版，仅包含**最小可信计算基（TCB）**：中断/异常、调度器、虚拟内存、IPC、syscall、块设备 HAL、VFS。图形子系统、窗口管理、桌面、应用、密码学组件、用户态服务等均属私有子系统，不在本仓库中。

## 设计红线（架构准则）

- **微内核 TCB 可形式化验证**：调度、内存、IPC、IDT/中断、系统调用仅保留最小集合，图形/桌面/应用一律不进内核。
- **可选内核组件（KC）**：驻内核共享内存空间的扩展机制，本公开版本不含任何 KC 实现，仅保留接口契约。
- **密码学双轨（可选）**：国密系列（SM2/SM3/SM4）+ SHA-512/Ed25519 系列，本公开版本不含密码学实现，签名验证恒拒绝。

## 快速开始

```powershell
# 构建（release，产出 target\x86_64-unknown-none\release\lysor-os）
cargo build --release

# QEMU 启动（无头模式，串口输出）
qemu-system-x86_64.exe -kernel target\x86_64-unknown-none\release\lysor-os -m 256M -nographic -serial stdio -no-reboot
```

按 `Ctrl+A` 然后 `X` 退出 QEMU。

## 自适应资源分配

| 资源 | 策略 |
|---|---|
| 物理内存 | `ram=NNN`(MiB) 启动参数 → fw_cfg RAM_SIZE → CMOS 0x35/0x36 → 256MB 默认 |
| 线程栈 | TCB 固定 4 页/线程 |

## 启动链

```
PVH boot（multiboot2 兼容头）
  → 32 位 stub（cli, 建栈, 填充启动页表）
  → CR3 ← boot_p4, EFER.LME, CR0.PG → 长模式
  → kernel_main()（Rust）
```

## 仓库结构

```
├── src\
│   ├── main.rs            # 启动序列、root 挂载、自测
│   ├── memory\            # 物理帧分配器 + 分页（自适应 RAM 探测）
│   ├── sched\             # 调度器（PIT 抢占 + TCB + 栈金丝雀）
│   ├── ipc\               # IPC 双通道（消息端口 + 共享内存白名单）
│   ├── syscall\           # INT 0x80 系统调用
│   ├── tcb\               # 可信计算基边界
│   ├── vfs\               # 虚拟文件系统
│   ├── fs\                # LFS 文件系统实现
│   ├── drivers\           # 块设备/输入/时钟驱动
│   ├── exec\              # LYX 可执行加载器
│   ├── sig\               # 签名验证接口契约
│   └── ...
├── docs\                  # 公开开发文档
└── Cargo.toml
```

## 公开版与完整版的区别

| 模块 | 公开版 | 完整版（私有仓库） |
|------|--------|-------------------|
| 微内核 TCB | ✅ 完整 | ✅ 完整 |
| KC 框架 | 接口契约 | 完整实现 |
| 图形子系统 | ❌ | ✅ |
| 窗口管理/桌面/应用 | ❌ | ✅ |
| 密码学组件 | ❌（恒拒绝） | ✅ |
| 矢量字体引擎 | ❌ | ✅ |
