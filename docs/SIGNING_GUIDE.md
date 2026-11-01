# LYSOR OS 软件签名操作指南（外部服务器版）

> **适用对象**：在你的外部签名服务器上完成 `.lyx` 原生应用与 `.lyap` 瞬应用的签名与密钥管理。
> **原则**：私钥**只存在于签名服务器**；内核、仓库、构建机永远只有公钥。
> **代码现状**：验签链路全部为真实现（无桩）——`exec` 装载 `.lyx`、`lyap` 装载瞬应用包都强制走 `sig::ly_verify_lyx` → `crypto::ly_verify_sig`（SM2 国密优先 / Ed25519 双轨），无签名或验签失败**拒绝装载**。

---

## 1. 签名对象与被签范围（签什么）

### 1.1 原生应用 `.lyx`

文件布局与被签范围：

```
┌──────────────────────────────────────────────┬────────┬─────────────┐
│ Header(64B) + .text + .rodata + .data        │ algo 1B│ sig 64B     │
└──────────────────────────────────────────────┴────────┴─────────────┘
 ←──────────── 被签内容 payload = 文件[0 .. len-65] ──────────────→
```

- **trailer 固定 65 字节**：`algo`（1 字节）+ `sig64`（64 字节），位于文件**末尾**
- 被签内容 = 文件去掉 trailer 后的**全部字节**（`image[0..len-65]`）
- 装载器（`src/exec/mod.rs`）按 `algo` 在 `len-65` 处读取算法标识，`len-64..len` 读取签名

### 1.2 瞬应用包 `.lyap`

manifest 头（`LyapManifest`，256 字节）内的签名区：

```
偏移 0x00 ── 0x9F (160 字节)：被签内容（LYAP_SIGNED_LEN）
偏移 0xA0 (160)      ：algo，1 字节算法标识
偏移 0xA1 ── 0xE0 (161..225)：sig64，64 字节签名
```

- **被签内容 = manifest 前 160 字节**（magic/app_name/version/perms/entry/resource_count 全部覆盖）
- 包内资源变更 → 重签整个 manifest；资源本身不单独签名，其完整性由 manifest.entry 与装载流程保证
- 解析与验签实现：`src/lyap/mod.rs`（`lyap_parse` → magic 校验 → `sig::ly_verify_lyx(msg, 160, algo, sig64)`）

### 1.3 算法标识字节

| algo | 算法 | 摘要 | 签名 | 优先级 |
|------|------|------|------|--------|
| `0x00` | SM2（GB/T 32918.2） | SM3 + ZA（默认 ID `1234567812345678`） | raw `r‖s`，各 32B 大端 | **国密优先** |
| `0x01` | Ed25519（RFC 8032） | SHA-512（内建） | raw 64B（`R‖S`） | 备用轨 |

未知算法标识 → 装载器直接拒绝。

---

## 2. 密钥生成（签名服务器上）

### 2.1 SM2（国密轨）

```bash
# 私钥（PEM，妥善离线保管）
openssl ecparam -genkey -name SM2 -out lysor_dev_sm2_priv.pem
chmod 600 lysor_dev_sm2_priv.pem

# 导出私钥原始标量 d（32 字节大端，HEX）——仅自存，绝不入内核/仓库
openssl ec -in lysor_dev_sm2_priv.pem -text -noout | awk '/priv:/{f=1} f&&/:/{getline; while($0!~/pub:/){gsub(/:/," ");printf "%s",$0; getline}} f&&/pub:/{exit}' | tr -d ' ' > d.hex

# 导出公钥（64 字节 x‖y 大端，HEX）——此值注册进内核证书表
openssl ec -in lysor_dev_sm2_priv.pem -pubout -outform DER \
  | tail -c 65 | tail -c 64 | xxd -p | tr -d '\n' > pub_xy.hex
```

- SM2 公钥 = 非压缩点 `04‖X‖Y` 去掉 `04` 前缀 = **64 字节 `x‖y`**
- 这 64 字节就是内核证书表的 `key64`（SM2 轨）

### 2.2 Ed25519（备用轨）

```bash
openssl genpkey -algorithm ed25519 -out lysor_dev_ed_priv.pem
chmod 600 lysor_dev_ed_priv.pem

# 公钥原始 32 字节（HEX）——注册进内核证书表 key64[..32]
openssl pkey -in lysor_dev_ed_priv.pem -pubout -outform DER \
  | tail -c 32 | xxd -p | tr -d '\n' > ed_pub.hex
```

---

## 3. 服务器端签名命令（签好怎么签）

### 3.1 Ed25519 轨（最简单，输出直接可用）

