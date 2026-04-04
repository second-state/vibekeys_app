use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter, WriteType};
use btleplug::platform::Adapter;
use btleplug::platform::Manager;
use btleplug::platform::Peripheral as PlatformPeripheral;
use clap::{Parser, Subcommand};
use log::{debug, info, warn};
use std::io::{self, Read};
use std::time::{Duration, Instant};
use tokio::time;
use uuid::Uuid;

/// BLE Controller CLI
#[derive(Parser, Debug)]
#[command(name = "vibekeys")]
#[command(version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Send a message to the connected device
    Send {
        /// Message to send
        message: String,
    },
    /// Configure key mapping (merged, can be done one key at a time)
    Keymap {
        /// Key name (MIC, CUSTOM, ESC, GUI, BACKSPACE, SWITCH, ACCEPT, ROTATE)
        key: String,
        /// Key binding (e.g., "A", "Ctrl+C", Alt+Tab", "\"text\"")
        binding: String,
    },
    /// Read Claude Code hook JSON from stdin and forward to device
    Hook,
}

// Controller Service UUID
const CONTROLLER_SERVICE_ID: Uuid = Uuid::from_u128(0x9c80ffb6_affa_4083_944a_91e34c88bd76);

// Keyboard Display Characteristic UUID
const KEYBOARD_DISPLAY_ID: Uuid = Uuid::from_u128(0xcdaa6472_67a8_4241_93cf_145051608573);

// Keymap Config Characteristic UUID
const KEYMAP_CONFIG_ID: Uuid = Uuid::from_u128(0x6f2a291c_0e4d_4f0f_9446_50bcd0b73bb0);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Send { message } => {
            send_to_device(KEYBOARD_DISPLAY_ID, message.as_bytes()).await?;
            Ok(())
        }
        Command::Keymap { key, binding } => {
            send_keymap(&key, &binding).await?;
            Ok(())
        }
        Command::Hook => handle_hook().await,
    }
}

// Get Bluetooth adapter
async fn get_adapter() -> anyhow::Result<(Manager, Adapter)> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters
        .into_iter()
        .next()
        .expect("No Bluetooth adapter found");
    Ok((manager, adapter))
}

// Scan and find the target peripheral, retrying until found
async fn scan_and_find_peripheral(adapter: &Adapter) -> anyhow::Result<PlatformPeripheral> {
    let mut filter = ScanFilter::default();
    filter.services.push(CONTROLLER_SERVICE_ID);

    loop {
        adapter.start_scan(filter.clone()).await?;

        let peripherals = loop {
            time::sleep(Duration::from_millis(100)).await;
            let peripherals = adapter.peripherals().await?;
            if !peripherals.is_empty() {
                break peripherals;
            }
        };

        if let Some(target) = find_peripheral(&peripherals, CONTROLLER_SERVICE_ID).await? {
            adapter.stop_scan().await?;
            return Ok(target);
        }

        adapter.stop_scan().await?;
        warn!("Target device not found, retrying...");
    }
}

// Send data to device (keeps manager alive)
async fn send_to_device(char_uuid: Uuid, data: &[u8]) -> anyhow::Result<()> {
    let t0 = Instant::now();

    let _manager = {
        let (manager, adapter) = get_adapter().await?;
        info!("[{:.0?}] Adapter ready", t0.elapsed());

        let peripheral = scan_and_find_peripheral(&adapter).await?;
        connect_and_discover(&peripheral).await?;
        info!("[{:.0?}] Connected & discovered", t0.elapsed());

        send_message(&peripheral, char_uuid, data).await?;
        info!("[{:.0?}] Data sent", t0.elapsed());

        peripheral.disconnect().await.ok();
        info!("[{:.0?}] Total", t0.elapsed());
        manager
    };
    Ok(())
}

// Handle Claude Code hook input from stdin
async fn handle_hook() -> anyhow::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).ok();

    let hook: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let event = hook["hook_event_name"].as_str().unwrap_or("");

    let message = match event {
        "UserPromptSubmit" => {
            let prompt = hook["prompt"].as_str().unwrap_or("");
            format!("[user] {}", truncate(prompt, 80))
        }
        "Stop" => "[stopped]".to_string(),
        "Notification" => {
            let msg = hook["message"].as_str().unwrap_or("");
            format!("[notify] {}", truncate(msg, 80))
        }
        "PreToolUse" => {
            let tool = hook["tool_name"].as_str().unwrap_or("");
            format!("[tool] {}", tool)
        }
        "PostToolUse" => {
            let tool = hook["tool_name"].as_str().unwrap_or("");
            format!("[done] {}", tool)
        }
        "SessionStart" => "[working]".to_string(),
        "StopFailure" => {
            let error = hook["error"].as_str().unwrap_or("unknown");
            format!("[error] {}", error)
        }
        _ => {
            info!("Unhandled hook event: {}", event);
            return Ok(());
        }
    };

    send_to_device(KEYBOARD_DISPLAY_ID, message.as_bytes())
        .await
        .ok();
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

// Send keymap configuration
async fn send_keymap(key: &str, binding: &str) -> anyhow::Result<()> {
    let key_upper = key.to_uppercase();

    // Validate key name
    let valid_keys = [
        "MIC",
        "CUSTOM",
        "ESC",
        "NEXT",
        "BACKSPACE",
        "SWITCH",
        "ACCEPT",
        "ROTATE",
    ];
    if !valid_keys.contains(&key_upper.as_str()) {
        anyhow::bail!("Invalid key name. Valid keys: {}", valid_keys.join(", "));
    }

    let parsed = parse_key_binding(binding);
    let config = serde_json::json!({ key_upper: parsed });
    let json_str = config.to_string();
    info!("Sending keymap: {}", json_str);

    send_to_device(KEYMAP_CONFIG_ID, json_str.as_bytes()).await
}

