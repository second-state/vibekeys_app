use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Read, Write as IoWrite};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, oneshot};

const APP_NAME: &str = "vibekeys";
const DEFAULT_PORT: u16 = 42837;

// TODO: uncomment when bluetooth is available
// use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter, WriteType};
// use btleplug::platform::{Adapter, Manager, Peripheral as PlatformPeripheral};
// use uuid::Uuid;
// use tokio::sync::mpsc;
// use anyhow;
//
// const CONTROLLER_SERVICE_ID: Uuid = Uuid::from_u128(0x623fa3e2_631b_4f8f_a6e7_a7b09c03e7e0);
// const KEYBOARD_DISPLAY_ID: Uuid = Uuid::from_u128(0xcdaa6472_67a8_4241_93cf_145051608573);
// const KEYMAP_CONFIG_ID: Uuid = Uuid::from_u128(0x6f2a291c_0e4d_4f0f_9446_50bcd0b73bb0);

// ===== CLI =====

#[derive(Parser, Debug)]
#[command(name = "vibekeys", version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Send a message to the connected device
    Send { message: String },
    /// Configure key mapping
    Keymap { key: String, binding: String },
    /// Read hook JSON from stdin and forward to device
    Hook,
    /// Stop the running server
    Stop,
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
    if let Ok(mut f) =
        fs::OpenOptions::new().create(true).append(true).open("/tmp/vibekeys.log")
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

// ===== HTTP Client (minimal, localhost only) =====

async fn http_request(
    method: &str,
    port: u16,
    path: &str,
    body: Option<&[u8]>,
) -> Option<String> {
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .ok()?;
    let header = match body {
        Some(b) => format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            method, path, port, b.len()
        ),
        None => format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            method, path, port
        ),
    };
    stream.write_all(header.as_bytes()).await.ok()?;
    if let Some(b) = body {
        stream.write_all(b).await.ok()?;
    }
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.ok()?;
    let s = String::from_utf8_lossy(&resp);
    s.split("\r\n\r\n").nth(1).map(|b| b.to_string())
}

async fn check_server(port: u16) -> bool {
    http_request("GET", port, "/health", None)
        .await
        .map(|s| s.trim() == "ok")
        .unwrap_or(false)
}

// ===== Daemon (Unix) =====

#[cfg(unix)]
fn do_daemonize() -> Result<(), Box<dyn std::error::Error>> {
    use daemonize::Daemonize;
    Daemonize::new().start()?;
    Ok(())
}

// ===== Axum HTTP Server =====

struct AppState {
    // ble_tx: mpsc::Sender<BleCmd>,  // TODO: uncomment
    counter: AtomicU64,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn root_handler(State(state): State<Arc<AppState>>) -> String {
    let count = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
    format!("vibekeys server running\nRequests: {}\n", count)
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
    state.counter.fetch_add(1, Ordering::Relaxed);
    log_message(&format!("send: {}", body));
    "ok\n".to_string()
}

async fn keymap_handler(State(state): State<Arc<AppState>>, body: String) -> String {
    state.counter.fetch_add(1, Ordering::Relaxed);
    log_message(&format!("keymap: {}", body));
    "ok\n".to_string()
}

async fn run_server(port: u16) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let state = Arc::new(AppState {
        counter: AtomicU64::new(0),
        shutdown_tx: Mutex::new(Some(shutdown_tx)),
    });

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

async fn forward_command(port: u16, cmd: &Command) {
    match cmd {
        Command::Send { message } => {
            match http_request("POST", port, "/send", Some(message.as_bytes())).await {
                Some(r) => print!("{}", r),
                None => eprintln!("Failed to connect to server"),
            }
        }
        Command::Keymap { key, binding } => {
            let config = build_keymap_config(key, binding);
            match http_request("POST", port, "/keymap", Some(config.as_bytes())).await {
                Some(r) => print!("{}", r),
                None => eprintln!("Failed to connect to server"),
            }
        }
        Command::Hook => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).ok();
            if let Some(msg) = format_hook_message(&input) {
                http_request("POST", port, "/send", Some(msg.as_bytes())).await;
            }
        }
        Command::Stop => unreachable!(),
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

fn format_hook_message(input: &str) -> Option<String> {
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
        "enter", "return", "space", "tab", "escape", "esc", "backspace", "delete", "insert",
        "home", "end", "pageup", "pagedown", "up", "down", "left", "right", "ctrl", "shift",
        "alt", "option", "gui", "win", "meta", "cmd", "command", "f1", "f2", "f3", "f4", "f5",
        "f6", "f7", "f8", "f9", "f10", "f11", "f12", "plus", "minus", "equal", "semicolon",
        "quote", "backquote", "backslash", "comma", "period", "slash", "bracketleft",
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
                    && last_part.chars().next().map_or(false, |c| c.is_alphanumeric()));

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
    if matches!(&cli.command, Some(Command::Stop)) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            match http_request("GET", port, "/shutdown", None).await {
                Some(_) => println!("Server stopped"),
                None => eprintln!("Server not running"),
            }
        });
        return;
    }

    // Check if server is already running
    let should_start = {
        let rt = tokio::runtime::Runtime::new().unwrap();
        if rt.block_on(check_server(port)) {
            if let Some(cmd) = &cli.command {
                rt.block_on(forward_command(port, cmd));
            } else {
                println!("vibekeys server running on port {}", port);
            }
            false
        } else {
            true
        }
    };

    if !should_start {
        return;
    }

    // Daemonize (Unix only, must be before creating tokio runtime)
    #[cfg(unix)]
    {
        if let Err(e) = do_daemonize() {
            eprintln!("Failed to daemonize: {}", e);
            std::process::exit(1);
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run_server(port));
}
