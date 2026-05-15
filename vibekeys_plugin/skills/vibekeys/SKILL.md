---
name: vibekeys
description: Control VibeKeys MAX BLE keyboard - install, configure keymaps, and send text
---

# VibeKeys Controller

Control the VibeKeys MAX BLE keyboard device from Claude Code. Use this skill to install the CLI, configure key mappings, and send text to the keyboard display.

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

VibeKeys runs as a background daemon server:

```bash
# Start the server (runs in background, first command starts it)
vibekeys

# Stop the server
vibekeys stop
```

### Send text to keyboard display

Display a text message on the VibeKeys MAX screen:

```bash
vibekeys send "Hello World"
```

This connects to the device via BLE, sends the text, and disconnects. Takes a few seconds.

### Configure key mapping

Map a physical key to a keyboard shortcut or text macro:

```bash
vibekeys keymap <KEY> <BINDING>
```

Each call configures one key. The device merges it into the existing keymap.

### Hook Mode (for Claude Code / Codex integration)

Reads hook JSON events from stdin and forwards to the keyboard display:

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
```
