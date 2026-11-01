# LYSOR OS 开发路线图（2026-10-24 更新）

## 已完成（全绿验证）

### Phase 1 — 内核基座 ✅
- [x] PVH 启动 → 长模式（GDT/IDT 256 门/串口）
- [x] 物理内存 bitmap + PML4 虚拟页表
- [x] PIT 100Hz 抢占式调度 + 栈金丝雀（每线程 arm）
- [x] IPC 双通道 + 白名单
- [x] 真 syscall（INT 0x80, open/write/ipc/yield/ENOSYS 自检）
- [x] 密码学双轨 kcomponent：国密 SM2/SM3/SM4（GB/T 向量）+ Ed25519（RFC 8032），**双轨并行、可选、驻留 kcomponent 不污染微内核**

### Phase 2 — 图形桌面 ✅
- [x] stdvga 真初始化：PCI BAR + Bochs VBE 800x600x32 + LFB 映射
- [x] wm 窗口管理 + 绘制 API + Bresenham
- [x] 真输入路由（PS/2 键盘完整事件码 + CapsLock/Win/Alt/F4/F10/方向键/E0；PS/2 鼠标 aux-only pump 先查 bit5 防吞键盘字节）
- [x] Win11 风格桌面（居中任务栏 + logo 位图 + PINNED + RTC 时钟）
- [x] 开始菜单（3×3 圆角 tile + 搜索框 + 用户/电源 + 键盘导航 + slide-up 动画）
- [x] Shift+F10 右键菜单 / 窗口 scale-in 动画 / 标题栏拖动 / Alt+F4 关机（ACPI S5 + 8042 fallback）
- [x] 个性化壁纸（横/竖两张用户素材，Settings Personalization 切换）
- [x] 矢量字体：LXGW WenKai（OFL 1.1）内嵌 + TrueType 栅格化 + 全 UTF-8 cmap（NUL 终止符 bug 已修）

### Phase 3 — 系统软件 ✅（对照 Windows 系统配置工具清单 + 用户点名，共 10 个）
- [x] 任务管理器（真 TCB/tick）    [x] 注册表编辑器（真 LFS 树）
- [x] 控制面板（应用瓷砖网格）     [x] 设置中心（About/Storage/Display/Personalization）
- [x] 记事本（LFS 持久化 /home/user/notes.txt）
- [x] 计算器（ACC/OP/CUR 状态机 + 键盘/网格点击，10 数字像素级正立实证）
- [x] 时钟/日历（CMOS RTC + 2026-10 日历）
- [x] 画图（自由绘制 + 调色板）   [x] 文件管理器（真 VFS 目录树）
- [x] 终端（真 tty 命令集 + 串口捕获回显）
- [x] 桌面时钟部件（boot 自动开启）

### Phase 4 — 持久化 ✅
- [x] ATA PIO 真块设备 + LFS 64MiB 落盘
- [x] fsck 修复（load_bitmaps 先于 fsck——boot29 错乱盘根因）
- [x] 跨 boot 持久化验证（bootcount 递增 + 文件往返 + 目录完整）

### Phase 5 — 桌面体验重构 ✅（用户素材 + 实证闭环）
- [x] 用户光栅化 logo `path2.png` → 透明 + LANCZOS → `src\assets\logo.rs`（禁 OCR、禁拆 shape）
- [x] 横/竖壁纸 cover-crop → `src\assets\wallpaper_*.bin`
- [x] 开始菜单键盘导航（方向键 + Enter 打开应用，scan_sel 像素实证）
- [x] 右键菜单 5 项（Terminal/Settings/Personalization/Power off/Restart）
- [x] 关机链路实证：`sendkey alt-f4` 组合键 → ACPI S5 → 8042 fallback → QEMU 退出（boot78/85/90）
- [x] Uptime NUL 终止符修复（8888 串 → 单个数字，boot89 实证）

## 进行中 / 待办

### 网络栈
- [x] e1000 网卡驱动（MMIO 轮询模型，QEMU user-net 上电，网关/DNS 自动配置）
- [x] ARP（请求/缓存/应答）、IPv4 收发/校验和、ICMP echo、UDP/DNS 客户端、TCP 客户端（三次握手/ACK/FIN/简单重传）、HTTP/1.1 客户端（GET/POST、双解码、重定向 1 跳）
- [x] **TLS 1.2 客户端**（ECDHE-RSA-AES128-GCM-SHA256：RSA-SHA256 验签 → PRF 派钥 → GCM 记录 AEAD；冒烟 `tls: E2E OK bytes=175` 通过，服务器纯 python 权威实现对拍）
- [ ] 浏览器 https 接线（TLS_HTML 占位页替换、/etc/tls 证书加载）、DHCP 自动获取、TCP 服务端监听、e1000 TX 描述符 DD 回写在非 tag QEMU 版本上的兼容

### 用户态服务
- [x] `exec::ly_exec_lyx` 真进程装载（独立地址空间 + ring 3，替换内核上下文直跑）
- [ ] init PID1 真服务管理（当前框架日志）
- [ ] auth/cgroup/cron/ntp/dbus/pm/udev 正式实现
- [x] 内核态/用户态地址空间分离（每进程独立页表根 + CR3 切换 + TSS/ring3 + 退出逐帧回收）

### 系统软件增强
- [ ] 窗口缩放、多窗口并发
- [ ] 记事本多行滚动、计算器科学模式
- [ ] 设置中心亮度/分辨率热切换（VBE 重设）
- [ ] 鼠标点击/拖动路径真机验证（QEMU 无 display 下 HMP 鼠标注入不产生 aux 字节，环境限制）

### 底层
- [x] SMP 多核 + 自旋锁（2026-10-21 交付：AP 经 LAPIC IPI 唤醒，每 CPU 独立 TSS/GS_BASE/本地 IDT，BSP PIT tick 发固定 IPI 驱动 AP 调度；TCB_LOCK 自旋锁保护 TCB 表；`-smp 4` 实测 4 核全并行（per-CPU worker 计数 21 万+），单核无回归，多核下 BSOD 取证验证通过）
- [x] 内核堆分配器（first-fit 空闲链表 + 边界标记，`ly_heap`）
- [ ] SLAB 分配器（基于现有内核堆优化小对象路径）
- [ ] 声卡/蓝牙
- [ ] 网络文件共享（NFS/SMB）

## 验证基线（boot90, 2026-10-18）
```
rtc: 2026/10/18 08:21
DISK PERSISTENCE VERIFIED (cross-boot)
desktop: shell ready (Win11 style, user logo + wallpapers)
desktop: shell loop started (real input routing)
apps: Settings opened            (键盘导航实证)
desktop: Alt+F4 -> shutdown       (ACPI S5 + 8042 fallback)
SHUTDOWN_EXITED: True            (QEMU 退出实证)
```

## SMP 验证基线（-smp 4, 2026-10-21）
```
>> spawning SMP per-CPU workers...
   sched: created tid=2..5 "smp_worker" aff=cpu0..cpu3
>> SMP verification: per-CPU worker counters (after 5s)
   cpu0 counter=243778 OK
   cpu1 counter=223511 ticks=3 OK
   cpu2 counter=260877 ticks=3 OK
   cpu3 counter=217199 ticks=3 OK
>> SMP OK: all online CPUs busy
```
- 单核回归：无 `-smp` 启动正常，SMP 验证单核同样通过。
- 多核 BSOD 取证：`smp_bsod_test` feature 触发 #PF，[BSOD] vec/rip/CR2/CR3/RSP 完整打印。
