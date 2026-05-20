use arboard::Clipboard;

use axum::{
    extract::State,
    routing::{get, post},
    Router,
};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Read, Write as IoWrite};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

const APP_NAME: &str = "vibekeys";
const DEFAULT_PORT: u16 = 42837;

use anyhow;
use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral as PlatformPeripheral};
use futures::StreamExt;
use tokio::sync::mpsc;
use uuid::Uuid;

const CONTROLLER_SERVICE_ID: Uuid = Uuid::from_u128(0x623fa3e2_631b_4f8f_a6e7_a7b09c03e7e0);
const KEYBOARD_DISPLAY_ID: Uuid = Uuid::from_u128(0xcdaa6472_67a8_4241_93cf_145051608573);
const KEYMAP_CONFIG_ID: Uuid = Uuid::from_u128(0x6f2a291c_0e4d_4f0f_9446_50bcd0b73bb0);
const KEYMAP_ASR_RESULT_ID: Uuid = Uuid::from_u128(0xf67f3c25_c9f0_456e_955e_cd9d9dd91051);

// ===== CLI =====

#[derive(Parser, Debug)]
#[command(name = "vibekeys", version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the vibekeys server (runs in background)
    Start,
    /// Send a message to the connected device
    Send { message: String },
    /// Configure key mapping
    Keymap { key: String, binding: String },
    /// Read Claude Code hook JSON from stdin and forward to device
    Claude,
    /// Alias for 'claude' - reads Claude Code hook JSON from stdin and forwards to device
    Hook,
    /// Read Codex hook JSON from stdin and forward to device
    Codex,
    /// Stop the running server
    Stop,
    /// Setup ASR configuration
    Setup {
        /// ASR provider (OpenAI, ByteFuture, Groq, GLM, Custom)
        #[arg(short, long)]
        platform: Option<String>,
        /// API key for the ASR service
        #[arg(long)]
        api_key: Option<String>,
    },
}

// ===== Port =====

