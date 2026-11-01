# LYSOR OS 开发者指南 v1.0

> 配套：`docs/WHITEPAPER.md`（架构白皮书）、`docs/SIGNING_GUIDE.md`（验签体系）、
> `docs/SHAN_LANG.md`（瞬语言规范）、`docs/LYAP_MANIFEST.md`（瞬应用包格式）、
> `docs/BROWSER_PLAN.md`（浏览器/WebView 选型）、`docs/PUBLIC_SYNC.md`（公开仓策略）。

## 0. 系统分层（开发者必读）

LYSOR OS 是"不兼容他人软件"的通用系统：**所有 API 均为自有**，不提供任何
第三方兼容层。代码分四层，迁移/新软件**只允许进 UC 或作为预装原生应用**，
禁止塞进内核或 KC。

| 层 | 位置 | 职责 | 可审查性 |
|----|------|------|----------|
| 内核（kernel） | `src/` 根模块（main/wm/gfx/sched/exec…） | 调度、内存、中断、图形、进程/线程、验签装载 | 最小可信集；改动需白皮书对齐 |
| KC（内核组件） | `src/kcomponent/` | **只放需要速度的基础库**（crypto、font、vfs 原语…） | 精简单可审查，禁止膨胀 |
| UC（用户组件） | 应用/服务（原生 .lyx / 瞬应用 .lyap） | **软件迁移主战场**：把软件改写成自有 API 的真应用 | 任意扩展，不碰内核 |
| 预装软件 | `/apps/`（此电脑/网络/文件管理器/设置/应用商店…） | 系统自带，但**是软件不是内核**，独立签名独立卸载 | 与内核解耦 |

**硬规则**：
- KC 只进基础库（速度敏感、被广泛依赖），否则 panic 级错误处理；
- UC 承担一切业务软件；预装软件=原生应用，只是系统装好了；
- 迁移软件 = 用自有 API 重写，**禁止兼容层**（不翻译、不模拟第三方调用）。

## 1. 原生应用开发（编译型语言，Rust）

原生应用**不支持瞬语言**，仅编译型语言；需要运行时依赖必须**随包打包**。

### 1.1 SDK（宿主侧工具链）

```
sdk/            # lyx-sdk：no_std Rust 原生应用 SDK
  Cargo.toml    # 链接进 .lyx 镜像的 crate
  linker-lyx.ld # LYSOR 镜像链接脚本
  src/lib.rs    # 应用入口宏 + 系统调用包装（自有 API）
examples/lyx_hello/   # 最小原生应用示例（ring3 自检同源）
tools/lyx_build_app.ps1  # 一键构建：cargo → .lyx 镜像
tools/lyx_build.py       # 镜像后处理（布局/对齐/ELF .rela 重定位）
tools/sign_lyx.py        # 签名（服务器侧）
```

构建流程：
```
tools/lyx_build_app.ps1 <app-dir>      # 产出 embed/<app>.lyx（未签名镜像）
python tools/sign_lyx.py <app>.lyx <out>.lyx <priv> 0   # SM2 国密
python tools/sign_lyx.py <app>.lyx <out>.lyx <priv> 1   # Ed25519
```
- 签名算法：algo=0x00 SM2（GB/T 32918.2，SM3+ZA 默认 ID `1234567812345678`，
  raw r‖s 32+32 大端）；algo=0x01 Ed25519（raw 64B）。
- 私钥**只存签名服务器**（chmod 600）；内核只持公钥证书表
  （`sig::ly_cert_add`，8 槽，SM2 轨 + Ed25519 轨各若干）。
- 装载门：`exec::ly_exec_lyx` → 双轨真验签 → 失败拒绝（Bad 永不放行）。

**地址模型契约（ring3 高区，smoke-82 定案）**：
- 链接基址 **0** + 应用级 `relocation-model=pic`（`examples/<app>/.cargo/config.toml`）：
  代码对 rodata/data 的引用为 RIP-relative（运行时自动正确）。
- 数据段指针由 `lyx_build.py` 按 ELF `.rela` 重定位加 `USER_BASE+LYX_OFF(96)`
  （RELATIVE=r_addend / R_X86_64_64=sym+addend；text 段从 file_offset 96 起）。
