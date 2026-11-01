# LYSOR OS — 构建与验证测试文档

> 与代码同步维护（v0.2.0）。验证以**冒烟日志断言**为收口标准：
> 内核串口日志中出现指定断言行即通过，不依赖人工目测。

## 1. 构建前置

- Rust stable toolchain（`rustup`）+ `x86_64-unknown-none` target + `rust-lld`。
- QEMU（官方版 `qemu-system-x86_64`，本机 `F:\qemu\qemu-system-x86_64.exe`）。
- cargo 不在 PATH 时用全路径：`& "$env:USERPROFILE\.cargo\bin\cargo.exe"`。
- python 沙箱路径（无密码库；TLS 服务器用纯 python 实现）。

## 2. 构建

```powershell
cd "F:\lysor os"
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release
```

产物：`target\x86_64-unknown-none\release\lysor-os`。
要求：**零警告**（#6 收口）。

## 3. 运行（冒烟）

```powershell
# 一键冒烟：QMP 探针 + 自动起 127.0.0.1:18443 python TLS 服务器
& "F:\lysor os\tools\run-smoke.ps1"          # 默认 78s 窗口
& "F:\lysor os\tools\run-smoke.ps1" -WaitSec 300   # 长窗口（TCG 下 Ed25519 大数验签慢）
```

日志：`logs\smoke-N.log`（内核串口）+ `logs\tls_srv.smoke.out`（TLS 服务器权威输出）。

## 4. 冒烟断言基线（出现即 PASS）

| 断言行 | 含义 |
|---|---|
| `crypto: AES-128 FIPS C.1 OK` | AES 基元过 FIPS 197 附录 C.1 |
| `crypto: SHA-256 vector OK` | SHA-256 过标准向量 |
| `rsa: RSA-2048 SHA256 verify OK` | RSA-SHA256 验签 |
| `ed25519: SHA-512 vector + sign/verify + tamper reject OK` | Ed25519 自检 |
| `shan: 0 1 2 3 4 13 shan-ok gfx-ok` | 瞬语言解释器 + 图形 API 自检 |
| `lyx: ring3 selftest` → `exit(0x2a)` | 原生应用 ring3 全链（签名→验签→建址→iretq→用户代码→syscall 退出→回收） |
| `tls: E2E OK bytes=175` | **TLS 1.2 客户端端到端闭环**（ECDHE-RSA-AES128-GCM-SHA256，body 含 OK） |
| `play-tips` → `exit(0x54)` | 玩机技巧瞬应用（独立预装，Ed25519 签 .lyx 进 embed） |
| `>> SMP OK: all online CPUs busy` | SMP 4 核实测（per-CPU worker 计数 ≥1000） |
| `vfs: root on /dev/sda OK`（真盘模式） | LFS 真盘持久化 |

## 5. 手动 TLS 对拍（无 QMP）

```powershell
& "F:\lysor os\tools\run-manual-tls.ps1"     # 120s 无探针；日志 logs/manual-tls.log
```

权威服务器参考实现 `tools/tls_server.py`（纯 python：AES-FIPS 197 / GCM-NIST SP 800-38D /
TLS 1.2 记录层），打印 `SEND-PROTECTED`/`RECV-PROTECTED` 与内核对拍。

## 6. 测试矩阵（现状）

| 域 | 验证 | 状态 |
|---|---|---|
| 引导 | PVH → 长模式 → kernel_main | PASS |
| 内存 | 帧分配器 + PML4 分页 + 每进程独立地址空间（CR3 切换） | PASS |
| 调度 | 抢占式 + TCB + 栈金丝雀；SMP 4 核（IPI + 自旋锁） | PASS |
| 密码 | SM2/SM4（国密）+ Ed25519/SHA-512 + RSA（Montgomery）+ AES-GCM | PASS |
| 网络 | e1000 + ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP + **TLS 1.2** | PASS（E2E OK） |
| 文件 | LFS 真盘持久化（跨重启） | PASS |
| 应用 | .lyx ring3 真进程 + .lyap 瞬应用 + 三态验签门 | PASS |
| 桌面 | 窗口/任务栏/开始菜单/动画（短促非模态） | PASS |
| 浏览器 | 轻量自研渲染 + CSS 子集 + CSP；TLS 接线为下一步 | 🔶 部分 |
| OAuth2 | 设备码/轮询/userinfo 会话；8087 本机代理待实现 | 🔶 部分 |

## 7. 已知未实现（与白皮书/ROADMAP 一致）

DHCP、TCP 服务端监听、浏览器 https 接线（TLS_HTML 占位页替换、/etc/tls 证书加载）、
SLAB 分配器、组件进程化隔离、声卡/蓝牙/网络文件共享、init/cgroup/cron/ntp/dbus/pm/udev 正式化。

## 8. 环境注意

- 无头模式下鼠标点击注入不可用（QEMU 无 display）；带显示环境（`-display` 窗口）可实测。
- TCG 下 Ed25519 大消息验签（8223B）较慢（~5-15 分钟），用长窗口冒烟（`-WaitSec 900`）。
- 官方 QEMU 版本差异：e1000 TX DD 回写在非 tag 版本兼容性待对照（结论：版本差异，非代码错误）。
