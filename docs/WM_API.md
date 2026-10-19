# LYSOR OS 窗口管理 API（WM_API）

`ly_wm` 提供窗口生命周期、输入事件分发与窗口内绘图原语。绘图全部转发 GFX API（见 [GFX_API.md](GFX_API.md)），应用不直接碰帧缓冲。

## 1. 窗口生命周期

```rust
/// 创建窗口，返回窗口句柄 idx。
pub fn ly_wm_create(title: &str, x: i32, y: i32, w: i32, h: i32)
    -> Result<usize, &'static str>;

/// 移动窗口。
pub fn ly_wm_move(idx: usize, x: i32, y: i32) -> Result<(), &'static str>;

/// 聚焦窗口（提升 z 序）。
pub fn ly_wm_focus(idx: usize) -> Result<(), &'static str>;

/// 最大化窗口。
pub fn ly_wm_maximize(idx: usize) -> Result<(), &'static str>;

/// 销毁窗口。
pub fn ly_wm_destroy(idx: usize) -> Result<(), &'static str>;

/// 查询窗口矩形。
pub fn ly_wm_window_rect(idx: usize) -> LyRect;

/// 注册窗口输入回调（None = 注销）。
pub fn ly_wm_set_handler(idx: usize, handler: Option<fn(LyInputEvent)>)
    -> Result<(), &'static str>;
```

`LyRect`：

```rust
pub struct LyRect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }
```

## 2. 输入事件

```rust
pub enum LyInputEvent {
    Key(u8),                    // 键盘扫描码
    MouseMove(i32, i32),        // 鼠标位置（帧缓冲坐标）
    MouseClick(i32, i32, u8),   // 点击位置 + 按键
}

/// 轮询输入事件（无事件返回 None）。
pub fn ly_wm_poll_input() -> Option<LyInputEvent>;

/// 分发事件到焦点窗口（配合 poll_input 或由系统事件泵调用）。
pub fn ly_wm_dispatch(ev: LyInputEvent);

/// 命中测试：屏幕坐标 → 窗口 idx。
pub fn ly_wm_hit_test(x: i32, y: i32) -> Option<usize>;
```

### 事件循环（典型应用骨架）

```rust
let win = ly_wm_create("MyApp", 100, 100, 400, 300)?;

loop {
    if let Some(ev) = ly_wm_poll_input() {
        match ev {
            LyInputEvent::Key(code) => { /* 键盘处理 */ }
            LyInputEvent::MouseMove(x, y) => { /* 悬停处理 */ }
            LyInputEvent::MouseClick(x, y, btn) => { /* 点击处理 */ }
        }
        ly_wm_dispatch(ev);
    }
}
```

## 3. 窗口绘图

窗口区域内的绘图原语（自动限制在窗口自身区域，绘制走 GFX 组件）：

```rust
pub fn ly_wm_rect(x: i32, y: i32, w: i32, h: i32, c: LyColor);
pub fn ly_wm_rrect(x: i32, y: i32, w: i32, h: i32, r: i32, c: LyColor);
pub fn ly_wm_round_rect(x: i32, y: i32, w: i32, h: i32, c: LyColor);   // 8px 圆角
pub fn ly_wm_hline(x: i32, y: i32, w: i32, c: LyColor);
pub fn ly_wm_vline(x: i32, y: i32, h: i32, c: LyColor);
pub fn ly_wm_line(x0: i32, y0: i32, x1: i32, y1: i32, c: LyColor);
pub fn ly_wm_ring(cx: i32, cy: i32, ro: i32, ri: i32, c: LyColor);
pub fn ly_wm_tri(x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32, c: LyColor);
pub fn ly_wm_rect_alpha(x: i32, y: i32, w: i32, h: i32, c: LyColor, a: u32);
pub fn ly_wm_blit_rgba(src: *const u8, x: i32, y: i32, w: i32, h: i32, sw: usize, sh: usize);
pub fn ly_wm_blit_rgb(data: *const u8, w: i32, h: i32);
pub fn ly_wm_text(x: i32, y: i32, s: &str, c: LyColor);
pub fn ly_wm_text_sz(x: i32, y: i32, s: &str, size: u16, c: LyColor);
```

## 4. 窗口装饰

系统提供 Win11 风格标题栏/边框绘制与按钮命中测试（最小化/最大化/关闭）：

```rust
pub fn ly_wm_win11_frame(x: i32, y: i32, w: i32, h: i32, title: &str, focused: bool);
pub fn ly_wm_win11_btn_at(x: i32, y: i32, r: LyRect) -> i32;  // 返回按钮 id
pub fn ly_wm_win11_hover(b: i32);
```

应用一般不直接调用装饰 API；创建窗口后由系统统一绘制标题栏，应用只画客户区。

## 5. 动画与分辨率

```rust
pub fn ly_wm_anim_tick();                       // 窗口/任务栏动画步进
pub fn ly_wm_set_res(w: u16, h: u16) -> Result<(), &'static str>; // 分辨率切换
pub fn ly_wm_cursor() -> (i32, i32);            // 光标位置查询
```

## 6. 样式规范

- 强调色 `#2196F3`（Material Blue），背景 `#F5F5F5`（surface），文字 `#212121`
- 圆角 8px、扁平无阴影
- 禁止纯黑方块按钮、默认系统灰
