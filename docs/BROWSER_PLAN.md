# LYSOR OS 浏览器 / WebView 选型与实现方案 v1.0

> 目标：本机 webview + 类 Chrome/Edge 浏览器应用。**轻量优先**（磁盘紧张，
> 禁拉几百 GB 内核）；**协议筛选**（不因使用开源项目导致整个系统强制开源）；
> **全部 API 替换为自有**（无兼容层）。
>
> 配套：`system.md`（本机 OAuth2 8087 接口约定）、`docs/DEVELOPER_GUIDE.md`、
> `docs/reference/oauth2.md`。

## 1. 约束

1. **磁盘**：浏览器内核候选必须 < 200 MB 源码/构建产物级别；优先纯自研子集。
2. **协议**：只允许 MIT / Apache-2.0 / BSD-2/3-Clause / Zlib / ISC；
   **禁止** GPL / LGPL / AGPL / MPL 等传染或强 Copyleft（用了=整个系统强制开源，否决）。
3. **自有 API**：渲染、布局、HTTP、导航全部走 LYSOR 图形栈与 net 栈，不借第三方兼容层。

## 2. 分层架构（全部自有）

```
┌─────────────────────────────────────────────────┐
│ Browser 应用（预装原生 .lyx，类 Chrome/Edge UI） │
│  标签页 · 地址栏 · 前进/后退 · 书签 · 下载      │
├─────────────────────────────────────────────────┤
│ WebView 核心（内核 /app 级）                    │
│  HTML 标签树解析 · CSS 子集布局 · 文本/图片渲染 │
│  （渲染对接 wm/gfx + font，自有实现）           │
├─────────────────────────────────────────────────┤
│ JS 引擎（轻量，协议筛选后嵌入）                 │
│  首选纯 Rust：boa_engine(MIT+Apache-2.0)       │
│  备选：MiniJS(MIT) / deno_core(MIT)             │
├─────────────────────────────────────────────────┤
│ HTTP/1.1 客户端（net 栈 TCP 之上自研，TLS 可选）│
│ OAuth2 本机代理（127.0.0.1:8087，system.md）    │
└─────────────────────────────────────────────────┘
```

## 3. 渲染栈选型（自研为主）

| 能力 | 方案 | 理由 |
|------|------|------|
| HTML 解析 | 自研标签树（DOM 子集） | 无依赖；只有块/行/文本/图片/链接等核心标签 |
| CSS 布局 | 自研盒模型子集 | block/inline/flex 基础；避第三方 CSS 引擎 |
| 文本渲染 | 现有 `font::font_render` + `gfx::ly_gfx_text_sz` | 已内建 TrueType 光栅化 |
| 图片 | 本机 /sandbox 资源 + R2 缓存下载 | 复用 vfs + gfx blit |
| 绘制 | `wm::ly_wm_*` / `gfx::ly_gfx_*` | 自有图形栈，脏矩形重绘 |

不选整机浏览器内核（Chromium/WebKit/Gecko）：体积数百 GB 级、构建链极重、
且部分组件 Copyleft（Chromium=BSD，可，但体积/复杂度远超本项目约束）。

## 4. JS 引擎协议筛选（关键决策）

瞬应用支持 JS/TS 的前置：需要一个轻量 JS 解释器。候选与协议审查：

| 引擎 | 协议 | 体积 | 结论 |
|------|------|------|------|
| `boa_engine`（Rust） | **MIT + Apache-2.0** | 纯 Rust，构建产物 ~10 MB | **首选**：无传染、与 Rust 生态一致、ES 2020+ 子集 |
| `deno_core`（Rust） | **MIT** | 核心解释器较小 | 备选：绑 V8 需 C++ 工具链，重 |
| `MiniJS` | **MIT** | 极小（~3k 行） | 备选：超轻，ES 子集有限 |
| V8 / JSC | BSD / LGPL 例外 | 百 MB 级 | **否决**（体积 + 构建链） |
| QuickJS | MIT | ~200k 行 C | 备选：需 FFI/C 集成，负但 MIT 无传染 |

**决策**：`boa_engine`（MIT+Apache-2.0，双许可最宽松）；若磁盘进一步紧张，
降级 `MiniJS`（MIT，极轻）。**在任何情况下不引入 GPL 系引擎。**

集成方式：boa 编译为 `.lyx` 侧运行时（随浏览器应用打包），JS 脚本经
自有 syscall（vfs/eval）桥接；**浏览器 JS 引擎与瞬应用 JS 支持共用同一引擎**。

## 5. HTTP/1.1 客户端（自有）

