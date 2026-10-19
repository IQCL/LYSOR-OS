# LYSOR OS 系统架构 — 组件化混合内核

## 分层总览

```
┌────────────────────────────────────────────────────────┐
│ 应用层（原生应用 .lyx / 瞬应用 .lyap）                   │
│   只通过系统 API（syscall / IPC / GFX / WM）与系统交互   │
├────────────────────────────────────────────────────────┤
│ 系统服务层（组件化扩展，KC）                             │
│   图形 / 密码学 / 窗口管理 … 以可选组件形式提供          │
│   独立故障域 + 稳定调用 ABI（ly_kcomp_invoke）           │
├────────────────────────────────────────────────────────┤
│ 内核基座（TCB，最小、可形式化验证）                      │
│   中断/异常 调度器 虚拟内存 IPC syscall 块设备 HAL       │
└────────────────────────────────────────────────────────┘
```

## 内核基座（TCB，最小不可剥离部分）

- 中断/异常：IRQ 分发、缺页、除零、权限违规
- 调度器：PIT 100Hz 抢占 round-robin，TCB 表，栈金丝雀
- 虚拟内存：PML4、bitmap 帧分配、map_page/invlpg
- IPC：消息端口（Channel A）+ 共享内存白名单（Channel B）
- 真 syscall：INT 0x80 trap gate，`write_volatile` 回写 rax
- 块设备 HAL：ATA PIO + LFS 真盘

## 组件（KC）机制

KC（kcomponent）是 LYSOR OS 组件化混合内核的扩展单元，以独立故障域运行、通过稳定 ABI 与内核基座交互。本公开版本**不包含任何 KC 实现**，仅在接口层定义契约：

- KC 拥有私有动态栈与独立故障域；KC 出错不拖垮内核基座
- 禁止热插拔；启用/禁止在下次重启生效
- 统一调用入口：`ly_kcomp_invoke(slot, fn_idx, regs)`，`regs: &mut [u64; 8]`，`regs[0]` 为 fn_idx 对应的第一参数

完整版在此机制上实现图形（`lycomp_gfx`）、密码学（`lycomp_crypto`）等组件。

## 应用可见的 API 面

| API 组 | 入口 | 说明 |
|--------|------|------|
| syscall | INT 0x80 | 文件、IPC、调度让位（见 [SYSCALL_API.md](SYSCALL_API.md)） |
| IPC | `ly_chan_*` / `ly_shm_*` | 消息端口 + 白名单共享内存（见 [IPC_API.md](IPC_API.md)） |
| GFX | `ly_gfx_*`（16 项） | 应用唯一绘图出口，全部转发至图形组件（见 [GFX_API.md](GFX_API.md)） |
| WM | `ly_wm_*` | 窗口生命周期 + 输入事件（见 [WM_API.md](WM_API.md)） |
| XMLUI | `<layout>` 标签 | 瞬应用声明式界面（见 [XMLUI_SPEC.md](XMLUI_SPEC.md)） |
| LYAP | `.lyap` 包 | 瞬应用分发格式（见 [LYAP_SPEC.md](LYAP_SPEC.md)） |

## 图形调用链（唯一出口原则）

```
应用 → ly_gfx_*(转发) → ly_kcomp_invoke(slot, idx, regs)
     → lycomp_gfx（组件侧）→ 栅格化/绘制 → 帧缓冲
```

应用**禁止**直接写帧缓冲；所有绘制统一走 GFX API。

## 自适应资源分配

| 资源 | 策略 |
|---|---|
| 物理内存 | `ram=NNN`(MiB) 启动参数 → fw_cfg RAM_SIZE（≥32MB）→ CMOS 0x35/0x36（扩展>16MB）→ 256MB 默认 |
| 线程栈 | TCB 4 页/线程 |

## 真数据链路（无桩）

- 任务管理器 ← `sched::ly_sched_list`（真 TCB 表 + 真 tick 数）
- 设置 Storage ← `lfs::ly_lfs_stats`（真 superblock 用量）
- 注册表编辑器 ← LFS `/registry/...` 真目录树
- 文件管理器 ← `vfs::ly_vfs_list` 真目录遍历
- 时钟 ← CMOS RTC（0x70/0x71，BCD+世纪寄存器）
- 终端 ← `tty::ly_term_exec` 真命令执行
