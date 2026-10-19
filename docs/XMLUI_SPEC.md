# LYSOR OS XML 布局规范（XMLUI_SPEC）

瞬应用（.lyap）使用 XML 文件声明式描述界面布局，由 XMLUI 解析引擎渲染。布局文件放在 `.lyap` 包内的 `/layout/main.xml`。

## 1. 标签

### `<label>` — 文字标签

```xml
<label x="20" y="20" text="Hello World" />
```

### `<button>` — 按钮

```xml
<button x="100" y="100" w="120" h="36" text="OK" onclick="on_ok" />
```

### `<input>` — 输入框

```xml
<input x="50" y="200" w="200" />
```

## 2. 属性

| 属性 | 类型 | 说明 |
|------|------|------|
| x | int | X 坐标 |
| y | int | Y 坐标 |
| w | int | 宽度 |
| h | int | 高度 |
| text | string | 显示文字 |
| onclick | string | 点击事件回调名 |

## 3. 完整布局示例

```xml
<layout>
    <label x="20" y="20" text="My Snap App" />
    <input x="20" y="60" w="200" />
    <button x="20" y="110" w="100" h="36" text="Submit" onclick="on_submit" />
    <button x="130" y="110" w="100" h="36" text="Cancel" onclick="on_cancel" />
</layout>
```

## 4. 沙盒限制

- 瞬应用 UI 只能在自己的窗口内绘制
- 禁止读取其他窗口的像素内容
- 事件回调只能调用沙盒内允许的 API
- 超出窗口边界的绘制会被裁剪

## 5. Manifest 关联字段

manifest 中声明布局与逻辑入口（见 [LYAP_SPEC.md](LYAP_SPEC.md)）：

```
layout_path = "/layout/main.xml"   # UI 布局文件路径
script_path = "/logic/main.js"     # 解释逻辑文件路径
```

`onclick` 指定的回调名在逻辑文件中实现，由 XMLUI 运行时按名分发。
