# 瞬语言（Shan Language）规范 v1.0

> 瞬应用 = 小程序语义：点开即用、无需安装、最多预编译。
> 瞬语言是 LYSOR OS 的**解释型**应用语言，类 Python 子集，由内核内建
> tree-walk 解释器（`src/shan/mod.rs`）直接执行，**无编译步骤、无依赖安装**。
> 解释器为 no_std 真实现：固定缓冲、无堆分配、步数上限防死循环。

## 1. 设计目标

| 目标 | 实现方式 |
|------|----------|
| 点开即用 | 点击瞬应用图标 → 内核提取包内 `main.sl` → 解释器立即执行 |
| 零安装 | `.lyap` 安装即提取到 `/sandbox/<app>/`，无注册 DLL / 无环境变量 |
| 最多预编译 | v1 纯解释；未来可对热路径做字节码预编译（格式预留 `shan:` entry） |
| 降低开发难度 | 语法对齐 Python 常见写法（无括号 if/while、print 免括号） |
| 安全 | 沙箱配额 + 双轨验签（SM2 国密 / Ed25519）+ 步数预算 + 固定缓冲 |

## 2. 语法

```
程序    := 语句*
语句    := let 变量 = 表达式 ;
        | 赋值          变量 = 表达式 ;
        | if 条件 { 语句* } [ else { 语句* } ]
        | while 条件 { 语句* }
        | fn 名字 ( 参数... ) { 语句* }
        | return [表达式] ;
        | print 参数... ;
        | 表达式 ;
条件/表达式 := 二元运算（|| && 比较 加减 乘除 % 一元 ! -）+ 原子
原子    := 数字 | "字符串" | true | false | 变量 | 函数调用 | ( 表达式 )
```

- `if` / `while` 的条件**括号可选**：`if a < 2 { ... }` 与 `if (a < 2) { ... }` 等价。
- `print` 的括号可选：`print "hello", i;` 与 `print("hello", i);` 等价。
- 行注释：`#` 到行尾。
- 语句以 `;` 结尾；块用 `{ }`。

## 3. 值类型

| 类型 | 表示 | 示例 |
|------|------|------|
| 整数 | i64 | `42`, `-7` |
| 字符串 | 字面量池切片 | `"hello"`, `"a" + "b"` |
| 布尔 | 整数 0/1 | `true`, `false` |

- 字符串用 `+` 连接（任一操作数为字符串时执行拼接）。
- 比较：`== != < <= > >=`，字符串按字典序。
- 逻辑：`&& || !`（非零即真，空字符串为假）。

## 4. 内建函数

| 函数 | 说明 |
|------|------|
| `print(...)` | 输出到当前瞬应用窗口 / 串口（关键字形式免括号） |
| `sleep(ms)` | 忙等约 ms 毫秒 |
| `tick()` | 返回 PIT 时钟 tick 计数（100Hz） |
| `strlen(s)` | 字符串字节数 |
| `abs(n)` | 绝对值 |
| `num(s)` | 字符串 → 整数（全数字，可带 `-`） |
| `str(n)` | 整数 → 字符串 |

## 4.1 原生图形化 draw API（直接对接图形 API）

瞬应用是图形化系统的一等公民：draw 系列内建**直写 256×192 画布**
（`src/shan/mod.rs` SHAN_CANVAS，RGBA），窗口渲染时 blit 上屏。
**不依赖任何第三方注解库**；`print` 文本输出保留在窗口输出区。
括号可选：`cls 10,12,16;` 与 `cls(10,12,16);` 等价。

| 函数 | 说明 |
|------|------|
| `cls(r,g,b)` | 清屏为 r,g,b（0-255） |
| `px(x,y,r,g,b)` | 单像素 |
| `rect(x,y,w,h,r,g,b)` | 填充矩形 |
| `line(x0,y0,x1,y1,r,g,b)` | 直线（Bresenham） |
| `text(x,y,s,r,g,b)` | 文本（16px 字形，直连 font::font_render 灰度混合） |

示例（自检同源）：

```
cls 10, 12, 16;
rect 4, 4, 32, 16, 0, 120, 212;      # 蓝色矩形
line 40, 4, 80, 4, 200, 200, 210;    # 灰白横线
text 84, 4, "LYSOR", 230, 230, 240;  # 文字
px 0, 0, 255, 255, 255;              # 左上角白点
```

启动自检输出 `0 1 2 3 4 13 shan-ok gfx-ok`：最后一段 `gfx-ok` 即 draw API
执行 + 画布像素校验（cls 底色 / rect 填充 / px 落点）通过。

## 5. 作用域与函数

