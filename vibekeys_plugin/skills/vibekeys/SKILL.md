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

```bash
# Linux example
curl -LO https://github.com/second-state/vibekeys_app/releases/latest/download/vibekeys-linux-x64
chmod +x vibekeys-linux-x64
sudo mv vibekeys-linux-x64 /usr/local/bin/vibekeys
```

### Install from source

```bash
# Linux: install BLE dependencies first
sudo apt-get install libudev-dev libdbus-1-dev pkg-config

# Build and install
cargo build --release
sudo cp target/release/vibekeys /usr/local/bin/
```

Verify installation:

```bash
vibekeys --version
```

## Commands

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

## Supported Keys

| Key | Description |
|-----|-------------|
| `MIC` | Microphone key |
| `CUSTOM` | Custom key |
| `ESC` | Escape key |
| `GUI` | GUI key |
| `BACKSPACE` | Backspace key |
| `SWITCH` | Switch key |
| `ACCEPT` | Accept key |
| `ROTATE` | Rotate key |

## Binding Formats

### Combo (keyboard shortcut)

```bash
# Single key
vibekeys keymap ESC A
vibekeys keymap GUI 1

# With modifiers
vibekeys keymap ESC Ctrl+C
vibekeys keymap CUSTOM Alt+Tab
vibekeys keymap GUI Ctrl+Shift+P
```

Supported modifiers: `Ctrl`, `Alt`, `Shift`, `Meta`, `Win`, `Cmd` (`Win` and `Cmd` auto-convert to `Meta`)

### Text macro

Text that gets typed when the key is pressed. Use quotes to explicitly specify:

```bash
vibekeys keymap MIC '"I am using Claude Code"'
vibekeys keymap CUSTOM '"hello world"'
```

### Binding resolution

1. Quoted string (`"..."` or `'...'`) → text macro
2. `+` separated with valid modifiers → combo
3. Single uppercase letter or digit → combo (no modifiers)
4. Anything else → text macro

## Examples

When the user asks to set up key bindings, run the appropriate commands:

```
# User: "Map ESC to Ctrl+C"
vibekeys keymap ESC Ctrl+C

# User: "Make MIC type 'I am using Claude Code'"
vibekeys keymap MIC '"I am using Claude Code"'

# User: "Show 'working' on the keyboard"
vibekeys send "working"

# User: "Map GUI to open command palette"
vibekeys keymap GUI Ctrl+Shift+P
```