// Unescape common escape sequences in quoted strings
fn unescape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some('0') => result.push('\0'),
                Some('x') => {
                    // Handle \xNN hex escape
                    let hex1 = chars.next().unwrap_or('0');
                    let hex2 = chars.next().unwrap_or('0');
                    if let Some(byte) = u8::from_str_radix(&format!("{}{}", hex1, hex2), 16).ok() {
                        result.push(byte as char);
                    }
                }
                Some(other) => {
                    // Unknown escape, keep as-is
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn parse_key_binding(input: &str) -> serde_json::Value {
    let trimmed = input.trim();

    // Text macro: quoted string
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let content = &trimmed[1..trimmed.len() - 1];
        // Unescape common escape sequences
        let unescaped = unescape_string(content);
        return serde_json::json!({
            "type": "text",
            "value": unescaped,
            "raw": input,
        });
    }

    // Valid special key names (matching controller_zh.html)
    let valid_keys = [
        // Basic navigation
        "enter",
        "return",
        "space",
        "tab",
        "escape",
        "esc",
        "backspace",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "up",
        "down",
        "left",
        "right",
        // Modifiers (can also be used as standalone keys)
        "ctrl",
        "shift",
        "alt",
        "option",
        // System keys
        "gui",
        "win",
        "meta",
        "cmd",
        "command",
        // F-keys
        "f1",
        "f2",
        "f3",
        "f4",
        "f5",
        "f6",
        "f7",
        "f8",
        "f9",
        "f10",
        "f11",
        "f12",
        // Symbols
        "plus",
        "minus",
        "equal",
        "semicolon",
        "quote",
        "backquote",
        "backslash",
        "comma",
        "period",
        "slash",
        "bracketleft",
        "bracketright",
    ];

    let valid_modifiers = ["ctrl", "alt", "option", "shift", "meta", "win", "cmd"];

    // Check if input is a single uppercase letter (as combo)
    if trimmed.len() == 1
        && trimmed
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_uppercase())
    {
        return serde_json::json!({
            "type": "combo",
            "modifiers": [],
            "key": trimmed,
            "raw": input,
        });
    }

    // Check if input is a single digit (as combo)
    if trimmed.len() == 1 && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return serde_json::json!({
            "type": "combo",
            "modifiers": [],
            "key": trimmed,
            "raw": input,
        });
    }

    // Check if input is a known key name
    let key_lower = trimmed.to_lowercase();
    if valid_keys.contains(&key_lower.as_str()) {
        return serde_json::json!({
            "type": "combo",
            "modifiers": [],
            "key": trimmed.to_uppercase(),
            "raw": input,
        });
    }

    // Combo with + separator
    if trimmed.contains('+') {
        let parts: Vec<&str> = trimmed.split('+').map(|p| p.trim()).collect();

        // Check if all parts except last are valid modifiers
        let all_mods_valid = parts[..parts.len() - 1]
            .iter()
            .all(|p| valid_modifiers.contains(&p.to_lowercase().as_str()));

        if all_mods_valid {
            let last_part = parts.last().unwrap();
            let last_lower = last_part.to_lowercase();

            // Check if last part is a valid key (known name OR alphanumeric)
            let is_valid_key = valid_keys.contains(&last_lower.as_str())
                || (last_part.len() == 1
                    && last_part
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_alphanumeric()));

            if is_valid_key {
                let modifiers: Vec<String> = parts[..parts.len() - 1]
                    .iter()
                    .map(|p| match p.to_lowercase().as_str() {
                        "win" | "cmd" => "meta".to_string(),
                        "option" => "alt".to_string(),
                        other => other.to_string(),
                    })
                    .collect();
                return serde_json::json!({
                    "type": "combo",
                    "modifiers": modifiers,
                    "key": last_part.to_uppercase(),
                    "raw": input,
                });
            }
        }
    }

    // Default: text
    serde_json::json!({
        "type": "text",
        "value": trimmed,
        "raw": input,
    })
}

async fn find_peripheral(
    peripherals: &[PlatformPeripheral],
    target_service: Uuid,
) -> anyhow::Result<Option<PlatformPeripheral>> {
    for peripheral in peripherals {
        let addr = peripheral.address();
        if let Some(props) = peripheral.properties().await? {
            let name = props.local_name.unwrap_or("(unknown)".to_string());
            let rssi = props.rssi.unwrap_or(0);
            info!("  {} - {} (RSSI: {})", addr, name, rssi);

            for service in &props.services {
                debug!("    Service UUID: {}", service);
            }

            let has_target_service = props.services.iter().any(|s| *s == target_service);

            if has_target_service {
                info!("    >>> Found target service!");
                return Ok(Some(peripheral.clone()));
            }

            debug!("----------------------------");
        }
    }

    Ok(None)
}

// Connect to device and discover services
async fn connect_and_discover(peripheral: &PlatformPeripheral) -> anyhow::Result<()> {
    let t = Instant::now();
    peripheral.connect().await?;
    info!("[{:.0?}] Connected", t.elapsed());

    let t2 = Instant::now();
    peripheral.discover_services().await?;
    info!("[{:.0?}] Services discovered", t2.elapsed());

    Ok(())
}

// Send message to characteristic
async fn send_message(
    peripheral: &PlatformPeripheral,
    char_uuid: Uuid,
    data: &[u8],
) -> anyhow::Result<()> {
    let characteristics = peripheral.characteristics();

    for char in &characteristics {
        if char.uuid == char_uuid {
            peripheral
                .write(char, data, WriteType::WithResponse)
                .await?;
            return Ok(());
        }
    }

    Err(anyhow::anyhow!("Characteristic {} not found", char_uuid))
}