fn get_port() -> u16 {
    std::env::var("VIBEKEYS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

// ===== Logging =====

fn log_message(msg: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let line = format!("[{}] {} {}\n", ts, APP_NAME, msg);
    #[cfg(unix)]
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/vibekeys.log")
    {
        let _ = f.write_all(line.as_bytes());
    }
    #[cfg(windows)]
    {
        let p = std::env::var("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("vibekeys.log");
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

// ===== HTTP Client =====

fn check_server(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
        .no_proxy()
        .build();

    if let Ok(client) = client {
        if let Ok(resp) = client.get(&url).send() {
            if let Ok(text) = resp.text() {
                return text.trim() == "ok";
            }
        }
    }
    false
}

// ===== BLE Functions =====

async fn get_adapter() -> anyhow::Result<(Manager, Adapter)> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Bluetooth adapter"))?;
    Ok((manager, adapter))
}

async fn scan_and_find_peripheral(adapter: &Adapter) -> anyhow::Result<PlatformPeripheral> {
    let mut filter = ScanFilter::default();
    filter.services.push(CONTROLLER_SERVICE_ID);
    adapter.start_scan(filter.clone()).await?;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let peripherals = adapter.peripherals().await?;
        if !peripherals.is_empty() {
            adapter.stop_scan().await?;
            if let Some(p) = find_peripheral(&peripherals, CONTROLLER_SERVICE_ID).await? {
                return Ok(p);
            }
            adapter.start_scan(filter.clone()).await?;
        }
    }
    adapter.stop_scan().await?;
    anyhow::bail!("Device scan timeout")
}

async fn find_peripheral(
    peripherals: &[PlatformPeripheral],
    target_service: Uuid,
) -> anyhow::Result<Option<PlatformPeripheral>> {
    for p in peripherals {
        if let Some(props) = p.properties().await? {
            if props.services.iter().any(|s| *s == target_service) {
                return Ok(Some(p.clone()));
            }
        }
    }
    Ok(None)
}

async fn connect_and_discover(p: &PlatformPeripheral) -> anyhow::Result<()> {
    p.connect().await?;
    p.discover_services().await?;
    // Subscribe to ASR result characteristic notifications
    for c in p.characteristics() {
        if c.uuid == KEYMAP_ASR_RESULT_ID {
            p.subscribe(&c).await?;
            log_message("Subscribed to ASR result notifications");
            break;
        }
    }
    Ok(())
}

async fn send_ble(p: &PlatformPeripheral, char_uuid: Uuid, data: &[u8]) -> anyhow::Result<()> {
    for c in &p.characteristics() {
        if c.uuid == char_uuid {
            p.write(c, data, WriteType::WithResponse).await?;
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("Characteristic {} not found", char_uuid))
}

enum BleCmd {
    Send {
        char_uuid: Uuid,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// Write acknowledge (1u8) to ASR result characteristic
async fn write_asr_acknowledge(
    peripheral: &PlatformPeripheral,
    asr_char: &btleplug::api::Characteristic,
) {
    #[cfg(not(target_os = "macos"))]
    const PASTE_CODE: u8 = 1;
    #[cfg(target_os = "macos")]
    const PASTE_CODE: u8 = 1;

    if let Err(e) = peripheral
        .write(asr_char, &[PASTE_CODE], WriteType::WithResponse)
        .await
    {
        log_message(&format!("Failed to write ASR acknowledge: {}", e));
    } else {
        log_message("ASR acknowledge sent");
    }
}

/// Set text to clipboard
fn set_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = Clipboard::new() {
        if let Err(e) = clipboard.set_text(text) {
            log_message(&format!("Failed to set clipboard: {}", e));
        } else {
            log_message(&format!("Text set to clipboard: {}", text));
        }
    } else {
        log_message("Failed to access clipboard");
    }
}

/// Handle ASR result notifications: set to clipboard and acknowledge with 1u8
async fn handle_asr_notifications(
    peripheral: &PlatformPeripheral,
) -> Option<(btleplug::api::Characteristic, String)> {
    log_message("ASR notification handler started");

    // Find the ASR result characteristic
    let asr_char = peripheral
        .characteristics()
        .iter()
        .find(|c| c.uuid == KEYMAP_ASR_RESULT_ID)
        .cloned()?;

    let mut notify_stream = peripheral.notifications().await.ok()?;

    // Listen for notifications, filtering by ASR characteristic UUID
    while let Some(notification) = notify_stream.next().await {
        // Only process notifications from ASR result characteristic
        if notification.uuid != KEYMAP_ASR_RESULT_ID {
            continue;
        }

        let data = notification.value;
        if !data.is_empty() {
            // Extract string from notification data
            let text = String::from_utf8_lossy(&data).to_string();
            return Some((asr_char, text));
        }
    }

    None
}

enum SelectResult {
    BleCmd(BleCmd),
    AsrResult(btleplug::api::Characteristic, String),
}

async fn select_rx_and_notify(
    rx: &mut mpsc::Receiver<BleCmd>,
    peripheral: &PlatformPeripheral,
) -> Option<SelectResult> {
    tokio::select! {
        cmd = rx.recv() => {
            cmd.map(|c| SelectResult::BleCmd(c))
        }
        asr_result = handle_asr_notifications(peripheral) => {
            asr_result.map(|res| SelectResult::AsrResult(res.0, res.1))
        }
    }
}

async fn ble_task(mut rx: mpsc::Receiver<BleCmd>) {
    let mut peripheral: Option<PlatformPeripheral> = None;
    let mut first_connect_deadline =
        Some(tokio::time::Instant::now() + std::time::Duration::from_secs(10));
    loop {
        let need_connect = match &peripheral {
            None => true,
            Some(p) => !p.is_connected().await.unwrap_or(false),
        };
        if need_connect {
            log_message("Scanning for BLE device...");
            match try_ble_connect().await {
                Ok(p) => {
                    log_message("BLE device connected");
                    peripheral = Some(p);
                    first_connect_deadline = None; // Connected, clear deadline
                }
                Err(e) => {
                    // If we were connected before and lost connection, exit
                    if peripheral.is_some() {
                        log_message(&format!("BLE disconnected: {}", e));
                        return;
                    }
                    // First connection attempt - check deadline
                    if let Some(deadline) = first_connect_deadline {
                        if tokio::time::Instant::now() > deadline {
                            log_message("First connection timeout, exiting");
                            return;
                        }
                        log_message(&format!("BLE scan failed: {}, retrying", e));
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    } else {
                        // Should not happen
                        return;
                    }
                }
            }
        }
        if let Some(ref p) = peripheral {
            match select_rx_and_notify(&mut rx, p).await {
                Some(SelectResult::BleCmd(BleCmd::Send {
                    char_uuid,
                    data,
                    reply,
                })) => {
                    // Check connection before sending with 1s timeout
                    log_message("check connect status");
                    let connected =
                        tokio::time::timeout(std::time::Duration::from_secs(1), p.is_connected())
                            .await
                            .unwrap_or(Ok(false))
                            .unwrap_or(false);

                    if !connected {
                        log_message("BLE disconnected before send, exiting");
                        let _ = reply.send(Err("BLE disconnected".to_string()));
                        return;
                    }

                    log_message("start send");
                    let result = send_ble(p, char_uuid, &data).await;
                    log_message("send end");
                    let _ = reply.send(result.map_err(|e| e.to_string()));
                }
                Some(SelectResult::AsrResult(asr_char, text)) => {
                    log_message(&format!("ASR result received: {}", text));
                    set_to_clipboard(&text);
                    write_asr_acknowledge(p, &asr_char).await;
                }
                None => return,
            }
        }
    }
}

async fn try_ble_connect() -> anyhow::Result<PlatformPeripheral> {
    let (_manager, adapter) = get_adapter().await?;
    let p = scan_and_find_peripheral(&adapter).await?;
    connect_and_discover(&p).await?;
    Ok(p)
}

async fn ble_send(ble_tx: &mpsc::Sender<BleCmd>, char_uuid: Uuid, data: &[u8]) -> String {
    let (tx, rx) = oneshot::channel();
    if ble_tx
        .send(BleCmd::Send {
            char_uuid,
            data: data.to_vec(),
            reply: tx,
        })
        .await
        .is_err()
    {
        return "error: BLE not available\n".to_string();
    }
    match rx.await {
        Ok(Ok(())) => "ok\n".to_string(),
        Ok(Err(e)) => format!("error: {}\n", e),
        Err(_) => "error: no response\n".to_string(),
    }
}

// ===== Axum HTTP Server =====

struct AppState {
    ble_tx: mpsc::Sender<BleCmd>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn root_handler(State(_): State<Arc<AppState>>) -> String {
    format!("vibekeys server running")
}

async fn shutdown_handler(State(state): State<Arc<AppState>>) -> String {
    log_message("Shutdown requested");
    let tx = state.shutdown_tx.lock().await.take();
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
    "shutting down\n".to_string()
}

async fn send_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let result = ble_send(&state.ble_tx, KEYBOARD_DISPLAY_ID, body.as_bytes()).await;
    // If BLE disconnected, shut down the server
    if result.contains("disconnected") {
        log_message("BLE disconnected, shutting down server");
        let tx = state.shutdown_tx.lock().await.take();
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
    }
    result
}

async fn keymap_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let result = ble_send(&state.ble_tx, KEYMAP_CONFIG_ID, body.as_bytes()).await;
    // If BLE disconnected, shut down the server
    if result.contains("disconnected") {
        log_message("BLE disconnected, shutting down server");
        let tx = state.shutdown_tx.lock().await.take();
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
    }
    result
}

async fn run_server(port: u16, initial_cmd: Option<Command>) {
    let (ble_tx, ble_rx) = mpsc::channel(16);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    if let Some(ble_cmd) = initial_cmd.map(command_to_blecmd).flatten() {
        if ble_tx.send(ble_cmd).await.is_err() {
            log_message("Failed to send initial command to BLE task");
        }
    }

    let state = Arc::new(AppState {
        ble_tx,
        shutdown_tx: Mutex::new(Some(shutdown_tx)),
    });

    tokio::spawn(ble_task(ble_rx));

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/shutdown", get(shutdown_handler))
        .route("/send", post(send_handler))
        .route("/keymap", post(keymap_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    log_message(&format!("Listening on {}", addr));
    println!("vibekeys server started on port {}", port);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();

    log_message("Server stopped");
}

// ===== Command Forwarding =====

async fn send_command(port: u16, message: &str) {
    let url = format!("http://127.0.0.1:{}/send", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build();

    if let Ok(client) = client {
        match client.post(&url).body(message.to_string()).send().await {
            Ok(resp) => {
                if let Ok(text) = resp.text().await {
                    print!("{}", text);
                }
            }
            Err(_) => eprintln!("Failed to connect to server"),
        }
    } else {
        eprintln!("Failed to connect to server");
    }
}

async fn send_keymap(port: u16, config: &str) {
    let url = format!("http://127.0.0.1:{}/keymap", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build();

    if let Ok(client) = client {
        match client.post(&url).body(config.to_string()).send().await {
            Ok(resp) => {
                if let Ok(text) = resp.text().await {
                    print!("{}", text);
                }
            }
            Err(_) => eprintln!("Failed to connect to server"),
        }
    } else {
        eprintln!("Failed to connect to server");
    }
}

fn command_to_blecmd(cmd: Command) -> Option<BleCmd> {
    match cmd {
        Command::Start | Command::Stop | Command::Setup { .. } => None,
        Command::Send { message } => {
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: KEYBOARD_DISPLAY_ID,
                data: message.into_bytes(),
                reply: tx,
            })
        }
        Command::Keymap { key, binding } => {
            let config = build_keymap_config(&key, &binding);
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: KEYMAP_CONFIG_ID,
                data: config.into_bytes(),
                reply: tx,
            })
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            format_claude_message(&input).map(|msg| {
                let (tx, _) = oneshot::channel();
                BleCmd::Send {
                    char_uuid: KEYBOARD_DISPLAY_ID,
                    data: msg.into_bytes(),
                    reply: tx,
                }
            })
        }
        Command::Codex => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            format_codex_message(&input).map(|msg| {
                let (tx, _) = oneshot::channel();
                BleCmd::Send {
                    char_uuid: KEYBOARD_DISPLAY_ID,
                    data: msg.into_bytes(),
                    reply: tx,
                }
            })
        }
    }
}

async fn forward_command(port: u16, cmd: &Command) {
    match cmd {
        Command::Start | Command::Stop | Command::Setup { .. } => unreachable!(),
        Command::Send { message } => {
            send_command(port, message).await;
        }
        Command::Keymap { key, binding } => {
            let config = build_keymap_config(key, binding);
            send_keymap(port, &config).await;
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            if let Some(msg) = format_claude_message(&input) {
                send_command(port, &msg).await;
            }
        }
        Command::Codex => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            if let Some(msg) = format_codex_message(&input) {
                send_command(port, &msg).await;
            }
        }
    }
}

fn build_keymap_config(key: &str, binding: &str) -> String {
    let key_upper = key.to_uppercase();
    let key_mapped = if key_upper == "YOLO" {
        "SWITCH".to_string()
    } else {
        key_upper
    };
    let parsed = parse_key_binding(binding);
    serde_json::json!({ key_mapped: parsed }).to_string()
}

fn format_claude_message(input: &str) -> Option<String> {
    let hook: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = hook["hook_event_name"].as_str().unwrap_or("");
    Some(match event {
        "UserPromptSubmit" => {
            let prompt = hook["prompt"].as_str().unwrap_or("");
            format!("[user] {}", truncate(prompt, 80))
        }
        "Stop" => {
            let msg = hook["last_assistant_message"].as_str().unwrap_or("");
            if msg.is_empty() {
                "[stopped]".to_string()
            } else {
                format!("[done]\n{}", truncate(msg, 150))
            }
        }
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
        _ => return None,
    })
}

fn format_codex_message(input: &str) -> Option<String> {
    let hook: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = hook["hook_event_name"].as_str().unwrap_or("");
    Some(match event {
        "UserPromptSubmit" => {
            let prompt = hook["prompt"].as_str().unwrap_or("");
            format!("[user] {}", truncate(prompt, 80))
        }
        "Stop" => {
            let msg = hook["last_assistant_message"].as_str().unwrap_or("");
            if msg.is_empty() {
                "[stopped]".to_string()
            } else {
                format!("[done]\n{}", truncate(msg, 150))
            }
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
        "PermissionRequest" => {
            let tool = hook["tool_name"].as_str().unwrap_or("");
            format!("[perm] {}", tool)
        }
        _ => return None,
    })
}

// ===== Parsing Utilities =====

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

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
                    let hex1 = chars.next().unwrap_or('0');
                    let hex2 = chars.next().unwrap_or('0');
                    if let Some(byte) = u8::from_str_radix(&format!("{}{}", hex1, hex2), 16).ok() {
                        result.push(byte as char);
                    }
                }
                Some(other) => {
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

    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let content = &trimmed[1..trimmed.len() - 1];
        let unescaped = unescape_string(content);
        return serde_json::json!({
            "type": "text",
            "value": unescaped,
            "raw": input,
        });
    }

    let valid_keys = [
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
        "ctrl",
        "shift",
        "alt",
        "option",
        "gui",
        "win",
        "meta",
        "cmd",
        "command",
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

    if trimmed.len() == 1 && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return serde_json::json!({
            "type": "combo",
            "modifiers": [],
            "key": trimmed,
            "raw": input,
        });
    }

    let key_lower = trimmed.to_lowercase();
    if valid_keys.contains(&key_lower.as_str()) {
        return serde_json::json!({
            "type": "combo",
            "modifiers": [],
            "key": trimmed.to_uppercase(),
            "raw": input,
        });
    }

    if trimmed.contains('+') {
        let parts: Vec<&str> = trimmed.split('+').map(|p| p.trim()).collect();
        let all_mods_valid = parts[..parts.len() - 1]
            .iter()
            .all(|p| valid_modifiers.contains(&p.to_lowercase().as_str()));

        if all_mods_valid {
            let last_part = parts.last().unwrap();
            let last_lower = last_part.to_lowercase();
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

    serde_json::json!({
        "type": "text",
        "value": trimmed,
        "raw": input,
    })
}

// ===== Main =====

fn main() {
    env_logger::init();
    let cli = Cli::parse();
    let port = get_port();

    // Handle stop
    if matches!(cli.command, Command::Stop) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let url = format!("http://127.0.0.1:{}/shutdown", port);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .no_proxy()
                .build();

            if let Ok(client) = client {
                match client.get(&url).send().await {
                    Ok(_) => println!("Server stopped"),
                    Err(_) => eprintln!("Server not running"),
                }
            } else {
                eprintln!("Server not running");
            }
        });
        return;
    }

    // Handle start
    if matches!(cli.command, Command::Start) {
        if check_server(port) {
            println!("vibekeys server already running on port {}", port);
            return;
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run_server(port, None));
        return;
    }

    // Other commands: check if server is already running, if not start it

    let server_running = check_server(port);

    if server_running {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(forward_command(port, &cli.command));
        return;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run_server(port, Some(cli.command)));
}
