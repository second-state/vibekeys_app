use axum::{extract::State, routing::get, routing::post, Json, Router};
use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter, WriteType};
use btleplug::platform::Manager;
use btleplug::platform::Peripheral as PlatformPeripheral;
use clap::Parser;
use log::{info, warn, error, debug};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use uuid::Uuid;

/// BLE Controller HTTP Server
#[derive(Parser, Debug)]
#[command(version, long_about = None)]
struct Args {
    /// HTTP server host address
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// HTTP server port
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

// Controller Service UUID
const CONTROLLER_SERVICE_ID: Uuid = Uuid::from_u128(0x9c80ffb6_affa_4083_944a_91e34c88bd76);

// Keyboard Display Characteristic UUID
const KEYBOARD_DISPLAY_ID: Uuid = Uuid::from_u128(0xcdaa6472_67a8_4241_93cf_145051608573);

#[derive(Clone)]
struct AppState {
    peripheral: Arc<tokio::sync::Mutex<Option<PlatformPeripheral>>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Working,
    Stop,
    Waiting,
}

#[derive(Deserialize)]
struct StatusRequest {
    status: Status,
}

#[derive(Deserialize)]
struct SendMessageRequest {
    message: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    env_logger::init();

    let state = AppState {
        peripheral: Arc::new(tokio::sync::Mutex::new(None)),
    };

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters
        .into_iter()
        .next()
        .expect("No Bluetooth adapter found");

    info!("Using adapter: {:?}", adapter.adapter_info().await);
    info!("Scanning for BLE devices...");

    let mut filter = ScanFilter::default();
    filter.services.push(CONTROLLER_SERVICE_ID);

    adapter.start_scan(filter.clone()).await?;
    time::sleep(Duration::from_secs(5)).await;

    let peripherals = adapter.peripherals().await?;
    info!("Found {} devices", peripherals.len());

    let target = loop {
        if let Some(p) = find_and_print_peripherals(&peripherals, CONTROLLER_SERVICE_ID).await? {
            adapter.stop_scan().await?;
            break p;
        }
        adapter.stop_scan().await?;
        warn!("Target device not found, retrying in 5s...");
        time::sleep(Duration::from_secs(5)).await;
        adapter.start_scan(filter.clone()).await?;
        time::sleep(Duration::from_secs(5)).await;
    };

    info!("Connecting to target device...");
    connect_and_discover(&target).await?;

    {
        let mut peripheral = state.peripheral.lock().await;
        *peripheral = Some(target);
    }

    info!("Device ready");

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = ble_monitor_task(state_clone).await {
            error!("BLE monitor error: {}", e);
        }
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/send", get(send_message_handler))
        .route("/send", post(send_message_post))
        .route("/status", post(status_handler))
        .with_state(state);

    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "BLE Controller Service\n"
}

async fn send_message_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    send_to_peripheral(&state, "Hello from HTTP GET!").await
}

async fn send_message_post(
    State(state): State<AppState>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    send_to_peripheral(&state, &req.message).await
}

async fn send_to_peripheral(
    state: &AppState,
    message: &str,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let peripheral = state.peripheral.lock().await;
    if let Some(ref p) = *peripheral {
        if let Err(e) = send_message(p, KEYBOARD_DISPLAY_ID, message).await {
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
        Ok(Json(
            serde_json::json!({ "status": "ok", "message": message }),
        ))
    } else {
        Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "No BLE device connected".to_string(),
        ))
    }
}

async fn status_handler(
    State(state): State<AppState>,
    Json(req): Json<StatusRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let message = match req.status {
        Status::Working => "working",
        Status::Stop => "stop",
        Status::Waiting => "waiting",
    };
    send_to_peripheral(&state, message).await
}

// BLE monitor task: watch for disconnect and reconnect
async fn ble_monitor_task(state: AppState) -> anyhow::Result<()> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters
        .into_iter()
        .next()
        .expect("No Bluetooth adapter found");

    let mut interval = tokio::time::interval(Duration::from_secs(2));

    loop {
        interval.tick().await;

        let peripheral = state.peripheral.lock().await;
        if let Some(ref p) = *peripheral {
            match p.is_connected().await {
                Ok(true) => {
                    // Still connected, do nothing
                }
                _ => {
                    warn!("Device disconnected!");
                    drop(peripheral);
                    *state.peripheral.lock().await = None;

                    loop {
                        info!("Scanning for devices...");
                        adapter.start_scan(ScanFilter::default()).await?;
                        time::sleep(Duration::from_secs(5)).await;

                        let peripherals = adapter.peripherals().await?;
                        if let Some(target) =
                            find_and_print_peripherals(&peripherals, CONTROLLER_SERVICE_ID).await?
                        {
                            adapter.stop_scan().await?;
                            if let Err(e) = connect_and_discover(&target).await {
                                warn!("Reconnect failed: {}, retrying...", e);
                                continue;
                            }
                            {
                                let mut p = state.peripheral.lock().await;
                                *p = Some(target);
                            }
                            info!("Reconnected successfully");
                            break;
                        }
                        adapter.stop_scan().await?;
                        time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

// Find and list peripherals with target service
async fn find_and_print_peripherals(
    peripherals: &[PlatformPeripheral],
    target_service: Uuid,
) -> anyhow::Result<Option<PlatformPeripheral>> {
    let mut result = None;

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
                if result.is_none() {
                    result = Some(peripheral.clone());
                }
            }

            debug!("----------------------------");
        }
    }

    Ok(result)
}

// Connect to device and discover services
async fn connect_and_discover(peripheral: &PlatformPeripheral) -> anyhow::Result<()> {
    let addr = peripheral.address();
    info!("Connecting to {}...", addr);

    peripheral.connect().await?;
    time::sleep(Duration::from_secs(1)).await;

    info!("Discovering services...");
    peripheral.discover_services().await?;
    time::sleep(Duration::from_secs(1)).await;

    let characteristics = peripheral.characteristics();
    info!("Found {} characteristics", characteristics.len());

    // Send "Connected" message after successful connection
    let _ = send_message(peripheral, KEYBOARD_DISPLAY_ID, "Connected").await;

    Ok(())
}

// Send message to characteristic
async fn send_message(
    peripheral: &PlatformPeripheral,
    char_uuid: Uuid,
    message: &str,
) -> anyhow::Result<()> {
    let characteristics = peripheral.characteristics();

    for char in characteristics {
        if char.uuid == char_uuid {
            info!("Found target characteristic: {}", char.uuid);
            info!("Sending data: {}", message);

            peripheral
                .write(&char, message.as_bytes(), WriteType::WithResponse)
                .await?;

            info!("Data sent successfully");
            return Ok(());
        }
    }

    warn!("Characteristic not found: {}", char_uuid);
    Ok(())
}