- **不能链接到 USER_BASE**：static 模型下 rodata 绝对地址是裸偏移（ring3 #PF
  CR2=0x101B 实证），且高基址触发 rust-lld `R_X86_64_32S out of range`。
- **cargo clean 必须**：linker.ld / .cargo config 变化不触发重链（`Finished in
  0.06s` 假成功，embed 仍是旧镜像——字节实读核验）。
- 内核 syscall 指针校验 `user_ptr` 放行 `USER_BASE..USER_STACK_TOP`（lyx 应用
  运行在 0x400000000000 高区，非 legacy 低内存；见 §1.2）。
- 镜像新旧判别：PIC 版 0x130 处 `lea rsi,[rip+...]`（RIP-relative）；旧 static
  缺陷版含裸偏移 imm32=0x101B。

**预装应用示范（玩机技巧）**：`examples/lyx_tips/` 是独立预装的原生应用——
SDK 仅用 `ly_open/ly_read/ly_write/ly_close/ly_exit`，运行时读 `/etc/apps.conf`
已装清单 + 写 `/tmp/tips.txt`，退出码 `0x54`（84，'T'=Tips）供冒烟断言。
预装链：构建 → Ed25519 签名 → `include_bytes!(embed/lyx_tips_selftest.lyx)`
进内核镜像 → 开机 `lyx_tips_selftest()` 装载执行。冒烟断言（双锚点）
`selftest PROCESS started tid=10` → `exit(0x2A)`（ring3 自检同源 hello）+
`play-tips PROCESS started tid=11` → `exit(0x54)`。**新预装应用照此三步接**
（源码 / 签名镜像 / `src/exec/mod.rs` 装载入口），验证通过后进冒烟基线断言表。

### 1.2 原生应用系统调用（自有 API）

见 `sdk/src/lib.rs`（syscall 包装：exit / serial / vfs 读写…）。
- 调用约定：`int 0x80`，rax=nr，rdi/rsi/rdx=参数；`ly_exit(code)` 必须
  `sc3(5, code, 0, 0)` 显式传退出码（sc0(5) 会丢参 → 内核日志 `exit(0x0)`）。
- 指针参数校验：内核 `user_ptr` 放行 `p < 0x4000_0000`（legacy 低内存身份映射）
  或 `USER_BASE..USER_STACK_TOP`（lyx 应用高区，含镜像与私有用户栈）。
新增系统调用 = 内核 `syscall` 入口 + SDK 包装 + 文档，三步同步。

### 1.3 打包为瞬应用（编译产物入口）

编译型语言进瞬应用：把 `main.lyx` 作为包内资源，manifest `entry = "bin:main.lyx"`。
- 装载：`lyap::lyap_launch_bin` → 读 `/sandbox/<app>/main.lyx` → `exec::ly_exec_lyx`
  （与原生应用同一装载器、同一双轨验签）；
- 分发：ShanRun 启动时 `lyap::lyap_launch` 自动按 `main.lyx` 优先 → 否则 `main.sl`；
- 预编译：瞬应用允许预编译产物提速（同 bin: 路径）。

## 2. 瞬应用开发（瞬语言 / 可嵌 Rust）

瞬应用=小程序语义：**点开即用、无需安装、最多预编译**。

### 2.1 瞬语言（`docs/SHAN_LANG.md`）

- 类 Python 子集，**括号可选**（`if a < 2 {`、`print "hi", i;`）；
- **原生图形化**：`cls/px/rect/line/text` 直写画布（256×192），窗口 blit 上屏，
  无第三方注解库；`print` 输出保留在输出区；
- 安全上限：步数预算、调用深度 15、token 512、字符串池 1024B、源 ≤8KB。

### 2.2 瞬应用内嵌 Rust（编译型）

- 把 Rust 编译的 `.lyx` 作为包资源，`entry = "bin:main.lyx"`（见 §1.3）；
- 运行时依赖必须打包进包（sdk 静态链接，无外部 DLL 概念）。

### 2.3 JS/TS（解释型，规划）

- 需要对应解释器才能进瞬应用（瞬语言解释器已有；JS 解释器选型见
  `docs/BROWSER_PLAN.md` §4，MIT/Apache-2.0 轻量引擎，集成后 JS/TS 可编译
  为字节码预编译进包）。

