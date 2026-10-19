# LYSOR OS 开发路线图

## 已完成

### Phase 1 — 内核基座 ✅
- [x] PVH 启动 → 长模式（GDT/IDT 256 门/串口）
- [x] 物理内存 bitmap + PML4 虚拟页表
- [x] PIT 100Hz 抢占式调度 + 栈金丝雀（每线程 arm）
- [x] IPC 双通道 + 白名单
- [x] 真 syscall（INT 0x80, open/read/write/close/ipc/yield/ENOSYS 自检）
- [x] ATA PIO 真块设备 + LFS 落盘
- [x] 跨 boot 持久化验证（bootcount 递增 + 文件往返 + 目录完整）
- [x] LYX 可执行格式 + 双轨签名装载契约（SM2 优先 / Ed25519）

## 进行中 / 待办

### 内核基座增强
- [ ] SMP 多核 + 自旋锁
- [ ] SLAB 分配器
- [ ] 地址空间隔离强化（组件与应用按最小权限运行）

### 网络栈
- [ ] e1000 网卡驱动（MMIO）
- [ ] ARP/IP/ICMP/TCP/UDP 实现
- [ ] DNS/DHCP + socket syscall

### 系统服务
- [ ] init PID1 服务管理
- [ ] auth/cgroup/cron/ntp/dbus/pm/udev 正式实现

### 驱动
- [ ] 声卡/蓝牙
- [ ] 网络文件共享（NFS/SMB）

## 验证基线

```
rtc: 2026/10/18
DISK PERSISTENCE VERIFIED (cross-boot)
```
