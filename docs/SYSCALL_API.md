# LYSOR OS 系统调用 ABI（SYSCALL_API）

## 1. 调用约定

系统调用通过 **INT 0x80** trap gate 进入内核：

| 寄存器 | 用途 |
|--------|------|
| `rax` | 系统调用号（入口）→ 返回值（出口，内核回写） |
| `rdi` | 参数 0 |
| `rsi` | 参数 1 |
| `rdx` | 参数 2 |

返回值：`isize`。成功 ≥ 0；失败为负的错误码（如 `-ENOSYS = -38`）。

内核侧封装示例（与 `src/main.rs` 自测代码一致）：

```rust
#[inline(never)]
unsafe fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    core::arch::asm!(
        "int 0x80",
        inout("rax") nr => ret,
        in("rdi") a0,
        in("rsi") a1,
        in("rdx") a2,
        options(nostack)
    );
    ret
}
```

## 2. 系统调用号

| nr | 名称 | 参数 (rdi, rsi, rdx) | 返回 |
|----|------|----------------------|------|
| 1 | `open` | path_ptr, flags, mode | fd（≥0）或负错误码 |
| 2 | `read` | fd, buf_ptr, count | 读取字节数 |
| 3 | `write` | fd, buf_ptr, count | 写入字节数 |
| 4 | `close` | fd, -, - | 0 |
| 5 | `exit` | code, -, - | 不返回 |
| 10 | `ipc_send` | chan, msg_ptr, - | 0 |
| 11 | `ipc_recv` | chan, msg_ptr, - | 收到字节数（无消息 0） |
| 20 | `sched_yield` | -, -, - | 0 |

未知调用号返回 `-38`（ENOSYS）。

## 3. 文件操作

`open` 的 flags 与 VFS 层常量一致（定义在 `src/vfs/`）：

| flag | 值 | 含义 |
|------|----|------|
| `O_CREAT` | 见 vfs 模块 | 不存在则创建 |
| `O_RDWR` | 见 vfs 模块 | 读写 |
| `O_DIRECTORY` | 见 vfs 模块 | 打开目录 |

示例（内核态自测真实代码，用户态等价 ABI 相同）：

```rust
let path = b"/tmp/sc_test\0";
let fd = syscall3(SYS_OPEN, path.as_ptr() as usize, O_CREAT as usize, 0);
let data = b"hello lysor";
let w = syscall3(SYS_WRITE, fd as usize, data.as_ptr() as usize, data.len());
let _ = syscall3(SYS_CLOSE, fd as usize, 0, 0);
```

标准路径约定：

| 路径 | 用途 |
|------|------|
| `/etc` | 系统配置 |
| `/home/user` | 用户文件 |
| `/bin` | 原生应用（.lyx） |
| `/tmp` | 临时文件 |
| `/registry` | 系统注册表 |
| `/var/log` | 日志 |

## 4. IPC 调用

`ipc_send` / `ipc_recv` 操作 `LyMsg` 结构（布局见 [IPC_API.md](IPC_API.md)）：

```rust
let ch = ly_chan_open("demo", 0).unwrap();
let mut m = LyMsg::empty();
m.dst_tid = 0;              // 0 = 任意接收者
m.len = 5;
m.data[..5].copy_from_slice(b"hello");
syscall3(SYS_IPC_SEND, ch, &m as *const LyMsg as usize, 0);

let mut out = LyMsg::empty();
let n = syscall3(SYS_IPC_RECV, ch, &mut out as *mut LyMsg as usize, 0);
```

## 5. 调度让位

```rust
syscall3(SYS_SCHED_YIELD, 0, 0, 0); // 主动让出 CPU，返回 0
```

## 6. 错误码

| 错误 | 值 | 含义 |
|------|----|------|
| ENOSYS | -38 | 未知系统调用号 |
| 其他负值 | <0 | 具体操作失败（fd 无效、权限拒绝、白名单拒绝等） |
