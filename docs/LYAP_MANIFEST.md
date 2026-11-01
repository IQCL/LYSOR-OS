# LYSOR App Package (.lyap) Manifest Specification

## 1. Overview

`.lyap` is the native application package format for LYSOR OS.
It is a container with a root 256-byte manifest header describing the
application, its permissions, resources, and signature.

## 2. File layout

```
[256-byte manifest header][resource 0][resource 1]...[resource N]
```

The manifest header is `LyapManifest` (256 bytes, packed, repr(C)):

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0x00 | 8 | magic | Must be `0x4B47505041594C` ("LYAPPKG", little-endian u64) |
| 0x08 | 64 | app_name | Null-terminated app name |
| 0x48 | 16 | version | Null-terminated version string (e.g. "1.0.0") |
| 0x58 | 4 | perms | Permission bitmask (see below) |
| 0x5C | 64 | entry | Path to entry executable (e.g. "/bin/main.lyx") |
| 0x9C | 4 | resource_count | Number of resources following manifest |
| 0xA0 | 1 | algo | Signature algorithm id: 0 = SM2 (国密优先), 1 = Ed25519 |
| 0xA1 | 64 | sig | 64-byte raw signature over manifest[0..160] |
| 0xE1 | 31 | (reserved) | Zero |

**Signature layout changed** (old 32-byte `sig`/`pubkey` stub fields are
obsolete): the signed region is manifest bytes `[0..160)` (`LYAP_SIGNED_LEN`),
the algorithm byte sits at 160, and the 64-byte raw signature at 161..225.
Verification is real and dual-track via `sig::ly_verify_lyx` → crypto
kcomponent (SM2 GB/T 32918.2 / Ed25519 RFC 8032). Unsigned or badly-signed
packages are rejected at parse time, never silently loaded.

Signing procedure (external signing server): see `SIGNING_GUIDE.md`.

## 3. Permission bits

| Bit | Name | Description |
|-----|------|-------------|
| 0 | PERM_NET | Network access |
| 1 | PERM_STORAGE | Filesystem write |
| 2 | PERM_AUDIO | Audio playback/capture |
| 3 | PERM_CAMERA | Camera access |

Applications default to zero permissions; undeclared bits are denied at load
time and cannot be elevated at runtime.

## 4. Manifest extension fields

Declared in the package's `manifest.toml` (mapped into the header resources):

```
layout_path = "/layout/main.xml"   # XMLUI layout file
script_path = "/logic/main.js"     # interpreted logic file
```

## 5. Sandbox extraction

Resources are extracted to `/sandbox/<app_name>/`. Each app runs in its own
isolated directory. No persistence across reboots.

## 6. Loading API

```rust
use lysor::lyap;
let manifest = lyap::lyap_parse(package_image, package_len)?;
// Flow: parse → magic → dual-track signature verify → perms → sandbox → run entry.
```

## 7. 瞬应用脚本语义（v2，点开即用）

瞬应用包把 `entry` 字段写成 `shan:main.sl`（`shan:` 前缀 = 瞬语言脚本入口）：

- 装载时资源 `main.sl` 被提取到 `/sandbox/<app>/main.sl`；
- 点击应用（开始菜单 / Shan Run 运行器）→ `lyap::lyap_launch_script(app, "shan:main.sl", out)`
  → 内核内建瞬语言解释器（`src/shan/mod.rs`）**直接解释执行，无需安装、无需编译**；
- 语言规范见 `docs/SHAN_LANG.md`（类 Python 子集，纯解释，安全上限内置）；
- 签名规则与 v1 完全一致：manifest 前 160 字节双轨验签（SM2 国密优先 / Ed25519）；
  `algo=0xFF` 未签名仅限开发者会话；`Bad` 永不放行；
- Shan IDE（控制面板 → Shan IDE）提供编辑/语法检查/试运行/打包一体化：
  `lyap::lyap_pack_unsigned(app_name, script, out)` 生成未签名 `.lyap`
  （algo=0xFF），外部签名服务器用 `tools/sign_lyap.py` 签名后即可分发。

判定：`lyap::lyap_is_script_pkg(&manifest)` 返回 true 即瞬应用脚本包。
