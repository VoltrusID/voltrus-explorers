mod system;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::sync::Mutex;
use tauri::Emitter;
use tauri::Manager;
use mt_bacnet::{
    BacnetClient, BacnetConfig, BacnetObject, BacnetValue,
    discover_devices, browse_objects as browse_objects_fn,
    read_object_properties as read_object_properties_fn,
    inspect_device as inspect_device_fn,
    BacnetDiscoveredDevice, BacnetObjectInfo,
    BacnetObjectDetail, BacnetPropertyEntry, DeviceInspection,
    // COV
    build_subscribe_cov_frame, build_cancel_cov_frame, CovSubscriptionInfo,
    parse_cov_packet, CovNotification,
    // Alarms
    get_alarm_summary_blocking, get_event_information_blocking,
    acknowledge_alarm_blocking, AlarmSummaryEntry, EventInformationResponse,
    // Trend
    read_trend_log_blocking, read_trend_log_count_blocking, LogRecord, TrendLogData,
    // Schedule
    read_schedule_blocking, ScheduleData,
    // Device simulator
    SimDevice, SimPointDef,
};
use mt_protocol::ProtocolClient;

static NEXT_SUBSCRIBER_ID: AtomicU32 = AtomicU32::new(1);

struct AppState {
    client: Arc<Mutex<Option<BacnetClient>>>,
    connected_device: Arc<Mutex<Option<ConnectedDevice>>>,
    cov_subscriptions: Arc<Mutex<Vec<CovSubscriptionInfo>>>,
    cov_listener_running: Arc<AtomicBool>,
    sim_device: Arc<Mutex<Option<SimDevice>>>,
}

#[derive(Clone, serde::Serialize)]
struct SimInfo {
    device_id: u32,
    port: u16,
}

#[derive(Clone, serde::Serialize)]
struct SimWritten {
    key: String,
    value: f64,
}

// ── Device Simulator ──

#[tauri::command]
async fn sim_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    points: Vec<SimPointDef>,
    device_id: u32,
    port: u16,
) -> Result<SimInfo, String> {
    // Stop any existing simulator first.
    if let Some(prev) = state.sim_device.lock().await.take() {
        prev.stop();
    }

    let app_tick = app.clone();
    let app_write = app.clone();
    let on_tick = Box::new(move |snap: mt_bacnet::SimSnapshot| {
        let _ = app_tick.emit("sim-tick", &snap);
    });
    let on_write = Box::new(move |key: String, value: f64| {
        let _ = app_write.emit("sim-written", &SimWritten { key, value });
    });

    const VENDOR_ID: u16 = 0;
    let device = mt_bacnet::device::start(device_id, port, VENDOR_ID, points, on_tick, on_write)?;
    let info = SimInfo { device_id, port: device.port() };
    *state.sim_device.lock().await = Some(device);
    Ok(info)
}

#[tauri::command]
async fn sim_stop(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(dev) = state.sim_device.lock().await.take() {
        dev.stop();
    }
    // Close the device control panel if it's open.
    if let Some(w) = app.get_webview_window("device-panel") {
        let _ = w.close();
    }
    Ok(())
}

/// Open the device control panel as a separate OS window. Built from Rust so it
/// doesn't depend on JS `create-webview-window` capability scoping.
#[tauri::command]
async fn open_device_panel(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("device-panel") {
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::webview::WebviewWindowBuilder::new(
        &app,
        "device-panel",
        tauri::WebviewUrl::App("index.html#/sim".into()),
    )
    .title("Device Control Panel")
    .inner_size(460.0, 720.0)
    .min_inner_size(380.0, 480.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("create window: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn sim_set_value(state: tauri::State<'_, AppState>, key: String, value: f64) -> Result<(), String> {
    let guard = state.sim_device.lock().await;
    if let Some(dev) = guard.as_ref() {
        dev.set_value(&key, value);
        Ok(())
    } else {
        Err("Simulator not running".into())
    }
}

#[tauri::command]
async fn sim_set_bool(state: tauri::State<'_, AppState>, key: String, active: bool) -> Result<(), String> {
    let guard = state.sim_device.lock().await;
    if let Some(dev) = guard.as_ref() {
        dev.set_bool(&key, active);
        Ok(())
    } else {
        Err("Simulator not running".into())
    }
}

#[tauri::command]
async fn sim_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let guard = state.sim_device.lock().await;
    Ok(guard.as_ref().map(|d| d.is_running()).unwrap_or(false))
}

#[derive(Clone)]
struct ConnectedDevice {
    ip: String,
    port: u16,
    device_id: u32,
}

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>, config: BacnetConfig) -> Result<(), String> {
    let ip = config.ip.clone();
    let port = config.port;
    let device_id = config.device_id;
    let client = BacnetClient::connect(config).await.map_err(|e| e.to_string())?;
    *state.client.lock().await = Some(client);
    *state.connected_device.lock().await = Some(ConnectedDevice { ip, port, device_id });
    Ok(())
}

#[tauri::command]
async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<BacnetObject>,
) -> Result<Vec<BacnetValue>, String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: BacnetObject,
    value: BacnetValue,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    let client = guard.as_ref().ok_or("Not connected")?;
    client.write(&tag, value).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn discover(timeout_secs: u64) -> Result<Vec<BacnetDiscoveredDevice>, String> {
    discover_devices(timeout_secs).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Stop the COV listener loop first.
    state.cov_listener_running.store(false, Ordering::Relaxed);

    // Clear connected-device + COV state immediately so the UI updates, even if a
    // poll/browse still holds the client lock. Never hold two locks at once.
    *state.connected_device.lock().await = None;
    state.cov_subscriptions.lock().await.clear();

    // Drop the client with a bounded wait so a stuck in-flight poll can't block
    // disconnect forever. UDP is connectionless; BacnetClient::disconnect is a no-op.
    if let Ok(mut guard) =
        tokio::time::timeout(std::time::Duration::from_millis(500), state.client.lock()).await
    {
        guard.take();
    }
    Ok(())
}

