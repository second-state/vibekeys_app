# VibeKeys

A BLE CLI tool for controlling the [VibeKeys MAX](https://github.com/L-jasmine/vibekeys) keyboard device. Connects via Bluetooth Low Energy (BLE) to send text, keymap configurations, ASR settings, and WiFi settings.

[中文文档](docs/README.zh.md)

## Installation

### Download Pre-built Binary

Download the latest release from [GitHub Releases](https://github.com/second-state/vibekeys_app/releases).

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
# Add to PATH or move to a directory in PATH
```

### Build from Source

```bash
# Install dependencies (Linux)
sudo apt-get install libudev-dev libdbus-1-dev pkg-config

# Build
cargo build --release

# The binary will be at target/release/vibekeys
```

### Install Claude Code Plugin

Alternatively, install the [VibeKeys plugin](https://github.com/second-state/marketplace) for Claude Code to automatically display status on your keyboard.

Run these commands in your terminal:
```bash
# Add Second State marketplace
claude plugin marketplace add second-state/marketplace

# Install VibeKeys plugin
claude plugin install vibekeys@second-state-tools
```

## Usage

### Server Mode

VibeKeys runs as a background server. Commands automatically start the server if it's not running, or communicate with the existing server instance.

```bash
# Start the server explicitly
vibekeys start

# Stop the server
vibekeys stop
```

### Send text to keyboard display

```bash
vibekeys send "Hello World"
```

### Configure key mapping

```bash
vibekeys keymap <KEY> <BINDING>
```

Configures one key at a time. The device merges it into the existing keymap.

### Keymap profiles (switch between Claude Code & Codex)

```bash
vibekeys profile <NAME>
```

Applies a predefined set of keymaps in one command. Both profiles touch only `CUSTOM` and `YOLO`, so switching between them is a clean round-trip and other keys are left untouched.

| Profile | `CUSTOM` | `YOLO` |
|---------|----------|--------|
| `codex` | `/review` + Enter (one-key code review) | `y` (approve) |
| `claude` | `/compact` + Enter | `Shift+Tab` (allow all edits) |

```bash
vibekeys profile codex    # set up the keyboard for Codex
vibekeys profile claude   # switch back to Claude Code
```

On success, a confirmation is printed to the terminal and shown on the keyboard display (`✨ You're with Codex now` / `✨ You're with Claude Code now`). It only appears once the profile is confirmed applied; a failed send prints an error instead.

> Note: `claude` resets `CUSTOM` and `YOLO` to their **factory Claude defaults**. It doesn't restore custom bindings you may have set on those keys, so the clean `codex` ↔ `claude` round-trip holds against the defaults, not arbitrary prior state.

## Keymap Reference

### Supported Keys

| Key | Description |
|-----|-------------|
| `MIC` | Microphone key |
| `CUSTOM` | Custom key |
| `ESC` | Escape key |
| `NEXT` | Next key |
| `BACKSPACE` | Backspace key |
| `YOLO` | Yolo key |
| `ACCEPT` | Accept key |
| `ROTATE` | Rotate key |

### Binding Types

Bindings support two types: **combo** (keyboard shortcut) and **text** (text macro).

#### Combo

Maps a key to a keyboard shortcut.

```bash
# Single key
vibekeys keymap ESC A          # Map to A key
vibekeys keymap NEXT 1         # Map to digit 1

# With modifiers
vibekeys keymap ESC Ctrl+C     # Map to Ctrl+C
vibekeys keymap CUSTOM Alt+Tab # Map to Alt+Tab
vibekeys keymap NEXT Shift+A    # Map to Shift+A

# Supported modifiers
# Ctrl, Alt, Shift, Meta, Win, Cmd
# Win and Cmd are automatically converted to Meta
```

Generated JSON format:

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

#### Text Macro

Maps a key to a text string that gets typed when pressed.

```bash
# Use quotes to explicitly specify a text macro
vibekeys keymap CUSTOM '"hello world"'

# Input not recognized as a combo is treated as text
vibekeys keymap CUSTOM "some text here"
```

Generated JSON format:

```json
{
  "CUSTOM": {
    "type": "text",
    "value": "hello world",
    "raw": "\"hello world\""
  }
}
```

#### Binding Resolution Rules

Input is parsed with the following priority:

1. **Quoted string** — content wrapped in `"` or `'` is parsed as text
2. **`+` separated combo** — parsed as combo when all modifier parts are valid (e.g. `Ctrl+Alt+Delete`)
3. **Single uppercase letter or digit** — parsed as combo with no modifiers
4. **Anything else** — defaults to text

### Full Configuration Example

```bash
# MIC key → type text
vibekeys keymap MIC '"I am using Claude Code"'

# ESC key → Ctrl+C interrupt
vibekeys keymap ESC Ctrl+C

# NEXT key → open command palette
vibekeys keymap NEXT Ctrl+Shift+P

# CUSTOM key → Alt+Tab switch window
vibekeys keymap CUSTOM Alt+Tab

# BACKSPACE key → backspace
vibekeys keymap BACKSPACE Backspace
```

## Hook Mode

Reads hook JSON events from stdin and forwards them to the keyboard display.

```bash
# For Claude Code (alias: hook)
vibekeys claude

# For Codex
vibekeys codex
```

### Supported Events

| Event | Display |
|-------|---------|
| `UserPromptSubmit` | `[user] <first 80 chars of prompt>` |
| `Stop` | `[stopped]` or `[done] <last message>` |
| `Notification` | `[notify] <first 80 chars of message>` |
| `PreToolUse` | `[tool] <tool name>` |
| `PostToolUse` | `[done] <tool name>` |
| `SessionStart` | `[working]` |
| `StopFailure` | `[error] <error type>` |

### Claude Code Configuration

Add to `.claude/settings.json`:

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

## ASR Configuration

Configure the ASR (Automatic Speech Recognition) service for voice features:

```bash
# Interactive mode - prompts for provider selection and API key
vibekeys asr-config

# Direct configuration
vibekeys asr-config --uri <URI> --api-key <KEY> --model <MODEL>

# Example with OpenAI
vibekeys asr-config --uri "https://api.openai.com/v1/audio/transcriptions" --api-key sk-xxxx --model whisper-1
```

### Supported Providers (for interactive mode)

| Provider | Default URI | Default Model |
|----------|-------------|---------------|
| `openai` | `https://api.openai.com/v1/audio/transcriptions` | `whisper-1` |
| `bytefuture` | `https://models.bytefuture.ai/v1/audio/transcriptions` | `groq/whisper-large-v3` |
| `groq` | `https://api.groq.com/openai/v1/audio/transcriptions` | `whisper-large-vurbo` |
| `glm` | `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions` | `glm-asr-2512` |
| `custom` | (required) | (required) |

**Note:** The `platform` field sent to the device is always `"whisper"`. The provider selection in interactive mode only affects the default URI and model values for convenience.

### Examples

```bash
# Interactive mode (recommended) - select from provider list with pre-configured defaults
vibekeys asr-config

# Configure with Groq (fast, often has free tier)
vibekeys asr-config --uri "https://api.groq.com/openai/v1/audio/transcriptions" --api-key gsk_xxxx --model whisper-large-vurbo

# Configure with OpenAI (specify URI and model explicitly)
vibekeys asr-config --uri "https://api.openai.com/v1/audio/transcriptions" --api-key sk-xxxx --model whisper-1
```

## WiFi Configuration

Configure WiFi settings for the device:

```bash
# Interactive mode - prompts for SSID and password
vibekeys wifi-config

# Direct configuration
vibekeys wifi-config <SSID> --pass <PASSWORD>

# Open network (no password)
vibekeys wifi-config MyNetwork
```

### Examples

```bash
# Interactive mode
vibekeys wifi-config

# Configure with password
vibekeys wifi-config "MyWiFi-5G" --pass "mypassword"

# Configure open network
vibekeys wifi-config "PublicWiFi"
```

## HTTP API

When the server is running, you can also use HTTP endpoints:

```bash
# Send text
curl -X POST http://127.0.0.1:42837/send -d "Hello"

# Configure keymap
curl -X POST http://127.0.0.1:42837/keymap -d '{"KEY":"value"}'

# Configure ASR
curl -X POST http://127.0.0.1:42837/asr-config -d '{
  "platform": "whisper",
  "uri": "https://api.openai.com/v1/audio/transcriptions",
  "api_key": "sk-xxxx",
  "model": "whisper-1"
}'

# Configure WiFi
curl -X POST http://127.0.0.1:42837/wifi-config -d '{
  "ssid": "MyWiFi",
  "pass": "password"
}'
```

## ASR Result Handling

When the device sends ASR transcription results via BLE notifications:
- The text is automatically copied to your clipboard
- An acknowledgement is sent back to the device

## Logs

VibeKeys stores log files in the following locations:

- **Linux/macOS**: `~/.vibekeys/logs/`
- **Windows**: `%USERPROFILE%\.vibekeys\logs` (usually `C:\Users\<Username>\.vibekeys\logs`)

Logs are automatically rotated:
- Maximum 10MB per file
- Keeps the 5 most recent log files

You can also view logs in real-time on stderr. Set the log level with the `RUST_LOG` environment variable.

## Development

```bash
# Run with debug logging
RUST_LOG=debug vibekeys send "test"

# Build release
cargo build --release
```

## License

GNU General Public License v3.0 (GPL-3.0). See [LICENSE](LICENSE).
