# LYSOR OS 服务器配置指南（SERVER-CONFIG.md）

> 本文件是**所有需要你（项目所有者）自行配置的服务端项**的唯一清单。
> 放在公开仓根目录，随仓库同步。**私钥、种子、令牌一律不进入本仓库**
> （见 docs/REDACTION_LOG.md 红线 #2/#4），本文件只给**模板、公钥与流程**。
> 配置项按"系统内读取路径 / 服务端要求 / 你把值填到哪"三段式写。

---

## 0. 配置总览（改这一节即可上线的项）

| 配置项 | 系统读取位置 | 状态 | 你需要做的事 |
|---|---|---|---|
| 应用商店服务端 | `/etc/store.conf` → `base_url=` | 未配置（显示"软件库未配置"） | 见 §3 |
| 软件库下载源 | 同 `base_url`（/repo/ 路径约定） | 占位 | 见 §3.2 |
| OAuth2 授权服务 | `www.iqcl.cn`（本机代理 127.0.0.1:8087） | 约定已留、服务端待建 | 见 §4 |
| 浏览器 TLS 证书 | `/etc/tls/`（M8 内核 TLS 客户端已闭环；证书加载待浏览器联调） | 已实现 | 见 §5 |
| 开发者签名公钥 | 内核内嵌证书（`src/sig/mod.rs`，LYSOR-DEV-0001/0002） | 已内置开发轨 | 见 §1 |
| 域名重定向 | Cloudflare 规则（rainyun.iqcl.de5.net → www.iqcl.de5.net/rainyun） | 用户已配 | 见 §6 |

---

## 1. 验签密钥体系（签名服务端 / 开发者公钥登记）

### 1.1 双轨算法与当前内置证书（公开信息，可进仓库）

- **国密轨**：SM2（GB/T 32918），证书 `LYSOR-DEV-0001`（slot 0）。
- **现代轨**：Ed25519（RFC 8032），证书 `LYSOR-DEV-0002`（slot 1）。
- 内核验证逻辑（fail-closed）：`src/sig/mod.rs` → `crypto::ly_verify_sig`；
  无匹配证书 / 算法非法 / 验签失败 → 一律拒绝（Unsigned/Bad 三态见
  docs/SIGNING_GUIDE.md）。

### 1.2 Ed25519 开发轨公钥（已内置内核，属公开密钥）

```
dev seed 指纹：f3669833a1e5e4beb5c2db9cea233cb58c7583aaa693fcd7252578afd423ddc6
派生公钥    ：2278f7387e5643945ea3d35c810b4c1b3a4df8dbd039d803102294dcd7168818
```

- **种子（私钥材料）只保存在你的离线签名服务器**，绝不进本仓库。
- 系统内签名/验签工具链：`tools/ed25519.py`（纯 python，RFC 8032）、
  `tools/sign_lyx.py`、`tools/sign_lyap.py`；用法见 docs/SIGNING_GUIDE.md §4。

### 1.3 你（服务端）要做的

1. 在离线机生成开发种子：`python tools/ed25519.py gen > /secure/dev-ed25519.seed`
   （或沿用上面指纹对应的种子，二者任选其一；换种子必须同步更新内核证书与
   已发布包签名）。
2. 派生公钥：`python -c "import sys;sys.path.insert(0,'tools');import ed25519;print(ed25519.public_key(open(r'/secure/dev-ed25519.seed','rb').read()).hex())"`。
3. 把公钥更新进 `src/sig/mod.rs` 的 `LYSOR-DEV-0002`（key[..32]），重建内核。
4. 生产签名一律走离线签名服务器：签 .lyx → `python tools/sign_lyx.py in.lyx out.sig.lyx <seed> 1`；
   签 .lyap → `python tools/sign_lyap.py ...`（参数见 SIGNING_GUIDE）。
5. **吊销**：在 `src/sig/mod.rs` 对应证书置 `revoked=true` 后重建发布（内核拒绝旧包）。

> 注意：内核证书里 `key[..32]` 放的是**公钥**（不是种子）。把种子误放进
> key 数组会导致验签恒失败（本仓库已修正过该问题，见 REDACTION_LOG）。

---

## 2. 应用安装与签名门（系统内既有，无需服务端）

- 安装门三态：`Valid`（验签通过）/ `Unsigned`（无签名区，默认拒绝）/
  `Bad`（签名无效，强制拒绝）。UI 可显式放行 Unsigned（测试用），执行路径仍拒。
- 已装应用登记 `/etc/apps.conf`；系统预装原生应用示范：`install_ex_selftest`
  （签名）+ 玩机技巧 `play-tips`（Ed25519 签 `.lyx` 内嵌内核镜像，开机装载，
  冒烟断言 `play-tips PROCESS started` → `exit(0x54)`）。
- 开发者判级：`auth::auth_is_developer()`（userinfo.level >= 100，见 §4）。

---

## 3. 应用商店与软件库（**待你搭建，本仓库只留配置位**）

### 3.1 系统内配置位（已实现）

- 运行时配置：`/etc/store.conf`，格式：
  ```
  base_url = https://你的商店域名/   ← 启用商店；缺省或空行 = 商店显示"软件库未配置"
  ```
- 商店 UI（`src/apps/mod.rs` Store 应用）：已安装列表 + signed 徽标 + 安装指引。
- 配置文档：docs/DEVELOPER_GUIDE.md §5.2（公开仓已同步）。

### 3.2 软件库下载源（占位 → 你提供地址后替换）

```
占位地址：<软件库根 URL，等你搭建后提供，替换这一行>
```