- 全局变量：顶层 `let` 定义，任意函数可读。
- 函数参数：按定义处的参数名绑定（`fn fib(a) { ... }` 中 `a` 即参数）。
- 调用深度上限 15 层；每帧 16 个变量槽。
- 函数体内 `return` 立即终止函数（含块内 `return`）。
- 递归示例：

```
fn fib(a) { if a < 2 { return a; } return fib(a - 1) + fib(a - 2); }
print fib(7);          # -> 13
```

## 6. 资源与安全上限

| 上限 | 值 |
|------|-----|
| 源脚本长度 | ≤ 8192 B（包内 `main.sl`） |
| token 数 | 512 |
| 字符串池 | 1024 B |
| 函数数 | 16 |
| 调用深度 | 15 层 |
| 步数预算 | 1,000,000（防死循环，超限报 `step budget exceeded`） |
| 输出缓冲 | 窗口 4096 B |

## 7. 包格式与验签

瞬应用以 `.lyap` 分发（`docs/LYAP_MANIFEST.md` v2）：

- manifest `entry` 字段：
  - `shan:main.sl` → 装载时提取 `/sandbox/<app>/main.sl` → `shan::ly_shan_run` 解释执行；
  - `bin:main.lyx` → 装载时提取 `/sandbox/<app>/main.lyx` → `exec::ly_exec_lyx`
    （编译型语言产物，双轨验签，同一装载器）。
- 统一启动 `lyap::lyap_launch(app, out)`：`main.lyx` 存在 → 装载；否则 `main.sl` → 解释。
- 签名仍为 manifest 前 160 字节双轨真验签（SM2 国密优先 / Ed25519 备用）；
  `algo=0xFF` 未签名仅限开发者会话放行；验签失败（Bad）任何情况拒装。
- 私钥只存外部签名服务器；内核只有公钥证书表（`sig::ly_cert_add`，8 槽）。

## 8. 内核 API（供 IDE / 装载器使用）

```rust
pub fn ly_shan_check(src: &[u8], err_out: &mut [u8]) -> Result<(), &'static str>; // 语法检查
pub fn ly_shan_run(src: &[u8], out: &mut [u8]) -> Result<isize, &'static str>;    // 解释执行
pub fn ly_shan_run_str(src: &str, out: &mut [u8]) -> Result<isize, &'static str>;
pub fn ly_shan_selftest(out: &mut [u8]) -> isize;                                 // 启动自检
pub fn ly_shan_canvas_ptr() -> *const u8;          // 画布 RGBA 指针（256x192）
pub fn ly_shan_canvas_clear(r: u8, g: u8, b: u8);  // 清画布
```

启动自检（`ly_shan_init`）执行 while 循环 + fib 递归 + 字符串连接 + draw 图形，
输出 `0 1 2 3 4 13 shan-ok gfx-ok` 即解释器与图形栈闭环正常（QEMU 冒烟已验）。

## 9. 开发流程（Shan IDE，可登录）

1. 打开 **Shan IDE**（控制面板第 10 格）编辑 `main.sl`；
2. `Tab` 语法检查；`Run` 立即试运行（画布 + print 输出页）；`Save` 存 `/sandbox/<app>/main.sl`；
3. `Pack`（需开发者登录，`userinfo.level >= 100`）生成未签名 `.lyap` 到 `/apps/`
   并登记 `/etc/apps.conf`；
4. 外部签名服务器执行 `tools/sign_lyap.py <pkg> <out> <priv.pem> <algo>`
   （F10 参数页选择 SM2/Ed25519 与 dev id）；
5. 文件管理器打开签名后的 `.lyap` → 验签三态 → 安装 → 开始菜单/Shan Run 点开即用；
6. **原生应用**（第 4 页 Native / F10 循环）：编译型语言（Rust 等）→
   `tools/lyx_build_app.ps1` 产出 `.lyx` → 服务器 `sign_lyx.py` 签名 →
   安装或打包为 `bin:main.lyx` 入口的 `.lyap`。

## 10. 跨语言支持（非兼容层，全部自有 API）

- **瞬语言**（解释型）：内核内建解释器，**点开即用**；
- **Rust（编译型）**：编译为 `.lyx` 进瞬应用（`bin:` 入口）或独立原生应用；
  允许预编译提速；
- **JS/TS（解释型，规划）**：需对应解释器（选型 `docs/BROWSER_PLAN.md` §4，
  MIT/Apache-2.0 轻量引擎），TS 编译为 JS 后解释；也可预编译字节码；
- **原生应用**：仅编译型语言，**不支持瞬语言**；运行时依赖必须随包打包
  （SDK 静态链接，见 `docs/DEVELOPER_GUIDE.md` §1）。
- 演进：v1.1 可加列表/字典与 for-in；v2 可选字节码预编译（"最多预编译"）。
