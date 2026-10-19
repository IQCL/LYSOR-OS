# LYSOR OS 应用开发指南（APP_DEV_GUIDE）

本指南面向应用开发者：如何在 LYSOR OS 上编写、打包、运行**原生应用（.lyx）**与**瞬应用（.lyap）**。

## 1. 环境搭建

```bash
# Rust（x86_64-unknown-none 目标）
rustup target add x86_64-unknown-none

# QEMU（运行/调试）
# 获取 SDK（含 lybuild / lyap-pack 打包工具）
```

## 2. 两类应用

| | 原生应用 | 瞬应用 |
|---|---------|--------|
| 格式 | `.lyx`（原生二进制） | `.lyap`（容器包） |
| 入口 | 编译后的原生代码 | `.lyx` 入口 + XMLUI 布局 + 逻辑脚本 |
| 生命周期 | 安装到 `/bin`，持久 | 装载即运行，关闭即回收，不落盘 |
| 界面 | WM/GFX API 手工绘制 | XMLUI 声明式布局（可选） |
| 规范 | 见 §3 | 见 [LYAP_SPEC.md](LYAP_SPEC.md) |

## 3. 原生应用：LYX 格式

LYX（"LYX1"）是 LYSOR OS 原生可执行格式（类似 .exe / ELF）：

```
[ Header 64B ][ .text ][ .rodata ][ .data ][ algo 1B ][ sig 64B ]
```

- magic = `0x4C595831`（"LYX1"），头结构 `LyxHeader` 64 字节（字段见公开源码 `src/exec/mod.rs`）
- **签名在文件尾**：1 字节算法标识（0 = SM2 国密轨，1 = Ed25519）+ 64 字节签名，被签内容 = 文件尾之前的全部字节
- 装载器强制验签：magic 非法、版本过新、text/data 超限（1MB）、算法未知或**验签失败一律拒绝装载**

### 3.1 第一个窗口程序

```rust
#![no_std]
#![no_main]

use lysor::wm::{self, LyInputEvent, LyColor};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 创建 400x300 窗口
    let win = wm::ly_wm_create("Hello", 100, 100, 400, 300).unwrap();

    // 客户区绘制（走 GFX 组件，禁止直接写帧缓冲）
    wm::ly_wm_rect(120, 150, 160, 80, LyColor::accent());
    wm::ly_wm_text_sz(150, 180, "Hello LYSOR!", 18, LyColor::text());

    // 事件循环
    loop {
        if let Some(ev) = wm::ly_wm_poll_input() {
            match ev {
                LyInputEvent::Key(code) => { /* 键盘 */ }
                LyInputEvent::MouseMove(x, y) => { /* 悬停 */ }
                LyInputEvent::MouseClick(x, y, btn) => { /* 点击 */ }
            }
            wm::ly_wm_dispatch(ev);
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { unsafe { core::arch::asm!("hlt"); } }
}
```

### 3.2 编译打包

```bash
# 用 SDK 的 lybuild 编译成 .lyx（自动生成头部并对接签名工具）
lybuild hello.rs -o hello.lyx

# 安装到系统 /bin 后即可运行
# 系统终端: run hello
```

签名要求：`.lyx` 尾部必须携带开发者有效签名（双轨：SM2 优先 / Ed25519）。签名工具与开发者证书由 SDK/发布流程管理。

## 4. 瞬应用：LYAP 包

瞬应用适合轻量工具类程序：声明式 XMLUI 布局 + 解释逻辑，关闭即回收。

```bash
# 1. 编写包目录（manifest.toml + layout/main.xml + logic/main.js）
# 2. 打包
lyap-pack myapp/ -o myapp.lyap
# 3. 系统终端装载
install myapp.lyap
```

完整包格式、权限位、验签语义见 [LYAP_SPEC.md](LYAP_SPEC.md)；XMLUI 标签见 [XMLUI_SPEC.md](XMLUI_SPEC.md)。

## 5. 系统服务 API 总览

应用可用的全部系统能力（文档均在本仓库 docs/）：

| 能力 | API | 文档 |
|------|-----|------|
| 文件读写 | INT 0x80 `open/read/write/close` | [SYSCALL_API.md](SYSCALL_API.md) |
| 进程间通信 | `ly_chan_*`（消息）/ `ly_shm_*`（共享内存白名单） | [IPC_API.md](IPC_API.md) |
| 绘图 | `ly_gfx_*`（唯一出口，16 项） | [GFX_API.md](GFX_API.md) |
| 窗口 | `ly_wm_create/move/focus/destroy/poll_input/…` | [WM_API.md](WM_API.md) |
| 让位调度 | INT 0x80 `sched_yield` | [SYSCALL_API.md](SYSCALL_API.md) |

## 6. 样式规范

- 强调色 `#2196F3`、背景 `#F5F5F5`、文字 `#212121`；圆角 8px、扁平无阴影
- 禁止纯黑方块按钮、默认系统灰
- 文字统一走 `ly_gfx_text_sz` / `ly_wm_text_sz`（组件侧矢量字体，全 UTF-8）

## 7. 调试

- 串口输出是主要调试手段：`serial::log(...)`（COM1，QEMU `-serial stdio`）
- 系统终端可执行 `ls / cat / ps` 等 coreutils 查看状态
- 内核构建/运行验证见 [TESTING.md](TESTING.md)
