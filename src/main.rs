use arboard::Clipboard;

use axum::{
    extract::State,
    routing::{get, post},
    Router,
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
const KEYMAP_ASR_CONFIG_ID: Uuid = Uuid::from_u128(0xfaf9e22c_e8fc_421b_afef_8b5236813fb1);
const WIFI_SSID_ID: Uuid = Uuid::from_u128(0x1fda4d6e_2f14_42b0_96fa_453bed238375);
const WIFI_PASS_ID: Uuid = Uuid::from_u128(0xa987ab18_a940_421a_a1d7_b94ee22bccbe);

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

enum BleCmd {
    Send {
        char_uuid: Uuid,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    WifiConfig {
        ssid: String,
        pass: Option<String>,
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
            Some(SelectResult::BleCmd(BleCmd::WifiConfig { ssid, pass, reply })) => {
                // Check connection before sending with 1s timeout
                let connected = check_connected(&peripheral).await;

                if !connected {
                    log::error!("BLE disconnected before send, exiting");
                    let _ = reply.send(Err("BLE disconnected".to_string()));
                    return;
                }

                // Send SSID first
                let ssid_result = send_ble(&peripheral, WIFI_SSID_ID, ssid.as_bytes()).await;
                if ssid_result.is_err() {
                    let _ = reply.send(ssid_result.map_err(|e| e.to_string()));
                    continue;
                }

                // Then send password if provided
                let result = if let Some(password) = pass {
                    send_ble(&peripheral, WIFI_PASS_ID, password.as_bytes()).await
                } else {
                    Ok(())
                };
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
    let result = ble_send(&state.ble_tx, KEYMAP_ASR_CONFIG_ID, body.as_bytes()).await;
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
    // Parse JSON with ssid and pass
    if let Ok(config) = serde_json::from_str::<serde_json::Value>(&body) {
        let ssid = config["ssid"].as_str();
        let pass = config["pass"].as_str();

        let mut results = Vec::new();

        // Send SSID first
        if let Some(s) = ssid {
            let result = ble_send(&state.ble_tx, WIFI_SSID_ID, s.as_bytes()).await;
            let r_string = match result {
                Ok(result) => result,
                Err(e) => {
                    log::error!("BLE disconnected: {}", e);
                    state.shutdown_tx.notify_waiters();
                    format!("error: {}\n", e)
                }
            };
            results.push(r_string);
        }

        // Then send password
        if let Some(p) = pass {
            let result = ble_send(&state.ble_tx, WIFI_PASS_ID, p.as_bytes()).await;
            let r_string = match result {
                Ok(result) => result,
                Err(e) => {
                    log::error!("BLE disconnected: {}", e);
                    state.shutdown_tx.notify_waiters();
                    format!("error: {}\n", e)
                }
            };
            results.push(r_string);
        }

        results.join(",")
    } else {
        "error: invalid JSON format, expected {\"ssid\": \"...\", \"pass\": \"...\"}\n".to_string()
    }
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
        .route("/asr-config", post(asr_config_handler))
        .route("/wifi-config", post(wifi_config_handler))
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

async fn send_wifi_config(port: u16, ssid: &str, pass: Option<&str>) {
    let url = format!("http://127.0.0.1:{}/wifi-config", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build();

    let config = serde_json::json!({
        "ssid": ssid,
        "pass": pass,
    });

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
        // Profiles are expanded into individual Keymap commands before reaching here.
        Command::Profile { .. } => None,
        Command::AsrConfig {
            clean,
            platform,
            uri,
            api_key,
            model,
        } => {
            if clean {
                let (tx, _) = oneshot::channel();
                Some(BleCmd::Send {
                    char_uuid: KEYMAP_ASR_CONFIG_ID,
                    data: vec![],
                    reply: tx,
                })
            } else if let Some(plat) = platform {
                let config =
                    build_asr_config(&plat, uri.as_deref(), api_key.as_deref(), model.as_deref());
                let (tx, _) = oneshot::channel();
                Some(BleCmd::Send {
                    char_uuid: KEYMAP_ASR_CONFIG_ID,
                    data: config.into_bytes(),
                    reply: tx,
                })
            } else {
                None // Interactive mode handled in main()
            }
        }
        Command::WifiConfig { ssid, pass } => {
            if let Some(s) = ssid {
                let (tx, _) = oneshot::channel();
                Some(BleCmd::WifiConfig {
                    ssid: s,
                    pass,
                    reply: tx,
                })
            } else {
                None // Interactive mode handled in main()
            }
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Hook input: {}", input);
            format_claude_message(&input).map(|msg| {
                log::info!("Hook formatted: {}", msg);
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
            log::debug!("Codex hook input: {}", input);
            format_codex_message(&input).map(|msg| {
                log::info!("Codex hook formatted: {}", msg);
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
            for (key, binding) in keymaps {
                let config = build_keymap_config(&key, &binding);
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
        Command::WifiConfig { ssid, pass } => {
            if let Some(s) = ssid {
                send_wifi_config(port, &s, pass.as_deref()).await;
            }
            // If ssid is None, interactive mode is handled in main()
        }
        Command::Claude | Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Hook input: {}", input);
            if let Some(msg) = format_claude_message(&input) {
                log::info!("Hook formatted: {}", msg);
                let _ = send_command(port, &msg).await;
            }
        }
        Command::Codex => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            log::debug!("Codex hook input: {}", input);
            if let Some(msg) = format_codex_message(&input) {
                log::info!("Codex hook formatted: {}", msg);
                let _ = send_command(port, &msg).await;
            }
        }
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

/// Interactive WiFi configuration
fn interactive_wifi_config() -> anyhow::Result<(String, Option<String>)> {
    let theme = ColorfulTheme::default();

    // SSID
    let ssid: String = Input::with_theme(&theme)
        .with_prompt("WiFi SSID")
        .allow_empty(false)
        .interact()?;

    // Password
    let pass = Password::with_theme(&theme)
        .with_prompt("WiFi Password")
        .allow_empty_password(true)
        .interact()?;
    let pass = if pass.is_empty() { None } else { Some(pass) };

    Ok((ssid, pass))
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

    // Handle interactive WiFi config
    if matches!(cli.command, Command::WifiConfig { ssid: None, .. }) {
        match interactive_wifi_config() {
            Ok((ssid, pass)) => {
                if check_server(port).await {
                    send_wifi_config(port, &ssid, pass.as_deref()).await;
                } else {
                    run_server(
                        port,
                        vec![Command::WifiConfig {
                            ssid: Some(ssid),
                            pass,
                        }],
                    )
                    .await;
                }
            }
            Err(e) => {
                log::error!("WiFi config failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Other commands: check if server is already running, if not start it
    if check_server(port).await {
        forward_command(port, &cli.command).await;
    } else {
        // Expand a profile into its individual keymap commands so they all get
        // applied when the server boots.
        let initial_cmds = match cli.command {
            Command::Profile { name } => match profile_keymaps(&name) {
                Some(keymaps) => {
                    let mut cmds: Vec<Command> = keymaps
                        .into_iter()
                        .map(|(key, binding)| Command::Keymap { key, binding })
                        .collect();
                    // Mirror the hot path: after the keymaps, show the confirmation
                    // on the keyboard display once the device connects.
                    cmds.push(Command::Send {
                        message: profile_message(&name),
                    });
                    cmds
                }
                None => {
                    log::error!(
                        "Unknown profile: '{}'. Available profiles: claude, codex",
                        name
                    );
                    std::process::exit(1);
                }
            },
            other => vec![other],
        };
        run_server(port, initial_cmds).await;
    }
}
