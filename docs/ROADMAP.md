# LYSOR OS 开发路线图

## 已完成

### Phase 1 — 内核基座 ✅
- [x] PVH 启动 → 长模式（GDT/IDT 256 门/串口）
- [x] 物理内存 bitmap + PML4 虚拟页表
- [x] PIT 100Hz 抢占式调度 + 栈金丝雀（每线程 arm）
- [x] IPC 双通道 + 白名单
- [x] 真 syscall（INT 0x80, open/write/ipc/yield/ENOSYS 自检）
- [x] ATA PIO 真块设备 + LFS 落盘
- [x] 跨 boot 持久化验证（bootcount 递增 + 文件往返 + 目录完整）

## 进行中 / 待办

### 微内核增强
- [ ] SMP 多核 + 自旋锁
- [ ] SLAB 分配器
- [ ] 内核态/用户态地址空间分离（当前单地址空间多线程）
- [ ] `exec::ly_exec_lyx` 迁到用户态承载（当前内核上下文直跑）

### 网络栈
- [ ] e1000 网卡驱动（MMIO）
- [ ] ARP/IP/ICMP/TCP/UDP 真实现（当前骨架）
- [ ] DNS/DHCP + socket syscall

### 用户态服务
- [ ] init PID1 真服务管理（当前框架日志）
- [ ] auth/cgroup/cron/ntp/dbus/pm/udev 正式实现

### 驱动
- [ ] 声卡/蓝牙
- [ ] 网络文件共享（NFS/SMB）

## 验证基线

```
rtc: 2026/10/18
DISK PERSISTENCE VERIFIED (cross-boot)
```