## 3. IDE（Shan IDE，可登录）

内置 Shan IDE（控制面板第 10 格），复用现有 IDE 加功能，四个页签（F10 循环）：

| 页 | 功能 |
|----|------|
| 0 编辑 | 编辑 `main.sl`（瞬语言源码）；Tab 语法检查；Save 落盘 |
| 1 运行输出 | 画布（draw 图形）+ print 文本输出 |
| 2 验签参数 | App id（dev id）+ algo（左/右切换 SM2/Ed25519） |
| 3 原生工具链 | 原生应用（.lyx）构建/签名/安装指引；dev id + algo 同页切换 |

- 打包门：`Pack` 需 `auth::auth_is_developer()`（userinfo.level>=100）；
- 原生应用：`tools/lyx_build_app.ps1`（宿主构建）+ 服务器签名 → 安装/打包。

## 4. 验签体系（`docs/SIGNING_GUIDE.md`）

- 双轨真验签：SM2 国密优先 / Ed25519 备用；私钥仅服务器；
- 安装三态：Valid（签名通过）/ Unsigned（algo=0xFF，仅开发者会话放行）/ Bad（永拒）；
- KC/.lykc 无 `allow_unsigned`；
- 工具：`sign_lyx.py`（原生/.lykc）、`sign_lyap.py`（瞬应用包）、`ed25519.py`（纯 python 签名链）。

## 5. 应用商店与软件库（配置指南）

### 5.1 应用商店（系统预装软件）

应用商店是**预装原生应用**（非内核）：列表、搜索、安装入口。
安装来源 = 软件库下载地址（.lyap/.lyx 签名包）→ 验签 → 安装 → 登记 `/etc/apps.conf`
（行格式 `name|version|path|signed(0/1)`）。

### 5.2 软件库配置（用户提供下载地址）

软件库默认配置在 `docs/APP_STORE.md`（尚无该文件时以本节约定为准），
配置点仅一处：

```
tools/app_store.conf（或 /etc/apps.conf 同源）
格式：
[store]
base_url = <用户提供的下载地址，替换此行>
```

- 用户给出下载地址后，把它替换到上面 `base_url`（同时写入 `docs/APP_STORE.md` 说明）；
- 商店拉取目录页 `index.json`（字段：name/version/entry/bin|shan/url/algo/dev_id）→
  校验 → 下载 `.lyap`/`.lyx` → 验签 → 安装；
- **未提供地址前商店显示"软件库未配置"**，不编默认值。

## 6. 公开仓策略（`docs/PUBLIC_SYNC.md`）

- 公开仓只放**最小可信集**，不是全部内核代码；
- 隐私代码全部插桩（公开版代码与私有版一致，插桩后不跑通是预期）；
- 密钥/私钥/种子/内部地址（`*.pem/*.key/d.hex/*priv*/*.seed`）**禁止入库**（.gitignore 已列红线）。

## 7. 文档同步规则

- 白皮书（`docs/WHITEPAPER.md`）与实现**零出入**：任何架构/API 变更必须同步
  白皮书 + 本指南 + 对应规范（SHAN_LANG / LYAP_MANIFEST / SIGNING_GUIDE）；
- 新增系统调用/内建/格式字段 = 三处同步（代码、规范、指南）+ 自检；
- 验证：`cargo build --release` 0 warning → QEMU 冒烟（自检序列见各文档）。

## 8. 常用命令速查

```
cargo build --release                          # 构建（0 warning 门禁）
F:\qemu\qemu-system-x86_64.exe -kernel target\x86_64-unknown-none\release\lysor-os ^
  -m 256 -smp 4 -no-reboot -drive file=tools\disk.img,format=raw,if=ide,index=0 ^
  -device e1000,netdev=n0 -netdev user,id=n0 -display none ^
  -serial file:logs\smoke.log -monitor none    # 冒烟（串口日志）
python tools/sign_lyx.py in.lyx out.lyx priv.pem 0   # SM2 签名
python tools/sign_lyap.py in.lyap out.lyap priv.pem 1 # Ed25519 签名
python tools/shan_pack.py main.sl out.lyap appname    # 本机打包未签名瞬应用
```
