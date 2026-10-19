# LYSOR OS IPC API（IPC_API）

LYSOR OS 提供**双通道 IPC**：Channel A（消息端口，拷贝式）与 Channel B（共享内存，白名单授权）。

## 1. Channel A — 消息端口

有界环形队列，固定大小消息，发送方拷入、接收方拷出。

### 消息结构 `LyMsg`（`repr(C)`，与源码一致）

```rust
pub struct LyMsg {
    pub src_tid: u32,          // 发送者线程 id
    pub dst_tid: u32,          // 目标线程 id（0 = 任意）
    pub len: u32,              // 有效载荷长度（≤ MSG_SIZE）
    pub data: [u8; 128],       // MSG_SIZE = 128
}
```

### API

```rust
/// 打开命名消息通道，返回通道句柄。
/// owner_tid = 0 表示任意调用者可收发（开放通道）。
pub fn ly_chan_open(name: &str, owner_tid: u32) -> Result<usize, &'static str>;

/// 发送消息（拷贝入队；队列满返回错误）。
pub fn ly_chan_send(chan: usize, msg: &LyMsg) -> Result<(), &'static str>;

/// 接收消息。无消息时返回 Ok(None)。
pub fn ly_chan_recv(chan: usize, caller_tid: u32) -> Result<Option<LyMsg>, &'static str>;

/// 关闭通道。
pub fn ly_chan_close(chan: usize) -> Result<(), &'static str>;
```

### 收发示例

```rust
use crate::ipc::channel_a::{ly_chan_open, ly_chan_send, ly_chan_recv, ly_chan_close, LyMsg};

let ch = ly_chan_open("demo", 0).unwrap();

// 发送
let mut m = LyMsg::empty();
m.src_tid = 1;
m.dst_tid = 0;
m.len = 5;
m.data[..5].copy_from_slice(b"hello");
ly_chan_send(ch, &m).unwrap();

// 接收
if let Ok(Some(msg)) = ly_chan_recv(ch, 1) {
    // msg.data[..msg.len as usize] 即载荷
}
ly_chan_close(ch).unwrap();
```

也可通过 syscall 走同样语义：`SYS_IPC_SEND`(10) / `SYS_IPC_RECV`(11)，见 [SYSCALL_API.md](SYSCALL_API.md)。

## 2. Channel B — 共享内存（白名单）

零拷贝共享内存。每块共享区有**访问白名单**：不在白名单中的调用者被显式拒绝（`"whitelist denied"`），不会静默放行。

```rust
/// 创建命名共享内存区，owner 为初始白名单成员。
pub fn ly_shm_create(name: &str, owner_tid: u32) -> Result<usize, &'static str>;

/// 写入 offset 起的 len 字节。caller 必须在白名单内。
pub fn ly_shm_write(shm: usize, offset: usize, data: &[u8],
                    caller_tid: u32, flag: u32) -> Result<(), &'static str>;

/// 读取 offset 起的 len 字节。caller 必须在白名单内。
pub fn ly_shm_read(shm: usize, offset: usize, buf: &mut [u8],
                   caller_tid: u32, flag: u32) -> Result<(), &'static str>;
```

### 白名单行为

```rust
let shm = ly_shm_create("trusted", 1).unwrap();
ly_shm_write(shm, 0, b"ipc-b", 1, 0).unwrap();          // tid 1 在白名单 → OK
ly_shm_read(shm, 0, &mut buf, 1, 0).unwrap();           // OK

// tid 99 不在白名单 → Err("whitelist denied")
assert!(ly_shm_write(shm, 0, b"x", 99, 0).is_err());
```

## 3. 使用约定

- 通道/共享区数量有上限（内核 TCB 表），用完即 `close` 归还
- `dst_tid = 0` 表示广播给任意接收者；指定 tid 则定向投递
- 载荷超过 128 字节请拆包或改用 Channel B 共享内存
- 白名单拒绝是**硬错误**：依赖它的调用方必须处理 `Err`
