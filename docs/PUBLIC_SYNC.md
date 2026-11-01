# LYSOR OS 公开仓同步策略（TCB-only：只开最小可信基座）

> 配套：`docs/DEVELOPER_GUIDE.md`、`docs/REDACTION_LOG.md`。
> 原则（2026-11-01 用户拍板）：**公开仓只开源 TCB 最小基座**；内核组件（KC）、
> 预装软件、用户态服务全部不公开。TCB 文件内若引用非 TCB 内容，留桩
> （`// [REDACTED: ...]` 占位注释）。**公开版不用跑通，只保证代码与私有版
> 的 TCB 部分一致**（留桩位除外）。

## 1. 分级

| 级别 | 内容 | 公开仓 |
|------|------|--------|
| P0 可信集 | 架构文档、白皮书、开发指南、SIGNING_GUIDE、SERVER-CONFIG、PUBLIC_SYNC、REDACTION_LOG | ✅ 全公开 |
| TCB 基座 | 中断与异常分发、抢占式调度、物理/虚拟内存管理、双通道 IPC、系统调用入口、块设备抽象（见 §5 清单） | ✅ 公开，TCB 内非 TCB 引用留桩 |
| KC / 预装 / 用户态 | 内核组件（kcomponent/component/drivers/net/crypto/gfx/font 等）、应用装载（exec/lyap/uservice/snapapp/shan/apps/wm/desktop/ui/xmlui）、用户态服务（auth/cgroup/coreutils/cron/dbus/init/ntp/pm/security 策略层/tty/udev/vga）、预装应用（embed/*.lyx） | ❌ 不公开（public 无此目录/文件） |
| P2 隐私 | 私钥/证书种子/内部地址/真实 dev id/内部端点/审计细节 | ❌ 任何仓禁止（见红线） |
| P3 工作产物 | 构建日志、临时诊断、冒烟日志、调试脚本 | ❌ 不入库 |

## 2. 公开版"代码一致"规则

- 公开仓每个 TCB 文件的字节级来源 = 私有仓同一文件（**留桩除外**）；
- 留桩方式：TCB 文件内对非 TCB 模块的调用/引用替换为 `// [REDACTED: 原因]`
  占位注释；被引用模块的**源码不进 public**（目录/文件移除）；
- 例外：`security.rs`（栈金丝雀）属 TCB 栈安全基座，保留；`fs/lfs.rs` 的
  块设备 I/O 层留桩（驱动 KC 接口不进 public），文件系统算法层保留；
- 留桩点记录在 `docs/REDACTION_LOG.md`。**没有记录 = 不允许留桩**。

## 3. 同步流程（公开仓同步执行）

```
1. git -C public status --porcelain    # 公开仓当前状态
2. TCB 判定：文件是否属于 §5 清单？
   - 属于 → 与主仓比对，若有差异则 Copy 覆盖；
   - 不属于（KC/预装/用户态）→ public 移除（git rm）；
   - TCB 文件内非 TCB 引用 → 留桩（REDACTION_LOG 记录）。
3. 全量扫描：public/src 对非 TCB 模块的 crate:: 引用 = 0（注释措辞不含 crate:: 路径）；
   *.pem/*.key/*seed/priv 零命中。
4. git -C public add/commit；push（需网络，触发确认）
```

## 4. 红线（任何情况下禁止入库）

| 内容 | 规则 |
|------|------|
| `*.pem` / `*.key` / `d.hex` / `*priv*` / `*.seed` | .gitignore 已列；commit 前 grep 复查 |
| 私钥种子（out/dev-ed25519.seed 等） | 禁入任何仓，out/ 整体忽略 |
| 真实 dev id / 内部域名 / 内部 API 端点 | 文档用占位符，代码用常量引用外部配置 |
| 已签名应用二进制（embed/*.lyx） | 主仓保留，public 无 embed/ 目录 |
| diag / TEMP-DIAG 日志代码 | 合并前移除或留桩标记 |
| 冒烟日志 / 构建日志 | logs/、build_*.txt 已忽略 |

## 5. TCB 最小基座清单（public/src 实际内容，2026-11-01）

```
src/main.rs            # 内核入口（非 TCB 初始化段留桩）
src/gdt.rs  src/idt.rs # 分段/中断与异常分发
src/pic.rs             # PIC8259 中断控制器
src/serial.rs  src/syslog.rs  # 串口/内核日志
src/security.rs        # 栈金丝雀（TCB 栈安全）
src/memory/{mod,paging,physical}.rs  # 物理/虚拟内存管理（含内核堆）
src/sched/{mod,capability,context,pit}.rs  # 抢占式调度 + 能力 + 上下文 + PIT
src/smp/{mod,lapic,cpu_local,spinlock}.rs  # SMP（LAPIC AP 唤醒/自旋锁）
src/ipc/{mod,channel_a,channel_b,whitelist}.rs  # 双通道 IPC（ring buffer + shm + 白名单）
src/syscall/mod.rs     # 系统调用入口（net 面留桩）
src/tcb/mod.rs         # TCB 结构/权限表
src/fs/{mod,lfs}.rs    # 块设备抽象（LFS 卷，块 I/O 层留桩）
src/vfs/{mod,memfs}.rs # 文件系统抽象（memfs + LFS 挂载面）
```

**不进 public（KC/预装/用户态）**：auth cgroup coreutils cron dbus drivers/ exec/
init net ntp pm security(策略层已并入 TCB 金丝雀后其余不公开) tty udev vga
kcomponent component crypto sig wm ui xmlui desktop apps assets font gfx
uservice snapapp shan lyap + embed/ + tools/（含签名工具）。

## 6. 验证

- 每次同步后：`git -C public status --porcelain` 干净；
- `public/src` 对非 TCB 模块引用 = 0（代码级，注释不匹配 crate:: 路径）；
- `grep -riE "\.(pem|key)|priv|seed" public` 零命中；
- 留桩点数量 = REDACTION_LOG 条目数（只减不增）。