```bash
# 被签内容 payload.bin = .lyx 去尾 65 字节（或 .lyap manifest 前 160 字节）
openssl pkeyutl -sign -inkey lysor_dev_ed_priv.pem -rawin -in payload.bin -out sig64.bin
# sig64.bin = 64 字节 raw（R‖S），无需转换，直接嵌入 trailer
```

### 3.2 SM2 轨（国密优先，需 DER → raw 转换）

```bash
# -rawin 对 SM2 密钥执行 GB/T 32918.2 签名（SM3 摘要 + 默认 ID 1234567812345678，
# 与内核 sm2.rs 的 za() 完全一致）
openssl pkeyutl -sign -inkey lysor_dev_sm2_priv.pem -rawin -in payload.bin -out sig.der
```

OpenSSL 输出为 DER 编码 `ECDSA-Sig-Value { r INTEGER, s INTEGER }`，需转成 64 字节 raw `r‖s`（各 32 字节大端，左侧补零）。用下面的脚本一步到位。

---

## 4. 一键签名脚本（服务器上运行）

### 4.1 `sign_lyx.py` — 签原生应用

```python
#!/usr/bin/env python3
"""sign_lyx.py <in.lyx> <out.lyx> <priv.pem> <algo 0|1>
对 .lyx 签名：payload = 文件去尾 65 字节，trailer = algo(1B) + sig64(64B)。
依赖：openssl 3.x。"""
import subprocess, sys, struct

def sm2_sign_raw(priv: bytes, payload: bytes) -> bytes:
    """SM2 签名并 DER→raw(r||s, 32+32 大端)。"""
    p = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", priv,
                        "-rawin", "-in", "/dev/stdin"],
                       input=payload, capture_output=True, check=True)
    der = p.stdout
    # 最简 DER: 30 LL 02 lr r... 02 ls s...
    assert der[0] == 0x30
    i, out = 2, []
    for _ in range(2):
        assert der[i] == 0x02
        ln = der[i + 1]; i += 2
        v = der[i:i + ln]; i += ln
        assert len(v) <= 32
        out.append(v.rjust(32, b"\x00"))   # 左补零到 32B
    return out[0] + out[1]

def ed_sign_raw(priv: bytes, payload: bytes) -> bytes:
    p = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", priv,
                        "-rawin", "-in", "/dev/stdin"],
                       input=payload, capture_output=True, check=True)
    assert len(p.stdout) == 64
    return p.stdout

def main():
    src, dst, priv, algo = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    data = open(src, "rb").read()
    assert len(data) > 64 + 65, "image too small"
    if data[-65] in (0, 1):            # 已带 trailer → 去掉重签
        data = data[:-65]
    payload = data
    sig = sm2_sign_raw(priv, payload) if algo == 0 else ed_sign_raw(priv, payload)
    open(dst, "wb").write(payload + bytes([algo]) + sig)
    print(f"OK: {dst}  payload={len(payload)}B  algo={'SM2' if algo==0 else 'Ed25519'}")

if __name__ == "__main__":
    main()
```

### 4.2 `sign_lyap.py` — 签瞬应用包

```python
#!/usr/bin/env python3
"""sign_lyap.py <in.lyap> <out.lyap> <priv.pem> <algo 0|1>
对 .lyap 签名：被签内容 = manifest 前 160 字节，
algo 写到 0xA0(160)，sig64 写到 0xA1..0xE1(161..225)。
依赖：openssl 3.x。（DER→raw 逻辑同 sign_lyx.py）"""
import subprocess, sys
# —— 复用 sign_lyx.py 的 sm2_sign_raw / ed_sign_raw（同目录 import 或复制）——
from sign_lyx import sm2_sign_raw, ed_sign_raw

LYAP_SIGNED_LEN = 160      # 被 manifest 签名覆盖的字节数（src/lyap/mod.rs）
ALGO_OFF = 0xA0            # 160
SIG_OFF  = 0xA1            # 161

def main():
    src, dst, priv, algo = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    data = bytearray(open(src, "rb").read())
    assert len(data) >= 256, "manifest header incomplete"
    if data[0:8] != b"LYAPPKG":
        # 注意：magic 按小端 u64 = 0x4B47505041594C 存放，hexdump 应显示
        # 4C 59 41 50 50 47 4B 00 —— 即 ASCII "LYAPPKG\0"
        pass
    payload = bytes(data[:LYAP_SIGNED_LEN])
    sig = sm2_sign_raw(priv, payload) if algo == 0 else ed_sign_raw(priv, payload)
    data[ALGO_OFF] = algo
    data[SIG_OFF:SIG_OFF + 64] = sig
    open(dst, "wb").write(bytes(data))
    print(f"OK: {dst}  algo={'SM2' if algo==0 else 'Ed25519'}")

if __name__ == "__main__":
    main()
```

### 4.3 服务器端自验（签完先自己验一遍）