// ── Object Browser ──

#[tauri::command]
async fn browse_objects(state: tauri::State<'_, AppState>) -> Result<Vec<BacnetObjectInfo>, String> {
    let dev = state.connected_device.lock().await;
    let dev = dev.as_ref().ok_or("Not connected")?;
    browse_objects_fn(&dev.ip, dev.port, dev.device_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn read_object_properties(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
) -> Result<Option<BacnetObjectDetail>, String> {
    let dev = state.connected_device.lock().await;
    let dev = dev.as_ref().ok_or("Not connected")?;
    let details = read_object_properties_fn(&dev.ip, dev.port, &object_type, object_instance)
        .await
        .map_err(|e| e.to_string())?;
    Ok(details.into_iter().next())
}

#[tauri::command]
async fn inspect_device_cmd(state: tauri::State<'_, AppState>) -> Result<DeviceInspection, String> {
    let dev = state.connected_device.lock().await;
    let dev = dev.as_ref().ok_or("Not connected")?;
    inspect_device_fn(&dev.ip, dev.port, dev.device_id)
        .await
        .map_err(|e| e.to_string())
}

// ── COV Subscriptions ──

#[tauri::command]
async fn subscribe_cov_cmd(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
    lifetime: u32,
    confirmed: bool,
) -> Result<(), String> {
    // Record the desired subscription; the COV listener (which owns the 47808
    // socket) sends the actual SubscribeCOV so notifications return to it.
    let subscriber_id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
    let mut subs = state.cov_subscriptions.lock().await;
    subs.retain(|s| !(s.object_type == object_type && s.object_instance == object_instance));
    subs.push(CovSubscriptionInfo {
        subscriber_process_id: subscriber_id,
        object_type,
        object_instance,
        lifetime,
        confirmed_notifications: confirmed,
        active: true,
    });
    Ok(())
}

#[tauri::command]
async fn unsubscribe_cov_cmd(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
) -> Result<(), String> {
    // Remove from desired set; the listener sends the cancellation from its socket.
    let mut subs = state.cov_subscriptions.lock().await;
    subs.retain(|s| !(s.object_type == object_type && s.object_instance == object_instance));
    Ok(())
}

#[tauri::command]
async fn get_cov_subscriptions(state: tauri::State<'_, AppState>) -> Result<Vec<CovSubscriptionInfo>, String> {
    Ok(state.cov_subscriptions.lock().await.clone())
}

// ── Alarm / Event Monitoring ──

#[tauri::command]
async fn get_alarm_summary(state: tauri::State<'_, AppState>) -> Result<Vec<AlarmSummaryEntry>, String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        get_alarm_summary_blocking(&socket, addr)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

#[tauri::command]
async fn get_event_information(state: tauri::State<'_, AppState>) -> Result<EventInformationResponse, String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        get_event_information_blocking(&socket, addr, None)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

#[tauri::command]
async fn acknowledge_alarm_cmd(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
    event_state: u32,
    ack_text: String,
) -> Result<(), String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        acknowledge_alarm_blocking(&socket, addr, &object_type, object_instance, event_state, &ack_text, None)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

// ── BBMD Registration ──

#[tauri::command]
async fn register_bbmd(bbmd_ip: String, bbmd_port: u16, ttl: u16) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        mt_bacnet::register_bbmd(&bbmd_ip, bbmd_port, ttl)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

// ── Trend Log ──

#[tauri::command]
async fn read_trend_log(
    state: tauri::State<'_, AppState>,
    object_instance: u32,
    start_index: u32,
    count: i32,
) -> Result<TrendLogData, String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        read_trend_log_blocking(&socket, addr, object_instance, start_index, count)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

#[tauri::command]
async fn read_trend_log_count(
    state: tauri::State<'_, AppState>,
    object_instance: u32,
) -> Result<u32, String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        read_trend_log_count_blocking(&socket, addr, object_instance)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

// ── Schedule ──

#[tauri::command]
async fn read_schedule(
    state: tauri::State<'_, AppState>,
    object_instance: u32,
) -> Result<ScheduleData, String> {
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        read_schedule_blocking(&socket, addr, object_instance)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

// ── COV Notification Listener ──

#[tauri::command]
async fn start_cov_listener(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Single listener task owns the 47808 socket and sends SubscribeCOV /
    // cancellations from it, so the device routes notifications back to this socket.
    if state.cov_listener_running.load(Ordering::Relaxed) {
        return Ok(());
    }
    state.cov_listener_running.store(true, Ordering::Relaxed);

    let running = state.cov_listener_running.clone();
    let cov_subs = state.cov_subscriptions.clone();
    let conn_dev = state.connected_device.clone();
    let _dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = match std::net::UdpSocket::bind("0.0.0.0:47808") {
            Ok(s) => s,
            Err(_) => match std::net::UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("COV listener bind failed: {e}");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            },
        };
        socket.set_read_timeout(Some(std::time::Duration::from_millis(250))).ok();

        // (object_type, instance) -> subscriber_process_id already sent.
        let mut sent: HashMap<(String, u32), u32> = HashMap::new();
        let mut buf = [0u8; 1500];

        while running.load(Ordering::Relaxed) {
            let dev_addr = match conn_dev.try_lock() {
                Ok(g) => g.as_ref().and_then(|d| format!("{}:{}", d.ip, d.port).parse::<std::net::SocketAddr>().ok()),
                Err(_) => None,
            };

            let current: Vec<CovSubscriptionInfo> = match cov_subs.try_lock() {
                Ok(g) => g.clone(),
                Err(_) => Vec::new(),
            };
            let current_keys: Vec<(String, u32)> = current
                .iter()
                .map(|s| (s.object_type.clone(), s.object_instance))
                .collect();

            if let Some(addr) = dev_addr {
                // Subscribe new entries.
                for s in &current {
                    let k = (s.object_type.clone(), s.object_instance);
                    if !sent.contains_key(&k) {
                        if let Ok(frame) = build_subscribe_cov_frame(
                            s.subscriber_process_id,
                            &s.object_type,
                            s.object_instance,
                            s.confirmed_notifications,
                            s.lifetime,
                        ) {
                            let _ = socket.send_to(&frame, addr);
                        }
                        sent.insert(k, s.subscriber_process_id);
                    }
                }
                // Cancel removed entries.
                let removed: Vec<(String, u32, u32)> = sent
                    .iter()
                    .filter(|(k, _)| !current_keys.contains(k))
                    .map(|((ot, oi), sid)| (ot.clone(), *oi, *sid))
                    .collect();
                for (ot, oi, sid) in removed {
                    if let Ok(frame) = build_cancel_cov_frame(sid, &ot, oi) {
                        let _ = socket.send_to(&frame, addr);
                    }
                    sent.remove(&(ot, oi));
                }
            }

            match socket.recv_from(&mut buf) {
                Ok((len, _src)) => {
                    if let Some(notification) = parse_cov_packet(&buf[..len]) {
                        let _ = app.emit("cov-notification", &notification);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => {}
            }
        }
        running.store(false, Ordering::Relaxed);
    });

    Ok(())
}

#[tauri::command]
async fn stop_cov_listener(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cov_listener_running.store(false, Ordering::Relaxed);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            client: Arc::new(Mutex::new(None)),
            connected_device: Arc::new(Mutex::new(None)),
            cov_subscriptions: Arc::new(Mutex::new(Vec::new())),
            cov_listener_running: Arc::new(AtomicBool::new(false)),
            sim_device: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            connect, read_tags, write_tag, discover, disconnect,
            browse_objects, read_object_properties, inspect_device_cmd,
            subscribe_cov_cmd, unsubscribe_cov_cmd, get_cov_subscriptions,
            get_alarm_summary, get_event_information, acknowledge_alarm_cmd,
            register_bbmd,
            read_trend_log, read_trend_log_count,
            read_schedule,
            start_cov_listener, stop_cov_listener,
            sim_start, sim_stop, sim_set_value, sim_set_bool, sim_status, open_device_panel,
            system::open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
