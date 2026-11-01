# LYSOR OS 图形 API（GFX_API）

GFX 是应用的**唯一绘图出口**。应用（含桌面、窗口、系统软件）禁止直接写帧缓冲，所有绘制经 GFX API 转发至图形组件 `lycomp_gfx` 完成。

## 1. 调用链

```
应用 → ly_gfx_*(转发) → ly_kcomp_invoke(slot, idx, regs)
     → lycomp_gfx（组件侧私有栈）→ 栅格化/绘制 → 帧缓冲
```

本公开版不含 `lycomp_gfx` 实现（属私有完整版），但 **API 契约与调用索引保持稳定**，面向组件开发者公开。

## 2. 颜色类型

```rust
pub struct LyColor { /* RGBA */ }

impl LyColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self;
    pub const fn surface() -> Self;   // #F5F5F5
    pub const fn accent() -> Self;    // #2196F3
    pub const fn text() -> Self;      // #212121
}
```

## 3. 调用索引（ly_kcomp_invoke fn_idx）

组件 ABI 层的 16 项功能索引（`regs[0]` 起为参数）：

| idx | 名称 | 签名 |
|-----|------|------|
| 0 | CLEAR | `clear(c)` |
| 1 | RECT | `rect(x, y, w, h, c)` |
| 2 | RRECT | `rrect(x, y, w, h, rad, c)` |
| 3 | HLINE | `hline(x, y, w, c)` |
| 4 | VLINE | `vline(x, y, h, c)` |
| 5 | LINE | `line(x0, y0, x1, y1, c)` |
| 6 | RING | `ring(cx, cy, r_outer, r_inner, c)` |
| 7 | TRI | `tri(x0, y0, x1, y1, x2, y2, c)` |
| 8 | RECT_ALPHA | `rect_alpha(x, y, w, h, c, alpha)` |
| 9 | BLIT_RGBA | `blit_rgba(src, x, y, w, h, src_w, src_h)` |
| 10 | BLIT_RGB | `blit_rgb(src, w, h)` |
| 11 | TEXT | `text(x, y, s, c)` |
| 12 | TEXT_SZ | `text_sz(x, y, s, size, c)` |
| 13 | WALLPAPER | `wallpaper(theme)` |
| 14 | FB_GEOM | `fb_geom() -> (w, h)` |
| 15 | SET_PIXEL | `set_pixel(x, y, c)` |

## 4. 应用层函数签名

应用代码使用的稳定封装（与索引一一对应）：

```rust
pub fn ly_gfx_clear(c: LyColor);
pub fn ly_gfx_rect(x: i32, y: i32, w: i32, h: i32, c: LyColor);
pub fn ly_gfx_rrect(x: i32, y: i32, w: i32, h: i32, rad: i32, c: LyColor);
pub fn ly_gfx_round_rect(x: i32, y: i32, w: i32, h: i32, c: LyColor); // 默认 8px 圆角
pub fn ly_gfx_hline(x: i32, y: i32, w: i32, c: LyColor);
pub fn ly_gfx_vline(x: i32, y: i32, h: i32, c: LyColor);
pub fn ly_gfx_line(x0: i32, y0: i32, x1: i32, y1: i32, c: LyColor);
pub fn ly_gfx_ring(cx: i32, cy: i32, ro: i32, ri: i32, c: LyColor);
pub fn ly_gfx_tri(x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32, c: LyColor);
pub fn ly_gfx_rect_alpha(x: i32, y: i32, w: i32, h: i32, c: LyColor, a: u32);
pub fn ly_gfx_blit_rgba(src: *const u8, x: i32, y: i32, w: i32, h: i32, sw: usize, sh: usize);
pub fn ly_gfx_blit_rgb(src: *const u8, w: i32, h: i32);
pub fn ly_gfx_text(x: i32, y: i32, s: &str, c: LyColor);
pub fn ly_gfx_text_sz(x: i32, y: i32, s: &str, size: u16, c: LyColor);
pub fn ly_gfx_wallpaper(theme: u8);
pub fn ly_gfx_fb_geom() -> (u32, u32);
pub fn ly_gfx_set_pixel(x: i32, y: i32, c: LyColor);
```

## 5. 示例

```rust
use crate::gfx::{ly_gfx_clear, ly_gfx_rrect, ly_gfx_text_sz, LyColor};

ly_gfx_clear(LyColor::surface());
ly_gfx_rrect(20, 20, 200, 80, 8, LyColor::accent());
ly_gfx_text_sz(32, 44, "Hello LYSOR", 20, LyColor::text());
```

## 6. 约定

- 文字渲染由组件侧矢量字体引擎完成，支持全 UTF-8；应用只给 `&str`
- 坐标超出帧缓冲的部分由组件侧裁剪
- 应用侧代码不感知帧缓冲地址/分辨率切换细节；分辨率用 `fb_geom()` 查询
