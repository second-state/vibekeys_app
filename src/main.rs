use arboard::Clipboard;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input, Password, Select};
use std::io::{self, Read};
use std::sync::Arc;
use tokio::sync::oneshot;

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
/// 统一配置特征值(新固件):读取返回整份快照,写入接收部分对象 patch。详见 docs/ble-config.md。
const CONFIG_ID: Uuid = Uuid::from_u128(0xcef520a9_bcb5_4fc6_87f7_82804eee2b20);

/// wifi_list 最多条数,与固件 MAX_WIFI_CREDS 一致。
const MAX_WIFI_CREDS: usize = 8;

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
    /// Apply a predefined keymap profile (e.g. `codex`)
    Profile { name: String },
    /// Read Claude Code hook JSON from stdin and forward to device
    Claude,
    /// Alias for 'claude' - reads Claude Code hook JSON from stdin and forwards to device
    Hook,
    /// Read Codex hook JSON from stdin and forward to device
    Codex,
    /// Stop the running server
    Stop,
    /// Configure ASR settings
    AsrConfig {
        /// Clear ASR configuration (send empty string)
        #[arg(long)]
        clean: bool,
        /// Platform (e.g., whisper) - omit for interactive mode
        platform: Option<String>,
        /// API URI
        #[arg(long)]
        uri: Option<String>,
        /// API key
        #[arg(long)]
        api_key: Option<String>,
        /// Model name
        #[arg(long)]
        model: Option<String>,
    },
    /// Configure WiFi settings
    WifiConfig {
        /// WiFi SSID - omit for interactive mode
        ssid: Option<String>,
        /// WiFi password
        #[arg(long)]
        pass: Option<String>,
    },
    /// Configure mic mode (ptt = push-to-talk, toggle = tap to start/stop)
    MicModel {
        /// "ptt" or "toggle" - omit for interactive mode
        mode: Option<String>,
    },
    /// Whether to prefer the built-in ASR in keyboard mode
    PreferBuiltinAsr {
        /// "on" or "off" - omit for interactive mode
        value: Option<String>,
    },
    /// Configure the server URL
    ServerUrl {
        /// server URL - omit for interactive mode
        url: Option<String>,
    },
    /// Send a plain text notification to the keyboard display
    Notify { message: String },
    /// Send one multi-session status event (sid + project + status, no text)
    Session {
        /// Short session id (first 8 chars of the real session id)
        sid: String,
        /// Status: work | tool | post | perm | note | done | err | end
        status: String,
    },
}

// ===== Port =====

