# LYSOR 瞬应用包格式规范（LYAP_SPEC）

`.lyap` 是 LYSOR OS 瞬应用（瞬时应用）的原生分发格式：一个自带 `manifest` 头的容器，描述应用元数据、权限、资源与签名。

## 1. 文件布局

```
[256 字节 manifest 头][resource 0][resource 1]...[resource N]
```

资源按 manifest 中 `resource_count` 依次排列，运行时装载到沙盒目录 `/sandbox/<app_name>/`，各应用相互隔离，不跨重启持久。

## 2. Manifest 头（`LyapManifest`，256 字节，packed，`repr(C)`）

| 偏移 | 大小 | 字段 | 说明 |
|------|------|------|------|
| 0x00 | 8 | magic | 必须为 `0x4B47505041594C`（"LYAPPKG"） |
| 0x08 | 64 | app_name | NUL 结尾应用名 |
| 0x48 | 16 | version | NUL 结尾版本串（如 "1.0.0"） |
| 0x58 | 4 | perms | 权限位掩码（见下） |
| 0x5C | 64 | entry | 入口可执行文件路径（如 `/bin/main.lyx`） |
| 0x9C | 4 | resource_count | manifest 后跟随的资源数量 |
| 0xA0 | 32 | sig | Ed25519 签名 |
| 0xC0 | 32 | pubkey | 开发者公钥 |
| 0xE0 | 32 | （保留） | 零 |

manifest 新增声明字段（写入资源区/manifest 附属数据）：

```
layout_path = "/layout/main.xml"   # XMLUI 布局文件路径
script_path = "/logic/main.js"     # 解释逻辑文件路径
```

## 3. 权限位

| Bit | 名称 | 说明 |
|-----|------|------|
| 0 | PERM_NET | 网络访问 |
| 1 | PERM_STORAGE | 文件系统写入 |
| 2 | PERM_AUDIO | 音频播放/采集 |
| 3 | PERM_CAMERA | 摄像头访问 |

瞬应用**默认零权限**：未声明的位一律拒绝。权限在装载时校验，运行期不可提升。

## 4. 签名

- `sig` 为 Ed25519 签名，覆盖 manifest + 全部资源块
- `pubkey` 标识开发者身份，与系统信任的证书表比对
- **无签名或验签失败的应用包拒绝装载**，不会静默放行

验签链路复用内核 `sig` 模块（见公开源码 `src/sig/`）：算法标识与证书表语义同 `.lyx` 原生应用（ALGO_ED25519 / ALGO_SM2 双轨，国密优先）。

## 5. 装载 API（系统侧）

```rust
use lysor::lyap;

/// 解析包镜像，返回 manifest（含 app_name / perms / entry）。
let manifest = lyap::lyap_parse(package_image, package_len)?;
```

装载流程：`parse → 验签 → 权限校验 → 沙盒提取 → 启动 entry`。

## 6. 打包

使用 SDK 的 `lyap-pack` 工具打包：

```bash
lyap-pack myapp/ -o myapp.lyap
```

包目录约定：

```
myapp/
├── manifest.toml        # 元数据（生成 manifest 头）
├── bin/main.lyx         # 入口原生应用（可选）
├── layout/main.xml      # XMLUI 布局（可选）
└── logic/main.js        # 解释逻辑（可选）
```

运行：系统终端 `install myapp.lyap` 装载，瞬应用关闭即回收，不落盘持久。
