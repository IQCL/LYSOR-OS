# LYSOR OS 系统架构

## 分层总览（微内核红线）

```
┌────────────────────────────────────────────────────────┐
│ 可选扩展层（私有子系统，不在本公开仓库）                 │
│   图形 / 窗口管理 / 桌面 / 应用 / 密码学组件 / 用户服务   │
├────────────────────────────────────────────────────────┤
│ 微内核 TCB（最小、可形式化验证）                        │
│   中断/异常 调度器 虚拟内存 IPC syscall 块设备 HAL       │
└────────────────────────────────────────────────────────┘
```

## 微内核（TCB，最小不可剥离部分）

- 中断/异常：IRQ 分发、缺页、除零、权限违规
- 调度器：PIT 100Hz 抢占 round-robin，TCB[16]，栈金丝雀
- 虚拟内存：PML4、bitmap 帧分配、map_page/invlpg
- IPC：消息端口（Channel A）+ 共享内存白名单（Channel B）
- 真 syscall：INT 0x80 trap gate，`write_volatile` 回写 rax
- 块设备 HAL：ATA PIO + LFS 真盘

## 可选内核组件（KC）机制

KC（kcomponent）是驻内核共享内存空间的可选扩展组件，本公开版本**不包含任何 KC 实现**，仅在接口层定义契约：

- KC 拥有私有动态栈与独立故障域
- 禁止热插拔；错误策略：捕获→禁用→重启 loader→持久化
- 启用/禁止在下次重启生效

完整版（私有仓库）在此机制上实现图形、密码学等组件。

## 自适应资源分配

| 资源 | 策略 |
|---|---|
| 物理内存 | `ram=NNN`(MiB) 启动参数 → fw_cfg RAM_SIZE（≥32MB）→ CMOS 0x35/0x36（扩展>16MB）→ 256MB 默认 |
| 线程栈 | TCB 固定 4 页/线程（后续版本按需分配） |

## 真数据链路（无桩）

- 任务管理器 ← `sched::ly_sched_list`（真 TCB 表 + 真 tick 数）
- 设置 Storage ← `lfs::ly_lfs_stats`（真 superblock 用量）
- 注册表编辑器 ← LFS `/registry/...` 真目录树
- 文件管理器 ← `vfs::ly_vfs_list` 真目录遍历
- 时钟 ← CMOS RTC（0x70/0x71，BCD+世纪寄存器）
- 终端 ← `tty::ly_term_exec` 真命令执行 + serial 捕获钩子回显

## 系统调用 ABI

| nr | 名称 | 参数 | 返回 |
|----|------|------|------|
| 0 | read | fd, buf, count | 字节数 |
| 1 | write | fd, buf, count | 字节数 |
| 2 | open | path, flags, mode | fd |
| 3 | close | fd | 0 |
| 4 | ipc_send | ch, msg, _ | 0 |
| 5 | ipc_recv | ch, msg, _ | 字节数 |
| 6 | sched_yield | - | 0 |

未知 nr 返回 `-ENOSYS`（-38）。