fn get_port() -> u16 {
    std::env::var("VIBEKEYS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

// ===== HTTP Client =====

async fn check_server(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
        .no_proxy()
        .build();

    if let Ok(client) = client {
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(text) = resp.text().await {
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
            log::info!("Subscribed to ASR result notifications");
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

async fn read_ble(p: &PlatformPeripheral, char_uuid: Uuid) -> anyhow::Result<Vec<u8>> {
    for c in &p.characteristics() {
        if c.uuid == char_uuid {
            return Ok(p.read(c).await?);
        }
    }
    Err(anyhow::anyhow!("Characteristic {} not found", char_uuid))
}

/// 把单个配置字段包成 CONFIG 部分对象 patch,例如 `config_patch("asr_config", v)` →
/// `{"asr_config": v}`。设备只更新出现的字段。
fn config_patch(field: &str, value: serde_json::Value) -> Vec<u8> {
    serde_json::json!({ field: value }).to_string().into_bytes()
}

/// 从 CONFIG 快照 JSON 文本解析出 wifi_list(顺序即优先级)。
fn parse_wifi_list(snapshot_json: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(snapshot_json)
        .ok()
        .and_then(|v| {
            v.get("wifi_list").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .map(|c| {
                        (
                            c.get("ssid")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            c.get("pass")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        )
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// 把 (ssid, pass) 列表序列化成固件 wifi_list JSON。
fn wifi_list_to_json(list: &[(String, String)]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = list
        .iter()
        .map(|(s, p)| serde_json::json!({"ssid": s, "pass": p}))
        .collect();
    serde_json::Value::Array(arr)
}

/// 规整来自客户端的 wifi_list:丢弃空 ssid,截断到 MAX_WIFI_CREDS。
fn sanitize_wifi_list(list: &[serde_json::Value]) -> Vec<serde_json::Value> {
    list.iter()
        .filter_map(|c| {
            let ssid = c.get("ssid").and_then(|v| v.as_str())?;
            if ssid.is_empty() {
                return None;
            }
            let pass = c.get("pass").and_then(|v| v.as_str()).unwrap_or("");
            Some(serde_json::json!({"ssid": ssid, "pass": pass}))
        })
        .take(MAX_WIFI_CREDS)
        .collect()
}

/// 经 server 的 BLE 任务读 CONFIG 快照里的 wifi_list。
async fn read_wifi_list(ble_tx: &mpsc::Sender<BleCmd>) -> anyhow::Result<Vec<(String, String)>> {
    let data = ble_read(ble_tx, CONFIG_ID).await?;
    Ok(parse_wifi_list(&String::from_utf8_lossy(&data)))
}

enum BleCmd {
    Send {
        char_uuid: Uuid,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Read {
        char_uuid: Uuid,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
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
    const PASTE_CODE: u8 = 2;

    if let Err(e) = peripheral
        .write(asr_char, &[PASTE_CODE], WriteType::WithResponse)
        .await
    {
        log::error!("Failed to write ASR acknowledge: {}", e);
    } else {
        log::info!("ASR acknowledge sent");
    }
}

/// Set text to clipboard
fn set_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = Clipboard::new() {
        if let Err(e) = clipboard.set_text(text) {
            log::error!("Failed to set clipboard: {}", e);
        } else {
            log::info!("Text set to clipboard: {}", text);
        }
    } else {
        log::error!("Failed to access clipboard");
    }
}

/// Handle ASR result notifications: set to clipboard and acknowledge with 1u8
async fn handle_asr_notifications(
    peripheral: &PlatformPeripheral,
) -> Option<(btleplug::api::Characteristic, String)> {
    log::info!("ASR notification handler started");

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

async fn loop_check_connection(peripheral: &PlatformPeripheral) {
    loop {
        if !check_connected(peripheral).await {
            log::info!("BLE disconnected, exiting BLE task");
            return;
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
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
        _ = loop_check_connection(peripheral) => {
            None
        }
    }
}

async fn check_connected(peripheral: &PlatformPeripheral) -> bool {
    let r = tokio::time::timeout(std::time::Duration::from_secs(1), peripheral.is_connected())
        .await
        .unwrap_or(Ok(false));

    match r {
        Ok(connected) => connected,
        Err(_) => {
            log::warn!("Connection check timed out");
            false
        }
    }
}

async fn ble_task(mut rx: mpsc::Receiver<BleCmd>) {
    // Initial connection with timeout
    log::info!("Scanning for BLE device...");
    let peripheral = match try_ble_connect().await {
        Ok(p) => {
            log::info!("BLE device connected");
            p
        }
        Err(e) => {
            log::error!("BLE connection failed: {}", e);
            return;
        }
    };

    loop {
        match select_rx_and_notify(&mut rx, &peripheral).await {
            Some(SelectResult::BleCmd(BleCmd::Send {
                char_uuid,
                data,
                reply,
            })) => {
                // Check connection before sending with 1s timeout
                log::info!("check connect status");
                let connected = check_connected(&peripheral).await;

                if !connected {
                    log::error!("BLE disconnected before send, exiting");
                    let _ = reply.send(Err("BLE disconnected".to_string()));
                    return;
                }

                log::info!("start send");
                let result = send_ble(&peripheral, char_uuid, &data).await;
                log::info!("send end");
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Some(SelectResult::BleCmd(BleCmd::Read { char_uuid, reply })) => {
                let connected = check_connected(&peripheral).await;
                if !connected {
                    log::error!("BLE disconnected before read, exiting");
                    let _ = reply.send(Err("BLE disconnected".to_string()));
                    return;
                }
                let result = read_ble(&peripheral, char_uuid).await;
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Some(SelectResult::AsrResult(asr_char, text)) => {
                log::info!("ASR result received: {}", text);
                set_to_clipboard(&text);
                write_asr_acknowledge(&peripheral, &asr_char).await;
            }
            None => return,
        }
    }
}

async fn try_ble_connect() -> anyhow::Result<PlatformPeripheral> {
    let (_manager, adapter) = get_adapter().await?;
    let p = scan_and_find_peripheral(&adapter).await?;
    connect_and_discover(&p).await?;
    Ok(p)
}

async fn ble_send(
    ble_tx: &mpsc::Sender<BleCmd>,
    char_uuid: Uuid,
    data: &[u8],
) -> anyhow::Result<String> {
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
        anyhow::bail!("Failed to send BLE command");
    }
    match rx.await {
        Ok(Ok(())) => Ok("ok\n".to_string()),
        Ok(Err(e)) => Err(anyhow::anyhow!("error: {}", e)),
        Err(_) => Err(anyhow::anyhow!("error: no response")),
    }
}

/// 经 server 的 BLE 任务读一个特征值(用于读 CONFIG 快照)。
async fn ble_read(ble_tx: &mpsc::Sender<BleCmd>, char_uuid: Uuid) -> anyhow::Result<Vec<u8>> {
    let (tx, rx) = oneshot::channel();
    if ble_tx
        .send(BleCmd::Read {
            char_uuid,
            reply: tx,
        })
        .await
        .is_err()
    {
        anyhow::bail!("Failed to send BLE read command");
    }
    match rx.await {
        Ok(Ok(data)) => Ok(data),
        Ok(Err(e)) => Err(anyhow::anyhow!("error: {}", e)),
        Err(_) => Err(anyhow::anyhow!("error: no response")),
    }
}

// ===== Axum HTTP Server =====

struct AppState {
    ble_tx: mpsc::Sender<BleCmd>,
    shutdown_tx: Arc<tokio::sync::Notify>,
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn root_handler(State(_): State<Arc<AppState>>) -> String {
    format!("vibekeys server running")
}

async fn shutdown_handler(State(state): State<Arc<AppState>>) -> String {
    log::info!("Shutdown requested");
    state.shutdown_tx.notify_waiters();
    "shutting down\n".to_string()
}

/// 多会话状态事件,发到 KEYBOARD_DISPLAY 特性。设备端解析 JSON 后按 `type`
/// 分流:`session` 走多会话表,其他内容按纯文本上屏。协议见 docs/session-events.md。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionEvent {
    #[serde(rename = "type")]
    kind: String,
    ver: u8,
    /// session-id 前 8 位(键盘上显示用的短码)
    sid: String,
    /// workspace 路径最后一段(项目名)
    proj: String,
    /// 状态:work | tool | post | perm | note | done | err | end
    st: String,
}

impl SessionEvent {
    fn new(sid: &str, proj: &str, st: &str) -> Self {
        Self {
            kind: "session".to_string(),
            ver: 1,
            sid: session_short_id(sid).to_string(),
            proj: proj.to_string(),
            st: st.to_string(),
        }
    }

    fn to_payload(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

fn json_payload(ev: &SessionEvent) -> String {
    serde_json::to_string(ev).unwrap_or_default()
}

/// 构造一条手工 session 事件(`session` 子命令),proj 取当前工作目录 basename。
fn session_event_cli(sid: &str, st: &str) -> anyhow::Result<SessionEvent> {
    const STATUSES: [&str; 8] = ["work", "tool", "post", "perm", "note", "done", "err", "end"];
    if !STATUSES.contains(&st) {
        anyhow::bail!(
            "Invalid status '{}'. Valid statuses: {}",
            st,
            STATUSES.join(" ")
        );
    }
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(SessionEvent::new(sid, &workspace_name(&cwd), st))
}

/// 取路径最后一段作为项目名(容忍尾部斜杠)。
fn workspace_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((_, last)) if !last.is_empty() => last,
        _ => trimmed,
    }
}

/// session-id 截前 8 位作为显示短码;不足 8 位则原样返回。
fn session_short_id(sid: &str) -> &str {
    if sid.len() > 8 {
        &sid[..8]
    } else {
        sid
    }
}

async fn send_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let result = ble_send(&state.ble_tx, KEYBOARD_DISPLAY_ID, body.as_bytes()).await;
    // If BLE disconnected, shut down the server
    match result {
        Ok(result) => result,
        Err(e) => {
            log::error!("BLE disconnected: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

/// 与 `send_handler` 相同,但请求体是 JSON 结构体(`SessionEvent`),
/// 反序列化失败返回 400。发给设备的仍是序列化后的紧凑 JSON。
async fn send_json_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionEvent>,
) -> String {
    let data = body.to_payload();
    let result = ble_send(&state.ble_tx, KEYBOARD_DISPLAY_ID, &data).await;
    // If BLE disconnected, shut down the server
    match result {
        Ok(result) => result,
        Err(e) => {
            log::error!("BLE disconnected: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

async fn keymap_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let result = ble_send(&state.ble_tx, KEYMAP_CONFIG_ID, body.as_bytes()).await;
    // If BLE disconnected, shut down the server
    match result {
        Ok(result) => result,
        Err(e) => {
            log::error!("BLE disconnected: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

async fn asr_config_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    // body 是 ASR 内层对象 {platform,uri,api_key,model};包成 CONFIG patch {"asr_config": <body>}。
    let value: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return format!("error: invalid ASR config JSON: {}\n", e),
    };
    let payload = config_patch("asr_config", value);
    let result = ble_send(&state.ble_tx, CONFIG_ID, &payload).await;
    // If BLE disconnected, shut down the server
    match result {
        Ok(result) => result,
        Err(e) => {
            log::error!("BLE disconnected: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

async fn wifi_config_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let req = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => v,
        Err(e) => return format!("error: invalid JSON: {}\n", e),
    };

    // 整包写:{"wifi_list": [{ssid, pass}, ...]}(来自 TUI)。规整后落盘。
    if let Some(list) = req.get("wifi_list").and_then(|v| v.as_array()) {
        let patched = sanitize_wifi_list(list);
        return write_config_field(&state, "wifi_list", serde_json::Value::Array(patched)).await;
    }

    // 追加单条:{"ssid": "...", "pass": "..."}。读现有 list → 去重追加 → 写回。
    let ssid = match req.get("ssid").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return "error: expected {\"wifi_list\":[...]} or {\"ssid\":\"...\",\"pass\":\"...\"}\n"
                .to_string();
        }
    };
    let pass = req
        .get("pass")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut list = read_wifi_list(&state.ble_tx).await.unwrap_or_default();
    if let Some(entry) = list.iter_mut().find(|(s, _)| s == &ssid) {
        entry.1 = pass;
    } else {
        if list.len() >= MAX_WIFI_CREDS {
            return format!("error: max {} WiFi networks reached\n", MAX_WIFI_CREDS);
        }
        list.push((ssid, pass));
    }
    write_config_field(&state, "wifi_list", wifi_list_to_json(&list)).await
}

/// 读 CONFIG 特性,返回整份快照 JSON。
async fn config_show_handler(State(state): State<Arc<AppState>>) -> String {
    match ble_read(&state.ble_tx, CONFIG_ID).await {
        Ok(data) => String::from_utf8_lossy(&data).to_string(),
        Err(e) => {
            log::error!("config read failed: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

/// 把单个 CONFIG 字段写成 patch 发下去,返回 handler 响应字符串。
async fn write_config_field(
    state: &Arc<AppState>,
    field: &str,
    value: serde_json::Value,
) -> String {
    let payload = config_patch(field, value);
    let result = ble_send(&state.ble_tx, CONFIG_ID, &payload).await;
    match result {
        Ok(r) => r,
        Err(e) => {
            log::error!("BLE disconnected: {}", e);
            state.shutdown_tx.notify_waiters();
            format!("error: {}\n", e)
        }
    }
}

async fn mic_model_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let mode = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["mode"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let m: u8 = match mode.as_str() {
        "ptt" => 0,
        "toggle" => 1,
        _ => return "error: mode must be 'ptt' or 'toggle'\n".to_string(),
    };
    write_config_field(&state, "mic_model", serde_json::json!(m)).await
}

async fn prefer_builtin_asr_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let value = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => match v["value"].as_bool() {
            Some(b) => b,
            None => return "error: value must be a boolean (true/false)\n".to_string(),
        },
        Err(e) => return format!("error: invalid JSON: {}\n", e),
    };
    write_config_field(&state, "prefer_builtin_asr", serde_json::json!(value)).await
}

async fn server_url_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    let url = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["url"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if url.is_empty() {
        return "error: url is required\n".to_string();
    }
    write_config_field(&state, "server_url", serde_json::json!(url)).await
}

async fn run_server(port: u16, initial_cmds: Vec<Command>) {
    let (ble_tx, ble_rx) = mpsc::channel(16);
    let notify = Arc::new(tokio::sync::Notify::new());

    for ble_cmd in initial_cmds.into_iter().filter_map(command_to_blecmd) {
        if ble_tx.send(ble_cmd).await.is_err() {
            log::error!("Failed to send initial command to BLE task");
        }
    }

    let state = Arc::new(AppState {
        ble_tx,
        shutdown_tx: notify.clone(),
    });

    let notify_ = notify.clone();
    tokio::spawn(async move {
        ble_task(ble_rx).await;
        log::warn!("BLE task ended, shutting down server");
        notify_.notify_waiters();
    });

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/shutdown", get(shutdown_handler))
        .route("/send", post(send_handler))
        .route("/keymap", post(keymap_handler))
        .route("/config", get(config_show_handler))
        .route("/asr-config", post(asr_config_handler))
        .route("/wifi-config", post(wifi_config_handler))
        .route("/mic-model", post(mic_model_handler))
        .route("/prefer-builtin-asr", post(prefer_builtin_asr_handler))
        .route("/server-url", post(server_url_handler))
        .route("/send-json", post(send_json_handler))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    log::info!("Listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            notify.notified_owned().await;
        })
        .await
        .unwrap();

    log::info!("Server stopped");
}

// ===== Command Forwarding =====

/// Sends text to the keyboard display. Returns the server response (e.g. "ok\n"),
/// or an Err message if the request could not be made.
async fn send_command(port: u16, message: &str) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/send", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|_| "Failed to connect to server".to_string())?;

    match client.post(&url).body(message.to_string()).send().await {
        Ok(resp) => resp
            .text()
            .await
            .map_err(|_| "Failed to read server response".to_string()),
        Err(_) => Err("Failed to connect to server".to_string()),
    }
}

/// Sends one keymap config to the server. Returns the server response (e.g. "ok\n"),
/// or an Err message if the request could not be made.
async fn send_keymap(port: u16, config: &str) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/keymap", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|_| "Failed to connect to server".to_string())?;

    match client.post(&url).body(config.to_string()).send().await {
        Ok(resp) => resp
            .text()
            .await
            .map_err(|_| "Failed to read server response".to_string()),
        Err(_) => Err("Failed to connect to server".to_string()),
    }
}

async fn send_asr_config(port: u16, config: &str) {
    let url = format!("http://127.0.0.1:{}/asr-config", port);
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
            Err(e) => log::error!("Failed to connect to server: {}", e),
        }
    } else {
        log::error!("Failed to connect to server");
    }
}

/// POST 一个 JSON body 到本地 server 的某个端点,返回响应文本。
async fn post_to_server(port: u16, path: &str, body: &str) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|_| "Failed to connect to server".to_string())?;
    client
        .post(&url)
        .body(body.to_string())
        .send()
        .await
        .map_err(|_| "Failed to connect to server".to_string())?
        .text()
        .await
        .map_err(|_| "Failed to read server response".to_string())
}

/// 把一个(已带参数的)命令经 server 转发;若 server 未运行则启动它并把命令作为 initial cmd。
async fn run_interactive(port: u16, cmd: Command) {
    if check_server(port).await {
        forward_command(port, &cmd).await;
    } else {
        run_server(port, vec![cmd]).await;
    }
}

/// 确保 server 在运行:已在跑返回 false;否则后台启动并等就绪,返回 true(调用方负责 shutdown)。
async fn ensure_server(port: u16) -> bool {
    if check_server(port).await {
        return false;
    }
    tokio::spawn(run_server(port, vec![]));
    // BLE 连接 + axum 起来需要点时间,轮询 health(~15s 上限)。
    for _ in 0..150 {
        if check_server(port).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    true
}

async fn shutdown_server(port: u16) {
    let url = format!("http://127.0.0.1:{}/shutdown", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build();
    if let Ok(client) = client {
        let _ = client.get(&url).send().await;
    }
}

/// GET /config 拿整份快照 JSON(给本地 TUI 用)。
async fn get_config_snapshot(port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/config", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .no_proxy()
        .build()
        .map_err(|_| "Failed to connect to server".to_string())?;
    client
        .get(&url)
        .send()
        .await
        .map_err(|_| "Failed to connect to server".to_string())?
        .text()
        .await
        .map_err(|_| "Failed to read server response".to_string())
}

fn command_to_blecmd(cmd: Command) -> Option<BleCmd> {
    match cmd {
        Command::Start | Command::Stop => None,
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
        // A profile is applied as a single multi-key write.
        Command::Profile { name } => match profile_keymaps(&name) {
            Some(keymaps) => {
                let config = build_keymap_configs(&keymaps);
                let (tx, _) = oneshot::channel();
                Some(BleCmd::Send {
                    char_uuid: KEYMAP_CONFIG_ID,
                    data: config.into_bytes(),
                    reply: tx,
                })
            }
            None => None,
        },
        Command::AsrConfig {
            clean,
            platform,
            uri,
            api_key,
            model,
        } => {
            // 写 CONFIG patch {"asr_config": {...}};clean 重置成空(新协议无删除语义,
            // patch 合并时空字段会覆盖现有值)。
            let asr_value = if clean {
                serde_json::json!({"platform":"whisper","uri":"","api_key":"","model":""})
            } else if let Some(plat) = platform {
                serde_json::from_str(&build_asr_config(
                    &plat,
                    uri.as_deref(),
                    api_key.as_deref(),
                    model.as_deref(),
                ))
                .ok()?
            } else {
                return None; // Interactive mode handled in main()
            };
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: CONFIG_ID,
                data: config_patch("asr_config", asr_value),
                reply: tx,
            })
        }
        Command::WifiConfig { .. } => None, // WiFi config goes through HTTP (see main)
        Command::MicModel { mode } => {
            let m: u8 = match mode.as_deref() {
                Some("ptt") => 0,
                Some("toggle") => 1,
                _ => return None, // Interactive / invalid handled elsewhere
            };
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: CONFIG_ID,
                data: config_patch("mic_model", serde_json::json!(m)),
                reply: tx,
            })
        }
        Command::PreferBuiltinAsr { value } => {
            let b: bool = match value.as_deref() {
                Some("on") => true,
                Some("off") => false,
                _ => return None,
            };
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: CONFIG_ID,
                data: config_patch("prefer_builtin_asr", serde_json::json!(b)),
                reply: tx,
            })
        }
        Command::ServerUrl { url } => {
            let u = url?;
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: CONFIG_ID,
                data: config_patch("server_url", serde_json::json!(u)),
                reply: tx,
            })
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Hook input: {}", input);
            claude_event(&input).map(|ev| {
                log::info!("Hook event: {:?}", ev);
                let (tx, _) = oneshot::channel();
                BleCmd::Send {
                    char_uuid: KEYBOARD_DISPLAY_ID,
                    data: ev.to_payload(),
                    reply: tx,
                }
            })
        }
        Command::Codex => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Codex hook input: {}", input);
            codex_event(&input).map(|ev| {
                log::info!("Codex hook event: {:?}", ev);
                let (tx, _) = oneshot::channel();
                BleCmd::Send {
                    char_uuid: KEYBOARD_DISPLAY_ID,
                    data: ev.to_payload(),
                    reply: tx,
                }
            })
        }
        Command::Notify { message } => {
            let (tx, _) = oneshot::channel();
            Some(BleCmd::Send {
                char_uuid: KEYBOARD_DISPLAY_ID,
                data: message.into_bytes(),
                reply: tx,
            })
        }
        Command::Session { sid, status } => match session_event_cli(&sid, &status) {
            Ok(ev) => {
                log::info!("Session event: {:?}", ev);
                let (tx, _) = oneshot::channel();
                Some(BleCmd::Send {
                    char_uuid: KEYBOARD_DISPLAY_ID,
                    data: ev.to_payload(),
                    reply: tx,
                })
            }
            Err(e) => {
                eprintln!("{}", e);
                None
            }
        },
    }
}

async fn forward_command(port: u16, cmd: &Command) {
    match cmd {
        Command::Start | Command::Stop => unreachable!(),
        Command::Send { message } => match send_command(port, message).await {
            Ok(resp) => print!("{}", resp),
            Err(e) => eprintln!("{}", e),
        },
        Command::Keymap { key, binding } => {
            let config = build_keymap_config(key, binding);
            match send_keymap(port, &config).await {
                Ok(resp) => print!("{}", resp),
                Err(e) => eprintln!("{}", e),
            }
        }
        Command::Profile { name } => {
            let keymaps = match profile_keymaps(name) {
                Some(k) => k,
                None => {
                    eprintln!(
                        "Unknown profile: '{}'. Available profiles: claude, codex",
                        name
                    );
                    return;
                }
            };
            // Apply the whole profile as a single multi-key write.
            let config = build_keymap_configs(&keymaps);
            match send_keymap(port, &config).await {
                // The server replies "ok\n" on success; anything else is a failure.
                Ok(resp) if resp.trim() == "ok" => {}
                Ok(resp) => {
                    eprintln!("Failed to apply '{}' profile: {}", name, resp.trim());
                    return;
                }
                Err(e) => {
                    eprintln!("Failed to apply '{}' profile: {}", name, e);
                    return;
                }
            }
            // Show the confirmation on the keyboard display, too, and print it locally.
            let message = profile_message(name);
            let _ = send_command(port, &message).await;
            println!("{}", message);
        }
        Command::AsrConfig {
            clean: true,
            platform: _,
            uri: _,
            api_key: _,
            model: _,
        } => {
            send_asr_config(port, "").await;
        }
        Command::AsrConfig {
            clean: false,
            platform,
            uri,
            api_key,
            model,
        } => {
            if let Some(plat) = platform {
                let config =
                    build_asr_config(&plat, uri.as_deref(), api_key.as_deref(), model.as_deref());
                send_asr_config(port, &config).await;
            }
            // If platform is None, interactive mode is handled in main()
        }
        Command::WifiConfig { .. } => {
            // WiFi config is handled directly in main() (it reads the existing list).
        }
        Command::MicModel { mode } => {
            if let Some(m) = mode {
                let body = serde_json::json!({"mode": m}).to_string();
                match post_to_server(port, "/mic-model", &body).await {
                    Ok(resp) => print!("{}", resp),
                    Err(e) => eprintln!("{}", e),
                }
            }
        }
        Command::PreferBuiltinAsr { value } => {
            if let Some(v) = value {
                let b = match v.as_str() {
                    "on" => true,
                    "off" => false,
                    other => {
                        eprintln!("invalid value '{}': expected on/off", other);
                        return;
                    }
                };
                let body = serde_json::json!({"value": b}).to_string();
                match post_to_server(port, "/prefer-builtin-asr", &body).await {
                    Ok(resp) => print!("{}", resp),
                    Err(e) => eprintln!("{}", e),
                }
            }
        }
        Command::ServerUrl { url } => {
            if let Some(u) = url {
                let body = serde_json::json!({"url": u}).to_string();
                match post_to_server(port, "/server-url", &body).await {
                    Ok(resp) => print!("{}", resp),
                    Err(e) => eprintln!("{}", e),
                }
            }
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Hook input: {}", input);
            if let Some(ev) = claude_event(&input) {
                log::info!("Hook event: {:?}", ev);
                let _ = post_to_server(port, "/send-json", &json_payload(&ev)).await;
            }
        }
        Command::Codex => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Codex hook input: {}", input);
            if let Some(ev) = codex_event(&input) {
                log::info!("Codex hook event: {:?}", ev);
                let _ = post_to_server(port, "/send-json", &json_payload(&ev)).await;
            }
        }
        Command::Notify { message } => match send_command(port, &message).await {
            Ok(resp) => print!("{}", resp),
            Err(e) => eprintln!("{}", e),
        },
        Command::Session { sid, status } => match session_event_cli(sid, &status) {
            Ok(ev) => match post_to_server(port, "/send-json", &json_payload(&ev)).await {
                Ok(resp) => print!("{}", resp),
                Err(e) => eprintln!("{}", e),
            },
            Err(e) => eprintln!("{}", e),
        },
    }
}

/// Returns the (key, binding) pairs for a named keymap profile.
///
/// Bindings use the same syntax as the `keymap` command: a quoted string is a
/// text macro, so `"/review\n"` types `/review` followed by Enter.
fn profile_keymaps(name: &str) -> Option<Vec<(String, String)>> {
    match name.to_lowercase().as_str() {
        // Claude Code. Restores the two keys that the codex profile overrides back to
        // their Claude defaults, so you can switch back. Other keys are left untouched.
        "claude" => Some(vec![
            ("CUSTOM".to_string(), "\"/compact\\n\"".to_string()), // /compact + Enter
            ("YOLO".to_string(), "Shift+Tab".to_string()),         // allow all edits
        ]),
        // Codex: only overrides the two keys that differ from the Claude defaults.
        // CUSTOM triggers `/review` (one-key code review), YOLO approves.
        "codex" => Some(vec![
            ("CUSTOM".to_string(), "\"/review\\n\"".to_string()),
            ("YOLO".to_string(), "\"y\"".to_string()),
        ]),
        _ => None,
    }
}

/// Friendly confirmation shown after a profile is applied.
fn profile_message(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "codex" => "✨ You're with Codex now".to_string(),
        "claude" => "✨ You're with Claude Code now".to_string(),
        other => format!("✨ Applied '{}' profile", other),
    }
}

/// Maps a user-facing key name to its physical key, applying the `YOLO` →
/// `SWITCH` alias.
fn physical_key_name(key: &str) -> String {
    let key_upper = key.to_uppercase();
    if key_upper == "YOLO" {
        "SWITCH".to_string()
    } else {
        key_upper
    }
}

fn build_keymap_config(key: &str, binding: &str) -> String {
    let parsed = parse_key_binding(binding);
    serde_json::json!({ physical_key_name(key): parsed }).to_string()
}

/// Builds a single keymap message carrying several bindings, e.g.
/// `{"CUSTOM": {...}, "SWITCH": {...}}`. The firmware merges every top-level
/// key into the existing keymap, so a whole profile is applied in one write
/// instead of one round-trip per key.
fn build_keymap_configs(keymaps: &[(String, String)]) -> String {
    let mut map = serde_json::Map::new();
    for (key, binding) in keymaps {
        map.insert(physical_key_name(key), parse_key_binding(binding));
    }
    serde_json::Value::Object(map).to_string()
}

fn build_asr_config(
    platform: &str,
    uri: Option<&str>,
    api_key: Option<&str>,
    model: Option<&str>,
) -> String {
    serde_json::json!({
        "platform": platform,
        "uri": uri,
        "api_key": api_key,
        "model": model
    })
    .to_string()
}

/// Interactive ASR configuration
fn interactive_asr_config(
) -> anyhow::Result<(String, Option<String>, Option<String>, Option<String>)> {
    let theme = ColorfulTheme::default();

    // Provider selection (matching vibetty setup)
    let providers = vec![
        (
            "openai",
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
        ),
        (
            "bytefuture",
            "https://models.bytefuture.ai/v1/audio/transcriptions",
            "groq/whisper-large-v3",
        ),
        (
            "groq",
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-vurbo",
        ),
        (
            "glm",
            "https://open.bigmodel.cn/api/paas/v4/audio/transcriptions",
            "glm-asr-2512",
        ),
        ("custom", "", ""),
    ];

    let provider_names: Vec<&str> = providers.iter().map(|(name, _, _)| *name).collect();
    let provider_index = Select::with_theme(&theme)
        .with_prompt("Select ASR provider")
        .items(&provider_names)
        .default(0)
        .interact()?;

    let (provider, default_uri, default_model) = providers[provider_index];

    // URL
    let uri = if provider == "custom" {
        let input: String = Input::with_theme(&theme)
            .with_prompt("API URL")
            .allow_empty(false)
            .interact()?;
        Some(input)
    } else {
        let input: String = Input::with_theme(&theme)
            .with_prompt("API URL")
            .default(default_uri.to_string())
            .allow_empty(true)
            .interact()?;
        if input.is_empty() {
            Some(default_uri.to_string())
        } else {
            Some(input)
        }
    };

    // API Key
    let api_key = Password::with_theme(&theme)
        .with_prompt("API Key")
        .allow_empty_password(false)
        .interact()?;
    let api_key = Some(api_key);

    // Model
    let model = if provider == "custom" {
        let input: String = Input::with_theme(&theme)
            .with_prompt("Model")
            .allow_empty(false)
            .interact()?;
        Some(input)
    } else {
        let input: String = Input::with_theme(&theme)
            .with_prompt("Model")
            .default(default_model.to_string())
            .allow_empty(true)
            .interact()?;
        if input.is_empty() {
            Some(default_model.to_string())
        } else {
            Some(input)
        }
    };

    Ok(("whisper".to_string(), uri, api_key, model))
}

/// 交互式编辑 wifi_list:展示现有网络,选择新增或删除,返回编辑后的列表。
fn interactive_wifi_config(current: &[(String, String)]) -> anyhow::Result<Vec<(String, String)>> {
    let theme = ColorfulTheme::default();

    if current.is_empty() {
        println!("No WiFi networks configured yet.");
    } else {
        println!("Current WiFi networks (priority order):");
        for (i, (s, _)) in current.iter().enumerate() {
            println!("  {}. {}", i + 1, s);
        }
    }

    let action = Select::with_theme(&theme)
        .with_prompt("Action")
        .items(&["Add a network", "Remove a network"])
        .default(0)
        .interact()?;

    let mut list: Vec<(String, String)> = current.to_vec();
    match action {
        0 => {
            if list.len() >= MAX_WIFI_CREDS {
                anyhow::bail!("already at max {} networks", MAX_WIFI_CREDS);
            }
            let ssid: String = Input::with_theme(&theme)
                .with_prompt("WiFi SSID")
                .allow_empty(false)
                .interact()?;
            let pass = Password::with_theme(&theme)
                .with_prompt("WiFi Password (empty for none)")
                .allow_empty_password(true)
                .interact()?;
            if let Some(entry) = list.iter_mut().find(|(s, _)| s == &ssid) {
                entry.1 = pass; // 已存在则更新密码
            } else {
                list.push((ssid, pass));
            }
        }
        1 => {
            if list.is_empty() {
                anyhow::bail!("no networks to remove");
            }
            let items: Vec<String> = list.iter().map(|(s, _)| s.clone()).collect();
            let idx = Select::with_theme(&theme)
                .with_prompt("Select network to remove")
                .items(&items)
                .default(0)
                .interact()?;
            list.remove(idx);
        }
        _ => {}
    }
    Ok(list)
}

fn interactive_mic_model() -> anyhow::Result<String> {
    let theme = ColorfulTheme::default();
    let items = ["toggle (tap to start/stop)", "ptt (hold to talk)"];
    let idx = Select::with_theme(&theme)
        .with_prompt("Mic mode")
        .items(&items)
        .default(0)
        .interact()?;
    Ok(if idx == 0 { "toggle" } else { "ptt" }.to_string())
}

fn interactive_prefer_builtin_asr() -> anyhow::Result<bool> {
    let theme = ColorfulTheme::default();
    let items = [
        "on (use built-in Whisper)",
        "off (pass mic through to host)",
    ];
    let idx = Select::with_theme(&theme)
        .with_prompt("Prefer built-in ASR")
        .items(&items)
        .default(0)
        .interact()?;
    Ok(idx == 0)
}

fn interactive_server_url() -> anyhow::Result<String> {
    let theme = ColorfulTheme::default();
    let url: String = Input::with_theme(&theme)
        .with_prompt("Server URL")
        .allow_empty(false)
        .interact()?;
    Ok(url)
}

/// 把 Claude Code 的 hook JSON 转成 SessionEvent。返回 None 表示该输入不产生事件
/// (非 JSON、或是不需要上屏的事件类型),调用方应静默忽略。
fn claude_event(input: &str) -> Option<SessionEvent> {
    let hook: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = hook["hook_event_name"].as_str().unwrap_or("");
    let sid = hook["session_id"].as_str().unwrap_or("");
    let proj = workspace_name(hook["cwd"].as_str().unwrap_or(""));
    let ev = |st: &str| SessionEvent::new(sid, proj, st);
    Some(match event {
        "UserPromptSubmit" => ev("work"),
        "Stop" => ev("done"),
        "Notification" => {
            // hooks.json 已用 matcher 过滤,只有这两类会进来;其余类型不上屏。
            match hook["notification_type"].as_str().unwrap_or("") {
                "permission_prompt" => ev("perm"),
                "idle_prompt" => ev("note"),
                _ => return None,
            }
        }
        "PreToolUse" => ev("tool"),
        "PostToolUse" => ev("post"),
        "StopFailure" => ev("err"),
        "SessionStart" => ev("work"),
        _ => return None,
    })
}

/// 把 Codex 的 hook JSON 转成 SessionEvent。语义与 `claude_event` 一致。
fn codex_event(input: &str) -> Option<SessionEvent> {
    let hook: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = hook["hook_event_name"].as_str().unwrap_or("");
    let sid = hook["session_id"].as_str().unwrap_or("");
    let proj = workspace_name(hook["cwd"].as_str().unwrap_or(""));
    let ev = |st: &str| SessionEvent::new(sid, proj, st);
    Some(match event {
        "UserPromptSubmit" => ev("work"),
        "Stop" => ev("done"),
        "PreToolUse" => ev("tool"),
        "PostToolUse" => ev("post"),
        "PermissionRequest" => ev("perm"),
        "SessionStart" => ev("work"),
        "SubagentStop" => ev("post"),
        _ => return None,
    })
}

// ===== Parsing Utilities =====

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

fn init_logger() {
    // Get log directory: ~/.vibekeys/logs
    let log_dir = dirs::home_dir()
        .map(|p| p.join(".vibekeys").join("logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Create log directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("Failed to create log directory {:?}: {}", log_dir, e);
        // Fall back to env_logger behavior (stderr only)
        env_logger::init();
        return;
    }

    // Initialize flexi_logger
    if let Err(e) = flexi_logger::Logger::try_with_env_or_str("info")
        .map(|logger| {
            logger
                .log_to_file(
                    flexi_logger::FileSpec::default()
                        .directory(&log_dir)
                        .basename("vibekeys")
                        .suppress_timestamp(),
                )
                .write_mode(flexi_logger::WriteMode::Direct)
                .rotate(
                    flexi_logger::Criterion::Size(10_000_000), // 10MB
                    flexi_logger::Naming::Numbers,
                    flexi_logger::Cleanup::KeepForDays(5),
                )
                .duplicate_to_stdout(flexi_logger::Duplicate::All)
                .format_for_stderr(flexi_logger::default_format)
                .format_for_files(flexi_logger::detailed_format)
        })
        .and_then(|logger| logger.start())
    {
        eprintln!("Failed to initialize logger: {}, falling back to stderr", e);
        env_logger::init();
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    init_logger();
    let cli = Cli::parse();
    let port = get_port();

    // Handle stop
    if matches!(cli.command, Command::Stop) {
        let url = format!("http://127.0.0.1:{}/shutdown", port);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .no_proxy()
            .build();

        if let Ok(client) = client {
            match client.get(&url).send().await {
                Ok(_) => log::info!("Server stopped"),
                Err(_) => log::error!("Server not running"),
            }
        } else {
            log::error!("Server not running");
        }
        return;
    }

    // Handle start
    if matches!(cli.command, Command::Start) {
        if check_server(port).await {
            log::info!("vibekeys server already running on port {}", port);
            return;
        }
        run_server(port, vec![]).await;
        return;
    }

    // Handle interactive ASR config
    if matches!(
        cli.command,
        Command::AsrConfig {
            platform: None,
            clean: false,
            ..
        }
    ) {
        match interactive_asr_config() {
            Ok((platform, uri, api_key, model)) => {
                let config = build_asr_config(
                    &platform,
                    uri.as_deref(),
                    api_key.as_deref(),
                    model.as_deref(),
                );
                if check_server(port).await {
                    send_asr_config(port, &config).await;
                } else {
                    run_server(
                        port,
                        vec![Command::AsrConfig {
                            clean: false,
                            platform: Some(platform),
                            uri,
                            api_key,
                            model,
                        }],
                    )
                    .await;
                }
            }
            Err(e) => {
                log::error!("ASR config failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Handle WiFi config — always via server HTTP (it must read the existing list first),
    // auto-starting and shutting down a temporary server if one isn't already running.
    if let Command::WifiConfig { ssid, pass } = cli.command {
        let started = ensure_server(port).await;
        let result = if let Some(s) = ssid {
            // 直接追加单条。
            let body = serde_json::json!({ "ssid": s, "pass": pass }).to_string();
            post_to_server(port, "/wifi-config", &body).await
        } else {
            // 交互式:读现有 list → TUI → 整包写回。
            match get_config_snapshot(port).await {
                Ok(text) => match interactive_wifi_config(&parse_wifi_list(&text)) {
                    Ok(new_list) => {
                        let body = serde_json::json!({ "wifi_list": wifi_list_to_json(&new_list) })
                            .to_string();
                        post_to_server(port, "/wifi-config", &body).await
                    }
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(r) => print!("{}", r),
            Err(e) => eprintln!("{}", e),
        }
        if started {
            shutdown_server(port).await;
        }
        return;
    }

    // Handle interactive mic-model
    if matches!(cli.command, Command::MicModel { mode: None }) {
        match interactive_mic_model() {
            Ok(mode) => run_interactive(port, Command::MicModel { mode: Some(mode) }).await,
            Err(e) => {
                log::error!("mic-model failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Handle interactive prefer-builtin-asr
    if matches!(cli.command, Command::PreferBuiltinAsr { value: None }) {
        match interactive_prefer_builtin_asr() {
            Ok(value) => {
                run_interactive(
                    port,
                    Command::PreferBuiltinAsr {
                        value: Some(if value { "on" } else { "off" }.to_string()),
                    },
                )
                .await
            }
            Err(e) => {
                log::error!("prefer-builtin-asr failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Handle interactive server-url
    if matches!(cli.command, Command::ServerUrl { url: None }) {
        match interactive_server_url() {
            Ok(url) => run_interactive(port, Command::ServerUrl { url: Some(url) }).await,
            Err(e) => {
                log::error!("server-url failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Other commands: check if server is already running, if not start it
    if check_server(port).await {
        forward_command(port, &cli.command).await;
    } else {
        // A profile is applied as a single multi-key write at boot (see
        // `command_to_blecmd`); follow it with the on-device confirmation,
        // mirroring the hot path. An unknown name fails fast.
        let initial_cmds = match cli.command {
            Command::Profile { name } => {
                if profile_keymaps(&name).is_none() {
                    log::error!(
                        "Unknown profile: '{}'. Available profiles: claude, codex",
                        name
                    );
                    std::process::exit(1);
                }
                vec![
                    Command::Profile { name: name.clone() },
                    Command::Send {
                        message: profile_message(&name),
                    },
                ]
            }
            other => vec![other],
        };
        run_server(port, initial_cmds).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_profile_bindings() {
        let keymaps = profile_keymaps("codex").expect("codex profile exists");
        let configs: Vec<String> = keymaps
            .iter()
            .map(|(k, b)| build_keymap_config(k, b))
            .collect();

        // CUSTOM types `/review` followed by Enter (the `\n` is unescaped to a newline).
        assert_eq!(
            configs[0],
            r#"{"CUSTOM":{"raw":"\"/review\\n\"","type":"text","value":"/review\n"}}"#
        );
        // YOLO is an alias for the physical SWITCH key; it types `y`.
        assert_eq!(
            configs[1],
            r#"{"SWITCH":{"raw":"\"y\"","type":"text","value":"y"}}"#
        );
    }

    #[test]
    fn codex_profile_is_one_multi_key_write() {
        // A profile is applied as a single message carrying every binding, not
        // one round-trip per key.
        let keymaps = profile_keymaps("codex").expect("codex profile exists");
        let config = build_keymap_configs(&keymaps);
        assert_eq!(
            config,
            r#"{"CUSTOM":{"raw":"\"/review\\n\"","type":"text","value":"/review\n"},"SWITCH":{"raw":"\"y\"","type":"text","value":"y"}}"#
        );
    }

    #[test]
    fn claude_profile_restores_codex_keys() {
        let keymaps = profile_keymaps("claude").expect("claude profile exists");
        let by_key: std::collections::HashMap<String, String> = keymaps
            .iter()
            .map(|(k, b)| (k.clone(), build_keymap_config(k, b)))
            .collect();

        // The claude profile restores exactly the two keys the codex profile overrides.
        let keys: std::collections::HashSet<&str> =
            keymaps.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["CUSTOM", "YOLO"]
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        );
        assert_eq!(
            by_key["CUSTOM"],
            r#"{"CUSTOM":{"raw":"\"/compact\\n\"","type":"text","value":"/compact\n"}}"#
        );
        assert_eq!(
            by_key["YOLO"],
            r#"{"SWITCH":{"key":"TAB","modifiers":["shift"],"raw":"Shift+Tab","type":"combo"}}"#
        );
    }

    #[test]
    fn unknown_profile_is_none() {
        assert!(profile_keymaps("nope").is_none());
    }

    #[test]
    fn profile_messages() {
        assert_eq!(profile_message("codex"), "✨ You're with Codex now");
        assert_eq!(profile_message("claude"), "✨ You're with Claude Code now");
    }

    #[test]
    fn session_short_id_takes_first_eight_chars() {
        assert_eq!(
            session_short_id("0aca72b2-9f2e-46b3-87f5-5d480aa88820"),
            "0aca72b2"
        );
        // Short ids pass through unchanged.
        assert_eq!(session_short_id("abcd1234"), "abcd1234");
        assert_eq!(session_short_id(""), "");
    }

    #[test]
    fn workspace_name_takes_last_segment() {
        assert_eq!(workspace_name("/Users/x/vibekeys_app"), "vibekeys_app");
        assert_eq!(workspace_name("/Users/x/vibekeys_app/"), "vibekeys_app");
        // Degenerate paths just yield an empty name.
        assert_eq!(workspace_name("/"), "");
        assert_eq!(workspace_name(""), "");
    }

    #[test]
    fn claude_pre_tool_use_event_carries_sid_and_project() {
        let input = r#"{
            "hook_event_name": "PreToolUse",
            "session_id": "0aca72b2-9f2e-46b3-87f5-5d480aa88820",
            "cwd": "/Users/x/vibekeys_app",
            "tool_name": "Edit"
        }"#;
        let ev = claude_event(input).expect("event");
        assert_eq!(ev.kind, "session");
        assert_eq!(ev.ver, 1);
        assert_eq!(ev.sid, "0aca72b2");
        assert_eq!(ev.proj, "vibekeys_app");
        assert_eq!(ev.st, "tool");
        // Wire format: compact JSON with the session marker first, no msg field.
        let payload = String::from_utf8(ev.to_payload()).unwrap();
        assert!(payload.starts_with(r#"{"type":"session""#));
        assert!(!payload.contains("msg"));
        assert!(!payload.contains("null"));
    }

    #[test]
    fn claude_stop_maps_to_done() {
        let input = r#"{
            "hook_event_name": "Stop",
            "session_id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "cwd": "/tmp/demo",
            "last_assistant_message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "All tests passed" }]
            }
        }"#;
        let ev = claude_event(input).expect("event");
        assert_eq!(ev.st, "done");
    }

    #[test]
    fn codex_stop_handles_null_message() {
        let input = r#"{
            "hook_event_name": "Stop",
            "session_id": "019f84955ef57b22a3c8ffd4bf90b7d4",
            "cwd": "/tmp/demo/",
            "last_assistant_message": null
        }"#;
        let ev = codex_event(input).expect("event");
        assert_eq!(ev.st, "done");
    }

    #[test]
    fn non_json_hook_input_is_ignored() {
        assert!(claude_event("not json").is_none());
        assert!(codex_event("").is_none());
        assert!(claude_event(r#"{"hook_event_name":"PreCompact"}"#).is_none());
    }

    #[test]
    fn claude_notification_types_map_to_statuses() {
        let base = |ty: &str| {
            format!(
                r#"{{"hook_event_name":"Notification","session_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","cwd":"/tmp/demo","notification_type":"{}","message":"raw message"}}"#,
                ty
            )
        };
        let ev = claude_event(&base("permission_prompt")).expect("event");
        assert_eq!(ev.st, "perm");

        let ev = claude_event(&base("idle_prompt")).expect("event");
        assert_eq!(ev.st, "note");

        // Other types are filtered out by the hooks.json matcher; even if one
        // slips through (older config), it produces no event.
        assert!(claude_event(&base("auth_success")).is_none());
    }

    #[test]
    fn session_rejects_unknown_status() {
        assert!(session_event_cli("abcd1234", "nope").is_err());
        assert!(session_event_cli("abcd1234", "end").is_ok());
    }
}