**服务端最小要求**（商店按此约定拉包）：
- 目录约定：`<base_url>/repo/<app_id>/<version>/` 存放
  `app.lyap`（或 `app.lyx`）+ `manifest.json`。
- `manifest.json` 最小字段：
  ```json
  {
    "app_id": "com.iqcl.demo",
    "name": "示例应用",
    "version": "1.0.0",
    "algo": 1,
    "sig_hex": "<签名 hex，ED25519>",
    "pub_hex": "<公钥 hex>",
    "size": 12345
  }
  ```
- 商店客户端流程：拉 manifest → 校验签名 → 下载包 → 安装门验签 → 登记 /etc/apps.conf。
- 你提供地址后：把 URL 填入 3.1 的 `base_url`（系统内 `/etc/store.conf`），并同步
  更新本文件 3.2 的占位。

### 3.3 应用发布流程（服务端侧）

1. 离线机签好 `app.lyap`（§1.3）。
2. 上传到 `<base_url>/repo/<app_id>/<version>/` + manifest。
3. 系统内商店拉取即完成"发布"；**没有中心审核节点**（社区陪审团机制为
   OAuth2 生态的一部分，见 docs/reference/oauth2.md）。

---

## 4. OAuth2 授权服务（www.iqcl.cn 服务端，待你实现）

系统内已有：设备码/轮询/userinfo 会话（`src/auth.rs`）+ 本机代理约定
`127.0.0.1:8087`（见 system.md "本机 OAuth2 接口"）。

**服务端要实现 3 个端点**（与内核约定对齐）：

| 端点 | 请求 | 返回 |
|---|---|---|
| `/oauth2/device` | 无参数 | `device_code`, `user_code`, `verification_uri`, `expires_in=600` |
| `/oauth2/poll` | `device_code` | `authorization_pending` / `success(+token)` / `access_denied` |
| `/oauth2/userinfo` | `access_token` | `{"level": 0..999}`（level>=100 判开发者） |

安全要求：
- `expires_in` 固定 600s；`refresh_token` 每次轮换。
- `client_secret` 永不下发（OS 只用免注册设备码轨）。
- 过渡期明文 HTTP 仅限该路径；TLS 上线后全站强制 HTTPS。
- userinfo 返回 `level` 字段是开发者判定（Shan IDE 打包门解锁）的唯一依据。

---

## 5. 浏览器 TLS / HTTPS 证书（M8 已实现；证书加载待浏览器联调）

- **M8 状态**：内核 TLS 1.2 客户端已完成端到端闭环（ECDHE-RSA-AES128-GCM-SHA256，
  RSA-SHA256 验签 → PRF 派钥 → GCM 记录保护），冒烟验收 `tls: E2E OK bytes=175`
  已通过（服务器纯 python 权威实现对拍）。实现位置 `src/net/tls.rs` + `src/crypto/`
  （rsa/sha256/aes），服务器参考实现 `tools/tls_server.py`。
- 证书存放：系统内 `/etc/tls/cert.pem` + `key.pem`（浏览器 HTTPS 联调阶段实现读取）。
- 服务端证书：`www.iqcl.cn` 用 Cloudflare Origin / 免费 CA 均可；浏览器内置
  LYSOR 根证书/固定钉扎，QEMU 内基础可用即可（用户已放权简化）。
- 生成自签（开发/内网测试）：
  ```sh
  openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 365 \
    -subj "/CN=iqcl.cn" -addext "subjectAltName=DNS:iqcl.cn,DNS:www.iqcl.cn"
  ```
- 系统侧已做：CSP 默认策略（§7）+ TLS 客户端验链（RSA 证书链 + 签名校验）+ 记录层
  AEAD；**不做**完整 Chrome 级 CA 体系（QEMU 可用即可）。
- 下一步（浏览器子系统，见 system.md 接力说明）：webview + 类 Chrome 浏览器，
  本机 OAuth2 接口 `127.0.0.1:8087` 已留。

---

## 6. 域名与 Cloudflare（已完成 / 无需再动）

- `iqcl.cn` 已购，用于项目部署。
- 重定向规则：`rainyun.iqcl.de5.net` → `www.iqcl.de5.net/rainyun`（用户已配）。
- Cloudflare Workers 静态→动态自动切换为后续规划（system.md 记录），
  当前不在服务端配置清单内。
- **不推荐也不使用 Oracle Cloud（OCI）任何产品**（用户明确要求）。

---

## 7. 浏览器内容安全策略（CSP，系统默认策略）

浏览器渲染远程/本地页面时应用默认 CSP（白名单优先）：

```
default-src 'none';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data:;
connect-src 'self' https://www.iqcl.cn;
frame-ancestors 'none'
```

- 本地 `ly://` 页面与商店包页面自动授予 `'self'`；远程 HTTP 页面降级执行
  （无 JS，仅文档渲染）直到 TLS 上线。
- 详情与实现状态见 docs/BROWSER_PLAN.md（M6 CSP 里程碑）。

---

## 8. 配置核对清单（上线前逐项打勾）

- [ ] §1：离线签名服务器就位，开发种子与内核证书公钥一致
- [ ] §3：软件库服务器搭建，`base_url` 填入系统 `/etc/store.conf` 并替换 §3.2 占位
- [ ] §4：OAuth2 三端点上线（device/poll/userinfo），level>=100 判开发者
- [ ] §5：TLS 证书就绪（服务端 + 系统 /etc/tls 浏览器联调），CSP 默认策略生效
  （M8 内核 TLS 客户端 E2E 已通过，见 docs/BROWSER_PLAN.md）
- [ ] §6：域名/重定向确认（已配）
- [ ] 本文件 §3.2 占位替换为真实软件库地址后同步回公开仓
