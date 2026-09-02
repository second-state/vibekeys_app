# VibeKeys

BLE CLI 工具，用于控制 [VibeKeys MAX](https://github.com/L-jasmine/vibekeys) 键盘设备。通过蓝牙低功耗 (BLE) 连接设备，发送文字、按键映射配置、ASR 设置和 WiFi 设置。

## 安装

### 下载预编译二进制

从 [GitHub Releases](https://github.com/second-state/vibekeys_app/releases) 下载最新版本。

**Linux:**
```bash
wget https://github.com/second-state/vibekeys_app/releases/latest/download/vibekeys-linux-x64
chmod +x vibekeys-linux-x64
sudo mv vibekeys-linux-x64 /usr/local/bin/vibekeys
```

**macOS (ARM64):**
```bash
wget https://github.com/second-state/vibekeys_app/releases/latest/download/vibekeys-macos-arm64
chmod +x vibekeys-macos-arm64
sudo mv vibekeys-macos-arm64 /usr/local/bin/vibekeys
```

**Windows (PowerShell):**
```powershell
Invoke-WebRequest -Uri "https://github.com/second-state/vibekeys_app/releases/latest/download/vibekeys-windows-x64.exe" -OutFile "vibekeys.exe"
# 添加到 PATH 或移动到 PATH 中的目录
```

### 从源码构建

```bash
# 安装依赖 (Linux)
sudo apt-get install libudev-dev libdbus-1-dev pkg-config

# 构建
cargo build --release

# 编译产物位于 target/release/vibekeys
```

### 安装 Claude Code 插件

你也可以安装 [VibeKeys 插件](https://github.com/second-state/marketplace) 让 Claude Code 自动在键盘上显示状态。

在终端运行以下命令：
```bash
# 添加 Second State marketplace
claude plugin marketplace add second-state/marketplace

# 安装 VibeKeys 插件
claude plugin install vibekeys@second-state-tools
```

## 用法

### 服务器模式

VibeKeys 作为后台服务器运行。命令会自动启动服务器（如果未运行）或与现有服务器通信。

```bash
# 显式启动服务器
vibekeys start

# 停止服务器
vibekeys stop
```

### 发送文字到键盘显示

```bash
vibekeys send "Hello World"
```

### 配置按键映射

```bash
vibekeys keymap <KEY> <BINDING>
```

每次配置一个键，设备会合并到已有的按键映射中。

### 按键映射 profile（在 Claude Code 与 Codex 之间切换）

```bash
vibekeys profile <NAME>
```

一条命令套用一组预设按键映射。两个 profile 都只修改 `CUSTOM` 和 `YOLO` 两个键，因此来回切换是干净的往返，不会影响其他键。

| Profile | `CUSTOM` | `YOLO` |
|---------|----------|--------|
| `codex` | `/review` + 回车（一键代码审查） | `y`（批准） |
| `claude` | `/compact` + 回车 | `Shift+Tab`（允许全部编辑） |

```bash
vibekeys profile codex    # 把键盘配成 Codex 用法
vibekeys profile claude   # 切回 Claude Code
```

切换成功后，会在终端打印并在键盘屏幕上显示确认信息（`✨ You're with Codex now` / `✨ You're with Claude Code now`）。该提示仅在整组按键确认下发成功后才出现；若下发失败，则改为显示错误。

> 注意:`claude` 会把 `CUSTOM` 和 `YOLO` 重置为 **Claude 出厂默认**。它不会恢复你之前给这两个键设置的自定义绑定,因此 `codex` ↔ `claude` 的干净往返只针对默认值,不针对任意先前状态。

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

从 stdin 读取 hook JSON 事件,提取会话短码、workspace 名和状态,转发为结构化的多会话事件到键盘。设备端按会话维护多条目,可同时显示多个 agent 会话。线格式见 [session-events.md](session-events.md)。

```bash
# For Claude Code (别名: hook)
vibekeys claude

# For Codex
vibekeys codex
```

### 支持的事件

| 事件 | 发送的状态 |
|------|-----------|
| `UserPromptSubmit` / `SessionStart` | `work` |
| `PreToolUse` | `tool` |
| `PostToolUse` / `SubagentStop`(Codex) | `post` |
| `Notification`(`permission_prompt`)/ `PermissionRequest`(Codex) | `perm` |
| `Notification`(`idle_prompt`) | `note` |
| `Stop` | `done` |
| `StopFailure` | `err` |

> Claude hooks 配置用 matcher `permission_prompt|idle_prompt` 过滤了 `Notification`,其他通知类型不会触发命令。长时间不活跃的会话由设备端超时自动移除。

### 手工发送状态(调试)

不经过 hooks,直接发一条会话事件,用于测试多会话显示:

```bash
# vibekeys session <会话短码> <状态>
vibekeys session abcd1234 tool
```

项目名取自当前工作目录。可用状态:`work`、`tool`、`post`、`perm`、`note`、`done`、`err`、`end`。

(发纯文本请用 `vibekeys notify "文本"` 或 `vibekeys send "文本"`。)

项目名取自当前工作目录。可用状态:`work`、`tool`、`post`、`perm`、`note`、`done`、`err`、`end`。

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
        "matcher": "permission_prompt|idle_prompt",
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "*",
        "hooks": [{ "type": "command", "command": "vibekeys hook" }]
      }
    ]
  }
}
```

## ASR 配置

配置 ASR（自动语音识别）服务用于语音功能：

```bash
# 交互式配置 - 提示选择供应商和输入 API Key
vibekeys asr-config

