# LYSOR OS 系统架构

## 分层总览（微内核红线）

```
┌────────────────────────────────────────────────────────┐
│ 用户态/外壳（单二进制多线程）                            │
│   desktop / wm / apps / tty / uservice / snapapp        │
│   全部通过 gfx KC 调用图形，不直接碰帧缓冲               │
├────────────────────────────────────────────────────────┤
│ KC（kcomponent）＝可选内核组件，驻内核共享内存           │
│   lycomp_gfx（图形子系统：渲染唯一出口）                 │
│   lycomp_crypto（国密双轨 + Ed25519，可选）              │
│   私有动态栈 + 独立故障域 + 策略启用/禁止（下次重启生效）│
│   禁止热插拔；错误策略：捕获→禁用→重启 loader→持久化    │
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

## KC（kcomponent）组件层（不污染微内核的可选组件）

| 组件 | slot | 内容 |
|------|------|------|
| lycomp_crypto | 0 | 国密 SM2/SM3/SM4（优先）+ Ed25519 双轨，自检 + tamper reject |
| lycomp_gfx | 1 | 图形子系统：16 项 API（clear/fill/text/rect/rrect/line/pixel/wallpaper/blit…），桌面/窗口/应用唯一的图形出口 |

### KC 故障隔离与错误策略

1. ISR 捕获 KC 域异常（CURRENT_KC 有效时）
2. 策略标记 `-1`（fault-locked，不可再启用）
3. loader 重启（普通上下文）统一持久化到 `/etc/kcomp.conf`
4. 启用/禁止在**下次重启**生效；禁止热插拔

## 自适应资源分配

| 资源 | 策略 |
|---|---|
| 物理内存 | `ram=NNN`(MiB) 启动参数 → fw_cfg RAM_SIZE（≥32MB）→ CMOS 0x35/0x36（扩展>16MB）→ 256MB 默认 |
| KC 栈 | 注册时帧分配器按需分配 4 页，映射到 `0xFFFF800000400000 + slot*16KB`，栈顶留 16 字节 guard；注销时 unmap+归还 |
| 线程栈 | TCB 固定 4 页/线程（后续版本按需分配） |

## 用户态/外壳层

| 服务 | 模块 |
|------|------|
| 桌面外壳 shell | desktop（任务栏/开始菜单/右键菜单/动画/电源，真输入路由） |
| 窗口管理 | wm（命中测试/焦点/绘制 API/动画 tick/拖动，绘制全部转发 gfx KC） |
| 系统软件 | apps（10 个应用，全部真数据） |
| 终端 | tty（真命令集：ls/cd/pwd/cat/mkdir/rm/run/ps/kcmd/echo/date/…） |
| 组件签名 | sig（SM2 国密轨 + Ed25519 双轨验证） |

## 图形调用链（唯一出口原则）

```
应用/桌面 → crate::gfx::ly_gfx_*(转发) → ly_kcomp_invoke(slot, idx, regs)
         → lycomp_gfx（KC 私有栈）→ 栅格化/绘制 → FB
```

应用与桌面**禁止**直接写帧缓冲；壁纸绘制也统一走 gfx（`blit_wallpaper_at` 已在 KC 侧）。

## 真数据链路（无桩）

- 任务管理器 ← `sched::ly_sched_list`（真 TCB 表 + 真 tick 数）
- 设置 Storage ← `lfs::ly_lfs_stats`（真 superblock 用量）
- 注册表编辑器 ← LFS `/registry/...` 真目录树
- 文件管理器 ← `vfs::ly_vfs_list` 真目录遍历
- 时钟 ← CMOS RTC（0x70/0x71，BCD+世纪寄存器），40px 矢量字
- 终端 ← `tty::ly_term_exec` 真命令执行 + serial 捕获钩子回显

## 素材资产（用户提供，脚本直转禁 OCR）

- `assets::logo.rs` ← `path2.png`（用户光栅化，透明 + LANCZOS 96×96）
- `assets::wallpaper_landscape.bin` / `wallpaper_portrait.bin` ← 用户两张壁纸（cover-crop RGB888）
- 字体：`assets\LXGWWenKai-Regular.ttf`（SIL OFL 1.1 开源无版权风险，37136 glyphs，cmap4+cmap12 全 UTF-8）

## 文档索引

- `GUI_DEV_GUIDE.md`：渲染/动画接入指南
- `WM_API.md`：窗口管理器 API
- `XMLUI_SPEC.md`：声明式 UI 规范
- `LFS_SPEC.md`：日志文件系统格式
- `LYAP_MANIFEST.md`：瞬时应用包格式
- `TESTING.md`：验证流程与 boot 锚点

## 已知边界（诚实声明）

- 网络：e1000 驱动 + ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP 客户端 + **TLS 1.2 客户端**（冒烟 `tls: E2E OK bytes=175` 通过）；DHCP/TCP 服务端监听/浏览器 https 接线待实现
- `exec::ly_exec_lyx` 已是 ring 3 真进程（独立地址空间 + CR3 切换 + 逐帧回收），非内核上下文直跑
- 内核态/用户态地址空间分离、SMP 4 核实测已实现（详见 ROADMAP.md）
- init/auth/cgroup/cron/ntp/dbus/pm/udev 为框架日志（ready/starting）
- 鼠标点击/拖动路径代码已落（hit_test/DRAG_WIN），QEMU 无 display 下 HMP 鼠标注入不产生 aux 字节（环境限制），仅键盘路径实证；真机路径待验证
