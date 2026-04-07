# VibeKeys

BLE CLI 工具，用于控制 [VibeKeys MAX](https://github.com/L-jasmine/vibekeys) 键盘设备。通过蓝牙低功耗 (BLE) 连接设备，发送文字和按键映射配置。

## 安装

```bash
# Linux 需要安装依赖
sudo apt-get install libudev-dev libdbus-1-dev pkg-config

cargo build --release
```

## 用法

### 发送文字到键盘显示

```bash
vibekeys send "Hello World"
```

### 配置按键映射

```bash
vibekeys keymap <KEY> <BINDING>
```

每次配置一个键，设备会合并到已有的按键映射中。

## Keymap 详细说明

### 支持的按键

| 按键名 | 说明 |
|--------|------|
| `MIC` | 麦克风键 |
| `CUSTOM` | 自定义键 |
| `ESC` | Escape 键 |
| `NEXT` | Next 键 |
| `BACKSPACE` | 退格键 |
| `YOLO` | Yolo 键 |
| `ACCEPT` | 确认键 |
| `ROTATE` | 旋转键 |

### 绑定格式

绑定支持两种类型：**组合键 (combo)** 和 **文本宏 (text)**。

#### 组合键 (combo)

将按键映射为一个快捷键组合。

```bash
# 单个按键
vibekeys keymap ESC A          # 映射为 A 键
vibekeys keymap NEXT 1         # 映射为数字 1

# 带修饰键的组合
vibekeys keymap ESC Ctrl+C     # 映射为 Ctrl+C
vibekeys keymap CUSTOM Alt+Tab # 映射为 Alt+Tab
vibekeys keymap NEXT Shift+A   # 映射为 Shift+A

# 支持的修饰键
# Ctrl, Alt, Shift, Meta, Win, Cmd
# Win 和 Cmd 会自动转换为 Meta
```

生成的 JSON 格式：

```json
{
  "ESC": {
    "type": "combo",
    "modifiers": ["ctrl"],
    "key": "C",
    "raw": "Ctrl+C"
  }
}
```

#### 文本宏 (text)

将按键映射为一段文字，按下时自动输入。

```bash
# 用引号包裹来明确指定文本宏
vibekeys keymap CUSTOM '"hello world"'

# 不识别为组合键的输入也会被当作文本宏
vibekeys keymap CUSTOM "some text here"
```

生成的 JSON 格式：

```json
{
  "CUSTOM": {
    "type": "text",
    "value": "hello world",
    "raw": "\"hello world\""
  }
}
```

#### 绑定类型判断规则

输入会按以下优先级解析：

1. **引号包裹** — 用 `"` 或 `'` 包裹的内容解析为 text
2. **`+` 分隔的组合键** — 所有修饰键部分合法时解析为 combo（如 `Ctrl+Alt+Delete`）
3. **单个大写字母或数字** — 解析为 combo（无修饰键）
4. **其他** — 默认解析为 text

### 示例：完整配置

```bash
# MIC 键 → 发送文本
vibekeys keymap MIC '"I am using Claude Code"'

# ESC 键 → Ctrl+C 中断
vibekeys keymap ESC Ctrl+C

# NEXT 键 → 打开命令面板
vibekeys keymap NEXT Ctrl+Shift+P

# CUSTOM 键 → Alt+Tab 切换窗口
vibekeys keymap CUSTOM Alt+Tab

# BACKSPACE 键 → 退格
vibekeys keymap BACKSPACE Backspace
```

## Hook 模式

从 stdin 读取 Claude Code hook JSON 事件，转发到键盘显示。用于 Claude Code 的 hooks 集成。

```bash
vibekeys hook
```

### 支持的事件

| 事件 | 显示内容 |
|------|----------|
| `UserPromptSubmit` | `[user] <prompt 前80字符>` |
| `Stop` | `[stopped]` |
| `Notification` | `[notify] <消息前80字符>` |
| `PreToolUse` | `[tool] <工具名>` |
| `PostToolUse` | `[done] <工具名>` |
| `SessionStart` | `[working]` |
| `StopFailure` | `[error] <错误类型>` |

### Claude Code 配置示例

在 `.claude/settings.json` 中配置：

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ],
    "Stop": [
      {
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ],
    "Notification": [
      {
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ]
  }
}
```

## 开发

```bash
# 调试模式运行（查看详细日志）
RUST_LOG=debug vibekeys send "test"

# 构建 release
cargo build --release
```

## License

MIT