# 直接配置
vibekeys asr-config --uri <URI> --api-key <密钥> --model <模型>

# 示例：OpenAI
vibekeys asr-config --uri "https://api.openai.com/v1/audio/transcriptions" --api-key sk-xxxx --model whisper-1
```

### 支持的供应商（交互式模式）

| 供应商 | 默认 URI | 默认模型 |
|--------|----------|----------|
| `openai` | `https://api.openai.com/v1/audio/transcriptions` | `whisper-1` |
| `bytefuture` | `https://models.bytefuture.ai/v1/audio/transcriptions` | `groq/whisper-large-v3` |
| `groq` | `https://api.groq.com/openai/v1/audio/transcriptions` | `whisper-large-vurbo` |
| `glm` | `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions` | `glm-asr-2512` |
| `custom` | (必需) | (必需) |

**注意：** 发送到设备的 `platform` 字段始终为 `"whisper"`。交互式模式中的供应商选择仅影响默认的 URI 和 model 值，方便快速配置。

### 使用示例

```bash
# 交互式模式（推荐）- 从供应商列表选择，自动填充默认值
vibekeys asr-config

# 配置 Groq（速度快，通常有免费额度）
vibekeys asr-config --uri "https://api.groq.com/openai/v1/audio/transcriptions" --api-key gsk_xxxx --model whisper-large-vurbo

# 配置 OpenAI（显式指定 URI 和 model）
vibekeys asr-config --uri "https://api.openai.com/v1/audio/transcriptions" --api-key sk-xxxx --model whisper-1
```

## WiFi 配置

为设备配置 WiFi 设置：

```bash
# 交互式配置 - 提示输入 SSID 和密码
vibekeys wifi-config

# 直接配置
vibekeys wifi-config <SSID> --pass <密码>

# 开放网络（无密码）
vibekeys wifi-config MyNetwork
```

### 示例

```bash
# 交互式模式
vibekeys wifi-config

# 配置带密码的网络
vibekeys wifi-config "MyWiFi-5G" --pass "mypassword"

# 配置开放网络
vibekeys wifi-config "PublicWiFi"
```

## HTTP API

服务器运行时，也可以使用 HTTP 端点：

```bash
# 发送文字
curl -X POST http://127.0.0.1:42837/send -d "Hello"

# 配置按键映射
curl -X POST http://127.0.0.1:42837/keymap -d '{"KEY":"value"}'

# 配置 ASR
curl -X POST http://127.0.0.1:42837/asr-config -d '{
  "platform": "whisper",
  "uri": "https://api.openai.com/v1/audio/transcriptions",
  "api_key": "sk-xxxx",
  "model": "whisper-1"
}'

# 配置 WiFi
curl -X POST http://127.0.0.1:42837/wifi-config -d '{
  "ssid": "MyWiFi",
  "pass": "password"
}'
```

## ASR 结果处理

当设备通过 BLE 通知发送 ASR 转录结果时：
- 文本会自动复制到剪贴板
- 会向设备发送确认信号

## 日志文件

VibeKeys 将日志存储在以下位置：

- **Linux/macOS**: `~/.vibekeys/logs/`
- **Windows**: `%USERPROFILE%\.vibekeys\logs`（通常是 `C:\Users\<用户名>\.vibekeys\logs`）

日志会自动轮转：
- 单文件最大 10MB
- 保留最近 5 个日志文件

日志同时输出到 stderr，方便实时查看。可通过 `RUST_LOG` 环境变量设置日志级别。

## 开发

```bash
# 调试模式运行（查看详细日志）
RUST_LOG=debug vibekeys send "test"

# 构建 release
cargo build --release
```

## License

GNU General Public License v3.0 (GPL-3.0)。见 [LICENSE](../LICENSE)。