```bash
# 提取 payload 后用公钥自验（Ed25519 示例）
openssl pkeyutl -verify -inkey lysor_dev_ed_pub.pem -pubin -rawin \
    -in payload.bin -sigfile sig64.bin
# SM2 同理：-inkey sm2_pub.pem -pubin -rawin
```

---

## 5. 公钥注册进内核（签完后系统侧还差什么）

签名服务器只产出带签文件；**内核必须先有你开发者的公钥证书**，否则一切验签都拒绝（证书表为空 = 全拒）。

### 5.1 注册 API（`src/sig/mod.rs`）

```rust
/// dev_id 任意 16 字节内标识；algo=0 SM2 / 1 Ed25519；
/// key64：SM2 轨填 64B x‖y；Ed25519 轨填 32B 公钥（放在 key64[..32]，后 32B 置零）。
sig::ly_cert_add(b"LYSOR-DEV-0001\0\0", crypto::ALGO_SM2, &pub_xy_64b)?;

/// 吊销（泄露/轮换时）：
sig::ly_cert_revoke(slot_idx)?;
```

### 5.2 注册位置

- 内置开发者证书在 `sig::ly_pkg_sig_init()`（`src/sig/mod.rs`）中注册，随内核启动生效
- 证书表 **8 槽**（`MAX_CERTS = 8`），满了会返回 `Err("cert store full")`
- 换新密钥：新增 `ly_cert_add` + 吊销旧槽，重新编译发布内核

---

## 6. 兼容性关键点（务必核对）

| 项 | 内核实现 | 服务器端对应 | 不一致的后果 |
|----|----------|--------------|--------------|
| SM2 ZA 的 ID | `1234567812345678`（sm2.rs `za()`） | OpenSSL 默认 ID（3.x 相同） | 验签必败 |
| SM2 签名格式 | raw `r‖s` 各 32B 大端 | OpenSSL 输出 DER | 必须转换（§4.1） |
| SM2 摘要 | SM3 + ZA，`e = SM3(ZA‖msg)` | `pkeyutl -rawin` 自动 | 不要用 `dgst -sm3` 裸摘要 |
| Ed25519 | RFC 8032 标准 | 标准 raw 64B | 直接可用 |
| `.lyx` 被签范围 | `image[0..len-65]` | 去尾 65B 的 payload | 差一字节都验不过 |
| `.lyap` 被签范围 | manifest `[0..160)` | 前 160B | 同上 |
| 尾部 trailer | `algo(1B)+sig(64B)` | 脚本自动写入 | 旧 32B sig 格式已废弃 |

> **废弃说明**：早期 `.lyap` manifest 中 32 字节 `sig`/`pubkey`（`0xA0`/`0xC0`）的 ed25519-stub 布局**已作废**；现行布局为 `algo@160 + sig64@161..225`，验签走内核双轨真实现。

---

## 7. 密钥管理红线

1. **私钥只存在签名服务器**：内核源码、公开仓库、构建机、CI 全程无私钥
2. `.gitignore` 永久排除：`*.pem`、`*.key`、`d.hex`、`*priv*`
3. 私钥文件 `chmod 600`，服务器磁盘加密，操作留审计日志
4. **签名必须在外部服务器完成**——内核里 `sm2_sign`/`ed_sign` 仅用于启动自检（签名→验签回路），不用于生产签发
5. 轮换：新证书 `ly_cert_add` → 新版本全部用新钥签 → 旧槽 `ly_cert_revoke`
6. 泄露应急：立即 `ly_cert_revoke`（重启生效）+ 服务器换钥 + 全量重签
7. 验签失败**永不降级放行**：装载器直接拒绝，无任何旁路

---

## 8. 全流程速查

```
[签名服务器]                          [内核/系统]
生成 SM2/Ed25519 密钥对
公钥 HEX → 交给内核开发者注册
      │
构建产物 in.lyx / in.lyap
      │
sign_lyx.py / sign_lyap.py
(payload → openssl -rawin → raw sig64 → trailer/manifest 回填)
      │
服务器自验（pkeyutl -verify）
      │
分发 out.lyx / out.lyap  ──────────→  装载时：
                                       exec/lyap → sig::ly_verify_lyx
                                       → 遍历证书表(未吊销+algo匹配)
                                       → crypto::ly_verify_sig
                                       → 通过装载 / 失败拒绝+审计
```

---

## 9. IQCL 开发者鉴权配置方案（v0.2 服务器侧）

应用安装签名门的"未签名仅限开发者"分支依赖会话开发者判定（`userinfo.level >= 100`）。以下配置在 IQCL 站点执行（写入 www.iqcl.cn 的 Cloudflare 侧与皮肤站 OAuth 服务）：

### 9.1 Cloudflare 放行明文 HTTP（过渡）

