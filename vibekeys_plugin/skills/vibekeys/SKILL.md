---
name: vibekeys
description: Control VibeKeys MAX BLE keyboard - install, configure keymaps, ASR, and WiFi
---

# VibeKeys Controller

Control the VibeKeys MAX BLE keyboard device from Claude Code. Use this skill to install the CLI, configure key mappings, configure ASR/WiFi settings, and send text to the keyboard display.

## Prerequisites

The `vibekeys` binary must be installed and the VibeKeys MAX device must be powered on and within Bluetooth range.

### Download from GitHub Releases

Download the prebuilt binary for your platform from [Releases](https://github.com/second-state/vibekeys_app/releases).

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

### Install from source

```bash
# Install dependencies (Linux)
sudo apt-get install libudev-dev libdbus-1-dev pkg-config

# Build
cargo build --release

# The binary will be at target/release/vibekeys
```

## Commands

### Server Mode

VibeKeys runs as a background server. Commands automatically start the server if it's not running:

```bash
# Explicitly start the server
vibekeys start

# Stop the server
vibekeys stop
```

### Send text to keyboard display

Display a text message on the VibeKeys MAX screen:

```bash
vibekeys send "Hello World"
```

The server connects via BLE and stays running for subsequent commands.

### Configure key mapping

Map a physical key to a keyboard shortcut or text macro:

```bash
vibekeys keymap <KEY> <BINDING>
```

Each call configures one key. The device merges it into the existing keymap.

### ASR Configuration

Configure the ASR (Automatic Speech Recognition) service for voice features:

```bash
# Interactive mode - prompts for provider selection and API key
vibekeys asr-config

# Direct configuration
vibekeys asr-config --uri <URI> --api-key <KEY> --model <MODEL>
```

**Supported providers (affects default URI and model):**
- `openai` - OpenAI Whisper (default: `https://api.openai.com/v1/audio/transcriptions`, `whisper-1`)
- `bytefuture` - ByteFuture Groq Whisper (default: `groq/whisper-large-v3`)
- `groq` - Groq Whisper (default: `whisper-large-vurbo`)
- `glm` - GLM (智谱) ASR (default: `glm-asr-2512`)
- `custom` - Custom ASR endpoint (URI and model required)

**Note:** The `platform` field sent to the device is always `"whisper"`. The provider only affects default URI and model values.

**Examples:**
```bash
# Interactive mode (recommended - select provider with pre-configured defaults)
vibekeys asr-config

# Direct configuration with URI and API key
vibekeys asr-config --uri "https://api.groq.com/openai/v1/audio/transcriptions" --api-key gsk_xxxx --model whisper-large-vurbo

# Direct configuration with API key only (uses defaults)
vibekeys asr-config --api-key sk-xxxx
```

### WiFi Configuration

Configure WiFi settings for the device:

```bash
# Interactive mode - prompts for SSID and password
vibekeys wifi-config

# Direct configuration
vibekeys wifi-config <SSID> --pass <PASSWORD>

# Open network (no password)
vibekeys wifi-config MyNetwork
```

**Examples:**
```bash
# Interactive mode
vibekeys wifi-config

# Configure with password
vibekeys wifi-config "MyWiFi-5G" --pass "mypassword"

# Configure open network
vibekeys wifi-config "PublicWiFi"
```

### Hook Mode (for Claude Code / Codex integration)

Reads hook JSON events from stdin and forwards them to the keyboard display:

```bash
# For Claude Code (alias: hook)
vibekeys claude

# For Codex
vibekeys codex
```

## Supported Keys

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

Note: `YOLO` is an alias for the SWITCH key.

## Binding Formats

### Combo (keyboard shortcut)

```bash
# Single key (letter, digit, or special key name)
vibekeys keymap ESC A
vibekeys keymap NEXT 1
vibekeys keymap CUSTOM Enter
vibekeys keymap MIC Space

# With modifiers
vibekeys keymap ESC Ctrl+C
vibekeys keymap CUSTOM Alt+Tab
vibekeys keymap NEXT Ctrl+Shift+P
vibekeys keymap ROTATE Option+Cmd+Space
```

Special key names: `Enter`, `Return`, `Space`, `Tab`, `Escape`, `Esc`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Up`, `Down`, `Left`, `Right`, `F1`-`F12`, `Plus`, `Minus`, `Equal`, `Semicolon`, `Quote`, `Backquote`, `Backslash`, `Comma`, `Period`, `Slash`, `BracketLeft`, `BracketRight`, `Ctrl`, `Shift`, `Alt`, `Option`, `GUI`, `Win`, `Meta`, `Cmd`, `Command`

Supported modifiers: `Ctrl`, `Alt`, `Option`, `Shift`, `Meta`, `Win`, `Cmd` (`Win`/`Cmd` → `Meta`, `Option` → `Alt`)

### Text macro

Text that gets typed when the key is pressed. Use quotes to explicitly specify:

```bash
vibekeys keymap MIC '"I am using Claude Code"'
vibekeys keymap CUSTOM '"hello world"'
```

### Binding resolution

1. Quoted string (`"..."` or `'...'`) → text macro
2. Known key name (case-insensitive) → combo
3. Single letter or digit → combo
4. `+` separated with valid modifiers → combo (e.g., `Ctrl+c`, `Alt+Tab`)
5. Anything else → text macro

Note: Modifiers and key names are case-insensitive. `Ctrl+c`, `ctrl+C`, and `CTRL+C` all work.

## Examples

When the user asks to set up key bindings, run the appropriate commands:

```
# User: "Map ESC to Ctrl+C"
vibekeys keymap ESC Ctrl+C

# User: "Make MIC type 'I am using Claude Code'"
vibekeys keymap MIC '"I am using Claude Code"'

# User: "Show 'working' on the keyboard"
vibekeys send "working"

# User: "Map NEXT to open command palette"
vibekeys keymap NEXT Ctrl+Shift+P

# User: "Map ROTATE to Cmd+Space (Mac Spotlight)"
vibekeys keymap ROTATE Cmd+Space

# User: "Map CUSTOM to Option+Tab"
vibekeys keymap CUSTOM Option+Tab

# User: "Map ACCEPT to F5"
vibekeys keymap ACCEPT F5

# User: "Configure ASR with Groq"
vibekeys asr-config

# User: "Configure WiFi"
vibekeys wifi-config
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