- net 栈已有 TCP（`src/net/`）；补 HTTP/1.1 解析（请求行/头/块体/分块）；
- **TLS 已实现**：M8 内核 TLS 1.2 客户端（ECDHE-RSA-AES128-GCM-SHA256）端到端闭环，
  冒烟 `tls: E2E OK bytes=175` 通过；浏览器 https:// 接线（替换 TLS_HTML 占位页、
  证书加载 `/etc/tls/`）为下一步；
- 缓存：KV/R2（Cloudflare 侧）下载资源，本机 vfs 缓存。

## 6. UI（类 Chrome/Edge）

- 复用 wm 窗口 + gfx + font + 任务栏/桌面同款 Win11 风格；
- 标签页栏（居中逻辑同任务栏）、地址栏、前进/后退、书签、下载页；
- 交互：地址栏 Enter 导航；Ctrl+T 新标签（后续）；右键菜单复用桌面实现。

## 7. OAuth2 接线（system.md 8087）

1. 地址栏访问 IQCL 登录页 → 浏览器识别 `iqcl.cn` 授权域 →
   请求本机 `127.0.0.1:8087/oauth2/device`；
2. 本地显示 user_code + verification_uri（www.iqcl.cn 输入 user_code）；
3. `GET /oauth2/poll` 轮询 → success → 会话注入 auth（`auth::auth_iqcl_start`
   已实现设备码/轮询/userinfo）；
4. 开发者判定生效（`userinfo.level >= 100` → `auth_is_developer()` → Shan IDE Pack 解锁）。

## 8. 里程碑

| M | 内容 | 验收 | 状态 |
|---|------|------|------|
| M1 | HTTP/1.1 客户端（解析+连接复用） | 能拉取本地/内网页面文本 | ✅ 已实现（`src/net/http.rs`：GET/POST、Content-Length+chunked、重定向、JSON 解析） |
| M2 | HTML 标签树 + 文本布局 + 链接 | 能打开本地 .ly 文档页 | ✅ 已实现（`src/apps/html.rs`：单遍流式布局，标签子集，实体解码，链接区域） |
| M3 | 浏览器窗口 UI（地址栏/标签页） | 标签页切换不闪屏（脏矩形） | ✅ 已实现（`src/apps/mod.rs` Browser：地址栏编辑/回车导航/点击链接/滚动/状态栏） |
| M4 | CSS 子集 + 图片 + 滚动 | 简单页面完整渲染 | 🔶 部分完成：inline style（color:#RRGGBB + text-align:center/right）+ `<style>body{color:#..}` 全局前景色 + `<img>`→[IMG] 占位 + 滚动已实现；真实图片解码待后续 |
| M5 | boa 引擎接入（JS 执行） | 页面脚本运行 + 瞬应用 JS 支持 | 🔶 骨架完成：`<script>` 内容收集 + `run_page_script` 用瞬语言解释器（`shan::ly_shan_run_str`）执行；boa_engine（MIT+Apache-2.0）正式引擎待接入 |
| M6 | OAuth2 8087 接线 | 完成一次设备码授权，level 显示 | 🔶 部分：`browser_csp()` 按 scheme 白名单（ly://=111 / http,https://=001）+ CSP 默认策略已生效；8087 本机代理未实现 |
| M7 | TLS + 缓存 + 书签 | 正式可用 | 🔶 TLS 客户端已实现（见 M8）；缓存/书签待规划（`browser_navigate` 对 https:// 已返回 TLS 占位页，待浏览器接线 TLS 客户端） |
| M8 | HTTPS/TLS 客户端 + 证书 | QEMU 内 https:// 页面可用 | ✅ **已实现（内核侧闭环）**：TLS 1.2（ECDHE-RSA-AES128-GCM-SHA256，RSA-SHA256 验签 → PRF 派钥 → GCM 记录 AEAD），冒烟验收 `tls: E2E OK bytes=175` 通过（服务器纯 python 权威实现对拍）。实现：`src/net/tls.rs` + `src/crypto/{rsa,sha256,aes}.rs`；X.509 证书链验签已含（RSA 验签）；浏览器 https:// 接线待下一步（TLS_HTML 占位页替换） |

## 9. 风险与红线

- **体积红线**：任何新依赖先查体积与协议；> 100 MB 或 Copyleft 一律否决；
- **协议红线**：合并前用 `cargo license` / 手动核对 LICENSE，全仓只有
  MIT/Apache-2.0/BSD/Zlib/ISC 允许入库；
- 自研优先：能自研的（解析/布局/HTTP）不自研才允许引入 MIT 级组件。
