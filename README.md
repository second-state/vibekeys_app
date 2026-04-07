# VibeKeys

A BLE CLI tool for controlling the [VibeKeys MAX](https://github.com/L-jasmine/vibekeys) keyboard device. Connects via Bluetooth Low Energy (BLE) to send text and keymap configurations.

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

### Send text to keyboard display

```bash
vibekeys send "Hello World"
```

### Configure key mapping

```bash
vibekeys keymap <KEY> <BINDING>
```

Configures one key at a time. The device merges it into the existing keymap.

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

Reads Claude Code hook JSON events from stdin and forwards them to the keyboard display. Used for Claude Code hooks integration.

```bash
vibekeys hook
```

### Supported Events

| Event | Display |
|-------|---------|
| `UserPromptSubmit` | `[user] <first 80 chars of prompt>` |
| `Stop` | `[stopped]` |
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

## Development

```bash
# Run with debug logging
RUST_LOG=debug vibekeys send "test"

# Build release
cargo build --release
```

## License

MIT
