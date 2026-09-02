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

Reads hook JSON events from stdin, extracts the session id (short code), workspace name, and status,
and forwards a structured multi-session event to the keyboard. The device keeps one entry per session
and can display several agent sessions at once. See [docs/session-events.md](docs/session-events.md)
for the wire format.

```bash
# For Claude Code (alias: hook)
vibekeys claude

# For Codex
vibekeys codex
```

### Supported Events

| Claude / Codex Event | Status sent |
|----------------------|-------------|
| `UserPromptSubmit` / `SessionStart` | `work` |
| `PreToolUse` | `tool` |
| `PostToolUse` / `SubagentStop` (Codex) | `post` |
| `Notification` (`permission_prompt`) / `PermissionRequest` (Codex) | `perm` |
| `Notification` (`idle_prompt`) | `note` |
| `Stop` | `done` |
| `StopFailure` | `err` |

> The Claude hooks config filters `Notification` with the matcher
> `permission_prompt|idle_prompt`, so other notification types never invoke
> the command. Stale sessions are removed by the device after an inactivity
> timeout.

### Manual Status (debugging)

Send one session event by hand — useful for testing the multi-session display without hooks:

```bash
# vibekeys session <sid> <status>
vibekeys session abcd1234 tool
```

The project name is taken from the current working directory. Valid statuses:
`work`, `tool`, `post`, `perm`, `note`, `done`, `err`, `end`.

(Use `vibekeys notify "text"` or `vibekeys send "text"` to display a plain text
message instead.)

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

The device stores up to 8 WiFi networks as a priority-ordered list. Interactive mode shows the current list and lets you add or remove a network; the direct form appends (or updates) one network.

```bash
# Interactive TUI: list current networks, then add or remove
vibekeys wifi-config

# Direct: add (or update) one network
vibekeys wifi-config <SSID> --pass <PASSWORD>

# Open network (no password)
vibekeys wifi-config MyNetwork
```

### Examples

```bash
# Interactive: add / remove from the list
vibekeys wifi-config

# Add a network with a password
vibekeys wifi-config "MyWiFi-5G" --pass "mypassword"

# Add an open network
vibekeys wifi-config "PublicWiFi"
```

## Mic Mode Configuration

Configure the microphone trigger mode in keyboard mode:

```bash
# Interactive mode - select toggle or ptt
vibekeys mic-model

# Direct configuration
vibekeys mic-model toggle   # tap to start/stop
vibekeys mic-model ptt      # push to talk (hold)
```

## Prefer Built-in ASR

In keyboard mode, choose whether to use the device's built-in ASR (Whisper) or pass the mic through to the host (which triggers the host's own dictation):

```bash
# Interactive mode
vibekeys prefer-builtin-asr

# Direct configuration
vibekeys prefer-builtin-asr on     # use built-in Whisper
vibekeys prefer-builtin-asr off    # pass mic through to host
```

## Server URL Configuration

Configure the server URL:

```bash
# Interactive mode
vibekeys server-url

# Direct configuration
vibekeys server-url https://example.com
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

# Configure WiFi (append/update one network)
curl -X POST http://127.0.0.1:42837/wifi-config -d '{
  "ssid": "MyWiFi",
  "pass": "password"
}'

# Replace the whole wifi_list (priority order, max 8)
curl -X POST http://127.0.0.1:42837/wifi-config -d '{
  "wifi_list": [{"ssid":"A","pass":"a"},{"ssid":"B","pass":""}]
}'

# Read the full config snapshot (wifi_list / server_url / asr_config / mic_model / prefer_builtin_asr)
curl http://127.0.0.1:42837/config

# Configure mic mode (toggle | ptt)
curl -X POST http://127.0.0.1:42837/mic-model -d '{"mode":"toggle"}'

# Prefer built-in ASR on/off
curl -X POST http://127.0.0.1:42837/prefer-builtin-asr -d '{"value":true}'

# Configure server URL
curl -X POST http://127.0.0.1:42837/server-url -d '{"url":"https://example.com"}'
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