- `/api/yggdrasil/oauth2/*` 路径**必须关闭 Always Use HTTPS 重定向**，放行明文 HTTP（v0.2 联调期 OS 端网络栈未实现 TLS，走明文过渡）；
- 安全代价如实记录：设备码轮询期间的 `authorization_pending` 响应、token 均以明文传输；仅限该路径放行，其余路径保持 HTTPS；
- 过渡性质：TLS 实现后（路线图）恢复全站强制 HTTPS。

### 9.2 开发者标记（两案并列，OS 端只认 userinfo.level）

- **方案 A（默认）**：开发者账号 `level` 置 ≥ 100（`LY_DEV_LEVEL` 编译期常量，当前 100）；`userinfo` 返回 `level`，OS 端比较即可；
- **方案 B（备选）**：维护后台开发者名单，`userinfo` 按名单把 `level` 组装为 ≥ 100 返回；两案 OS 端逻辑不变。

### 9.3 防滥用（照抄 oauth2.md 安全建议）

- 设备码 10 分钟时效（`expires_in` 收紧为 600s）；
- 会话可在皮肤站后台吊销（吊销后 OS 端下次轮询得 `access_denied`，清除本地会话回登录屏）；
- `refresh_token` 轮换：每次刷新签发新 token，旧 token 立即失效。

### 9.4 红线

- `client_secret` **永不下发**：OS 端只用免注册设备码轨，不走通用 OAuth 授权码轨；
- OS 端不存任何服务端私钥，签名/密钥管理全部留在签名服务器（§7 红线不变）。

---

## 10. .lykc 驱动签名（开放端子）

`.lykc` 容器尾部 trailer 与 `.lyx` 完全同构（`algo 1B + sig 64B`，被签范围 = 去尾 65B），**签名服务器脚本零改动即可签**：

```bash
# 1) 打包（内核侧工具，把固定基址链接的驱动 ELF 平铺为 .lykc）
python tools/mklykc.py --elf out/lykc_hello.elf --out out/lykc_hello.lykc --algo none

# 2) 签名（服务器侧，复用 §4.1 的 sign_lyx.py——payload 即 .lykc 去尾 65B）
python sign_lyx.py in.lykc out.lykc priv.pem 0     # SM2 轨（默认内核证书轨）
python sign_lyx.py in.lykc out.lykc priv.pem 1     # Ed25519 备用轨
```

- **吊销即拒绝**：签名驱动装载强制 `sig::ly_verify_lyx`（fail-closed），证书吊销（§5.1 `ly_cert_revoke`）后该驱动立即拒载，无需改驱动本身；
- **KC/.lykc 路径无 allow_unsigned 开关**：无签名 / 验签失败 / ABI 不符一律 `Err`，串口审计 + 装载界面直接报错（与内核组件装载口径一致）；
- 示例驱动走读见公开仓蓝皮书 `DRIVER_SPEC.md`。

---

## 11. 应用安装门策略（v0.2 E 项）

### 11.1 三态语义表（`lyx_verify_tri` / `lyap_verify_tri`）

| 验签结果 | 含义 | 安装行为 | 开发者会话 |
|----------|------|----------|-----------|
| `Valid` | 签名有效（证书表内、未吊销） | 直接安装（进度条 → 成功 → 开始菜单"已安装"） | 同左 |
| `Unsigned` | 无签名（algo=0xFF，`ALGO_NONE`） | **风险对话框**："该软件未签名，来源不明。继续安装仅限开发者" | `auth_is_developer()` 时"仍要安装"可选，否则按钮禁用 |
| `Bad` | 有签名但验签失败（≠ 无签名） | **直接报错对话框** | **开发者也不可绕过**（与 Unsigned 语义分离） |

### 11.2 装载器分层（`ly_install_ex(image, len, allow_unsigned)`）

- `Valid` → 静默装载；`Unsigned` 且 `allow_unsigned=false` → 拒绝；`Unsigned` 且 `allow_unsigned=true` → 装载并带"未签名"标记登记进 `/apps/` 注册表；`Bad` → **任何情况下拒绝**（fail-closed 不破坏）；
- 已安装注册表 `/etc/apps.conf` 行格式：`name|version|path|signed(0/1)`，开始菜单"已安装"分组读取渲染；
- 安装 = 拷入 `/apps/<name>.lyx` + 登记表项 + 双击启动走 exec 装载链。

### 11.3 硬错误口径（不可协商）

- **KC/.lykc 路径**：无签名即硬错误，无对话框、无旁路（用户明确要求，与 .lyx 应用路径区分）；
- **`Bad` 与 `Unsigned` 必须区分**：`Unsigned` 可经开发者门放行，`Bad` 永不可放行——混淆两者等于让篡改包获得"未签名"待遇。
