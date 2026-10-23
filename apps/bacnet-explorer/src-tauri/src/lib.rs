mod system;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use mt_bacnet::transport::{self, Transaction, REQUEST_TIMEOUT};
use mt_bacnet::alarm::{parse_alarm_packet, SERVICE_CONFIRMED_EVENT_NOTIFICATION};
use mt_bacnet::cov::simple_ack_for;
use tokio::sync::Mutex;
use tauri::Emitter;
use tauri::Manager;
use mt_bacnet::{
    BacnetClient, BacnetConfig, BacnetObject, BacnetValue,
    discover_devices, browse_objects as browse_objects_fn,
    read_object_properties as read_object_properties_fn,
    inspect_device as inspect_device_fn,
    BacnetDiscoveredDevice, BacnetObjectInfo,
    BacnetObjectDetail, DeviceInspection,
    // COV
    build_subscribe_cov_frame, build_cancel_cov_frame, CovSubscriptionInfo,
    parse_cov_packet, renewal_delay,
    // Alarms
    get_alarm_summary_blocking, get_event_information_blocking,
    acknowledge_alarm_blocking, AlarmSummaryEntry, EventInformationResponse, EventTimeStamp,
    // Trend
    read_trend_log_blocking, read_trend_log_count_blocking, TrendLogData, TrendLogKind,
    // Schedule
    read_schedule_blocking, ScheduleData,
    // Device simulator
    SimDevice, SimPointDef,
};
use mt_protocol::ProtocolClient;

static NEXT_SUBSCRIBER_ID: AtomicU32 = AtomicU32::new(1);

/// Every open device keeps its own client. `client` / `connected_device` mirror the
/// active one: all device commands (browse, read, write, COV, alarms ...) act on it.
struct AppState {
    sessions: Arc<Mutex<Vec<Session>>>,
    client: Arc<Mutex<Option<BacnetClient>>>,
    connected_device: Arc<Mutex<Option<ConnectedDevice>>>,
    cov_subscriptions: Arc<Mutex<Vec<CovSubscriptionInfo>>>,
    cov_listener: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    sim_device: Arc<Mutex<Option<SimDevice>>>,
}

#[derive(Clone)]
struct Session {
    device: ConnectedDevice,
    client: BacnetClient,
}

/// Identifies an open device (BACnet/IP address plus device instance).
#[derive(Clone, serde::Deserialize)]
struct DeviceRef {
    ip: String,
    port: u16,
    device_id: u32,
}

impl DeviceRef {
    fn matches(&self, d: &ConnectedDevice) -> bool {
        d.ip == self.ip && d.port == self.port && d.device_id == self.device_id
    }
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
    // Take the device out under the lock, stop it after the guard is dropped.
    let dev = state.sim_device.lock().await.take();
    if let Some(dev) = dev {
        dev.stop();
    }
    // Close the device control panel if it's open.
    if let Some(w) = app.get_webview_window("device-panel") {
        let _ = w.close();
    }
    app.emit("sim-stopped", ()).map_err(|e| e.to_string())
}

/// Open the device control panel as a separate OS window. Built from Rust so it
/// doesn't depend on JS `create-webview-window` capability scoping. iOS has a
/// single window, so the UI shows the same panel inline there and never calls this.
#[tauri::command]
async fn open_device_panel(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
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
    #[cfg(mobile)]
    {
        let _ = app;
        Err("The control panel opens inside the app on this device".into())
    }
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

/// Make `session` the device the commands act on. COV state belongs to one device, so
/// switching drops the listener and the desired subscriptions (the old device lets them
/// expire; the UI re-creates the trend subscriptions it still wants on the new device).
/// The lock order matches `disconnect`: device first, then the listener, then the rest.
async fn set_active(state: &AppState, session: Option<Session>) {
    *state.connected_device.lock().await = None;
    stop_cov_listener_inner(&state.cov_listener).await;
    state.cov_subscriptions.lock().await.clear();
    match session {
        Some(s) => {
            *state.client.lock().await = Some(s.client);
            *state.connected_device.lock().await = Some(s.device);
        }
        None => {
            state.client.lock().await.take();
        }
    }
}

/// Snapshot of the connected device. The lock is released before returning so
/// callers never hold it across a network await.
async fn current_device(state: &AppState) -> Result<ConnectedDevice, String> {
    state
        .connected_device
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())
}

/// Cheap clone of the connected client (shared socket); the lock is released
/// before returning.
async fn current_client(state: &AppState) -> Result<BacnetClient, String> {
    state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Not connected".to_string())
}

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>, config: BacnetConfig) -> Result<(), String> {
    let device = ConnectedDevice {
        ip: config.ip.clone(),
        port: config.port,
        device_id: config.device_id,
    };
    let client = BacnetClient::connect(config).await.map_err(|e| e.to_string())?;
    let r = DeviceRef { ip: device.ip.clone(), port: device.port, device_id: device.device_id };
    let session = Session { device, client };
    {
        let mut sessions = state.sessions.lock().await;
        sessions.retain(|s| !r.matches(&s.device));
        sessions.push(session.clone());
    }
    if is_active(&state, &r).await {
        // Reconnecting the active device only swaps its client; COV state stays valid.
        *state.client.lock().await = Some(session.client);
    } else {
        set_active(&state, Some(session)).await;
    }
    Ok(())
}

async fn is_active(state: &AppState, device: &DeviceRef) -> bool {
    state
        .connected_device
        .lock()
        .await
        .as_ref()
        .is_some_and(|d| device.matches(d))
}

/// Switch the device the commands act on to one that is already open.
#[tauri::command]
async fn activate_device(state: tauri::State<'_, AppState>, device: DeviceRef) -> Result<(), String> {
    let session = state
        .sessions
        .lock()
        .await
        .iter()
        .find(|s| device.matches(&s.device))
        .cloned()
        .ok_or_else(|| "Device is not open".to_string())?;
    let already_active = is_active(&state, &device).await;
    if !already_active {
        set_active(&state, Some(session)).await;
    }
    Ok(())
}

/// Close one open device. When it was the active one nothing is active afterwards:
/// the caller activates another open device (or none is left).
#[tauri::command]
async fn close_device(state: tauri::State<'_, AppState>, device: DeviceRef) -> Result<(), String> {
    state.sessions.lock().await.retain(|s| !device.matches(&s.device));
    let was_active = is_active(&state, &device).await;
    if was_active {
        set_active(&state, None).await;
    }
    Ok(())
}

#[tauri::command]
async fn read_tags(
    state: tauri::State<'_, AppState>,
    tags: Vec<BacnetObject>,
) -> Result<Vec<BacnetValue>, String> {
    let client = current_client(&state).await?;
    client.read(&tags).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_tag(
    state: tauri::State<'_, AppState>,
    tag: BacnetObject,
    value: BacnetValue,
    priority: Option<u8>,
) -> Result<(), String> {
    let client = current_client(&state).await?;
    client
        .write_with_priority(&tag, value, priority)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn discover(timeout_secs: u64) -> Result<Vec<BacnetDiscoveredDevice>, String> {
    discover_devices(timeout_secs).await.map_err(|e| e.to_string())
}

/// Close every open device.
#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // No network await happens under any of these locks (commands clone what they
    // need), so none of them can be held for long. Never hold two locks at once.
    state.sessions.lock().await.clear();
    set_active(&state, None).await;
    Ok(())
}

// ── Object Browser ──

#[tauri::command]
async fn browse_objects(state: tauri::State<'_, AppState>) -> Result<Vec<BacnetObjectInfo>, String> {
    let dev = current_device(&state).await?;
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
    let dev = current_device(&state).await?;
    let details = read_object_properties_fn(&dev.ip, dev.port, &object_type, object_instance)
        .await
        .map_err(|e| e.to_string())?;
    Ok(details.into_iter().next())
}

#[tauri::command]
async fn inspect_device_cmd(state: tauri::State<'_, AppState>) -> Result<DeviceInspection, String> {
    let dev = current_device(&state).await?;
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
        get_event_information_blocking(&socket, addr)
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
    time_stamp: EventTimeStamp,
    ack_source: String,
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
        acknowledge_alarm_blocking(&socket, addr, &object_type, object_instance, event_state, &time_stamp, &ack_source, None)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

// ── Trend Log ──

#[tauri::command]
async fn read_trend_log(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
    reference_index: u32,
    count: i32,
) -> Result<TrendLogData, String> {
    let kind = TrendLogKind::parse(&object_type).map_err(|e| e.to_string())?;
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        read_trend_log_blocking(&socket, addr, kind, object_instance, reference_index, count)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

/// ReadRange `bySequenceNumber`: continuation pages of a live log. Sequence numbers do not
/// shift when a full circular buffer wraps, unlike positions.
#[tauri::command]
async fn read_trend_log_by_sequence(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
    reference_sequence_number: u32,
    count: i32,
) -> Result<TrendLogData, String> {
    let kind = TrendLogKind::parse(&object_type).map_err(|e| e.to_string())?;
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        mt_bacnet::trend::read_trend_log_by_sequence_blocking(
            &socket,
            addr,
            kind,
            object_instance,
            reference_sequence_number,
            count,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join: {e}"))?
}

#[tauri::command]
async fn read_trend_log_count(
    state: tauri::State<'_, AppState>,
    object_type: String,
    object_instance: u32,
) -> Result<u32, String> {
    let kind = TrendLogKind::parse(&object_type).map_err(|e| e.to_string())?;
    let dev = {
        let guard = state.connected_device.lock().await;
        guard.as_ref().ok_or("Not connected")?.clone()
    };

    tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Socket bind: {e}"))?;
        let addr: std::net::SocketAddr = format!("{}:{}", dev.ip, dev.port)
            .parse().map_err(|e| format!("Invalid addr: {e}"))?;
        read_trend_log_count_blocking(&socket, addr, kind, object_instance)
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

/// Emitted when a SubscribeCOV (or renewal) fails: the device answered with an
/// Error/Reject/Abort PDU, did not answer in time, or the request could not be
/// sent. The failed subscription is removed from `cov_subscriptions`.
#[derive(Clone, serde::Serialize)]
struct CovSubscribeFailed {
    object_type: String,
    object_instance: u32,
    message: String,
}

/// Emitted when the listener thread dies on a socket error.
#[derive(Clone, serde::Serialize)]
struct CovListenerError {
    message: String,
}

type CovKey = (String, u32);

/// A SubscribeCOV request sent from the listener socket, awaiting its reply.
struct CovPending {
    key: CovKey,
    subscriber_id: u32,
    addr: SocketAddr,
    sent_at: Instant,
    txn: Transaction,
}

/// State owned by the single COV listener thread. It sends SubscribeCOV /
/// cancellations from its own socket so the device routes notifications (and
/// replies) back to that socket.
struct CovListener {
    app: tauri::AppHandle,
    socket: UdpSocket,
    cov_subs: Arc<Mutex<Vec<CovSubscriptionInfo>>>,
    /// Subscriptions sent from this socket: key -> (subscriber id, time of the last SubscribeCOV).
    sent: HashMap<CovKey, (u32, Instant)>,
    /// Outstanding SubscribeCOV requests by invoke id.
    pending: HashMap<u8, CovPending>,
}

impl CovListener {
    /// Bring the device in line with the desired subscription set: subscribe new
    /// entries, renew those near expiry, cancel removed ones.
    fn reconcile(&mut self, addr: SocketAddr, current: &[CovSubscriptionInfo]) {
        for s in current {
            let key = (s.object_type.clone(), s.object_instance);
            match self.sent.get(&key).copied() {
                None => self.subscribe(addr, s),
                Some((sid, _)) if sid != s.subscriber_process_id => {
                    self.cancel(addr, &key, sid);
                    self.subscribe(addr, s);
                }
                Some((_, sent_at)) => {
                    let due = renewal_delay(s.lifetime).is_some_and(|d| sent_at.elapsed() >= d);
                    if due && !self.pending.values().any(|p| p.key == key) {
                        self.subscribe(addr, s);
                    }
                }
            }
        }

        let removed: Vec<(CovKey, u32)> = self
            .sent
            .iter()
            .filter(|(k, _)| !current.iter().any(|s| s.object_type == k.0 && s.object_instance == k.1))
            .map(|(k, (sid, _))| (k.clone(), *sid))
            .collect();
        for (key, sid) in removed {
            self.cancel(addr, &key, sid);
        }
    }

    /// Send SubscribeCOV (initial or renewal) and track the reply.
    fn subscribe(&mut self, addr: SocketAddr, s: &CovSubscriptionInfo) {
        let key = (s.object_type.clone(), s.object_instance);
        let sid = s.subscriber_process_id;
        let invoke_id = transport::next_invoke_id();
        let frame = match build_subscribe_cov_frame(
            invoke_id,
            sid,
            &s.object_type,
            s.object_instance,
            s.confirmed_notifications,
            s.lifetime,
        ) {
            Ok(f) => f,
            Err(e) => return self.fail(&key, sid, e.to_string()),
        };
        if let Err(e) = self.socket.send_to(&frame, addr) {
            return self.fail(&key, sid, format!("Send SubscribeCOV: {e}"));
        }
        let now = Instant::now();
        self.sent.insert(key.clone(), (sid, now));
        self.pending.retain(|_, p| p.key != key);
        self.pending.insert(
            invoke_id,
            CovPending {
                key,
                subscriber_id: sid,
                addr,
                sent_at: now,
                txn: Transaction::new(invoke_id),
            },
        );
    }

    /// Send a cancellation (SubscribeCOV without confirmed flag and lifetime) and
    /// forget the subscription. The reply is not tracked: the subscription is
    /// gone from the UI either way.
    fn cancel(&mut self, addr: SocketAddr, key: &CovKey, sid: u32) {
        self.sent.remove(key);
        self.pending.retain(|_, p| p.key != *key);
        let invoke_id = transport::next_invoke_id();
        match build_cancel_cov_frame(invoke_id, sid, &key.0, key.1) {
            Ok(frame) => {
                if let Err(e) = self.socket.send_to(&frame, addr) {
                    eprintln!("COV cancel for {}:{} not sent: {e}", key.0, key.1);
                }
            }
            Err(e) => eprintln!("COV cancel for {}:{} not built: {e}", key.0, key.1),
        }
    }

    /// Drop a subscription that could not be established or renewed and tell the UI.
    fn fail(&mut self, key: &CovKey, sid: u32, message: String) {
        self.sent.remove(key);
        self.pending.retain(|_, p| p.key != *key);
        // Runs on a spawn_blocking thread, so blocking_lock is allowed; the lock is only
        // ever held briefly. Only remove the entry this failure belongs to.
        self.cov_subs.blocking_lock().retain(|s| {
            !(s.object_type == key.0 && s.object_instance == key.1 && s.subscriber_process_id == sid)
        });
        let payload = CovSubscribeFailed {
            object_type: key.0.clone(),
            object_instance: key.1,
            message,
        };
        if let Err(e) = self.app.emit("cov-subscribe-failed", payload) {
            eprintln!("cov-subscribe-failed event not delivered: {e}");
        }
    }

    /// Fail every SubscribeCOV that got no reply within the request timeout.
    fn expire_pending(&mut self) {
        let expired: Vec<(u8, CovKey, u32, SocketAddr)> = self
            .pending
            .iter()
            .filter(|(_, p)| p.sent_at.elapsed() >= REQUEST_TIMEOUT)
            .map(|(id, p)| (*id, p.key.clone(), p.subscriber_id, p.addr))
            .collect();
        for (id, key, sid, addr) in expired {
            self.pending.remove(&id);
            self.fail(&key, sid, format!("SubscribeCOV timed out: no reply from {addr}"));
        }
    }

    /// Handle one received datagram: a reply to a pending SubscribeCOV, or a COV notification.
    fn handle_frame(&mut self, src: SocketAddr, frame: &[u8]) {
        if let Some(apdu) = transport::apdu_of_frame(frame) {
            let mut finished = Vec::new();
            for (id, p) in self.pending.iter_mut() {
                if p.addr != src {
                    continue;
                }
                if let Some(result) = p.txn.handle(apdu).done {
                    finished.push((*id, result));
                }
            }
            for (id, result) in finished {
                if let Some(p) = self.pending.remove(&id) {
                    if let Err(e) = result {
                        self.fail(&p.key, p.subscriber_id, e.to_string());
                    }
                }
            }
        }

        if let Some(packet) = parse_cov_packet(frame) {
            // ConfirmedCOVNotification (service choice 1) must be acknowledged, otherwise
            // the device retransmits and may drop the subscription.
            if let Some(invoke_id) = packet.confirmed_invoke_id {
                self.ack_confirmed(frame, src, invoke_id, 1);
            }
            if let Err(e) = self.app.emit("cov-notification", &packet.notification) {
                eprintln!("cov-notification event not delivered: {e}");
            }
        } else if let Some(packet) = parse_alarm_packet(frame, src) {
            if let Some(invoke_id) = packet.confirmed_invoke_id {
                self.ack_confirmed(frame, src, invoke_id, SERVICE_CONFIRMED_EVENT_NOTIFICATION);
            }
            if let Err(e) = self.app.emit("alarm-notification", &packet.notification) {
                eprintln!("alarm-notification event not delivered: {e}");
            }
        }
    }

    /// Send the SimpleAck for a confirmed notification to its originator (the BVLC
    /// origin of a Forwarded-NPDU, routed through the NPDU source network if any).
    fn ack_confirmed(&self, frame: &[u8], src: SocketAddr, invoke_id: u8, service_choice: u8) {
        match simple_ack_for(frame, src, invoke_id, service_choice) {
            Some((to, ack)) => {
                if let Err(e) = self.socket.send_to(&ack, to) {
                    eprintln!("Notification SimpleAck to {to} not sent: {e}");
                }
            }
            None => eprintln!("Notification SimpleAck for invoke id {invoke_id} could not be built"),
        }
    }
}

/// Listener thread body; returns when `token` is cleared or the socket fails.
fn run_cov_listener(
    mut l: CovListener,
    token: Arc<AtomicBool>,
    conn_dev: Arc<Mutex<Option<ConnectedDevice>>>,
) {
    let mut buf = [0u8; 1500];

    while token.load(Ordering::Relaxed) {
        // When either lock is busy, skip the diff for this pass: a busy lock must
        // never be read as "no subscriptions", which would cancel them all.
        let snapshot = match (conn_dev.try_lock(), l.cov_subs.try_lock()) {
            (Ok(dev), Ok(subs)) => Some((
                dev.as_ref()
                    .and_then(|d| format!("{}:{}", d.ip, d.port).parse::<SocketAddr>().ok()),
                subs.clone(),
            )),
            _ => None,
        };
        if let Some((Some(addr), current)) = snapshot {
            l.reconcile(addr, &current);
        }

        match l.socket.recv_from(&mut buf) {
            Ok((len, src)) => l.handle_frame(src, &buf[..len]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::ConnectionReset
                ) => {}
            Err(e) => {
                let message = format!("COV listener stopped: socket receive failed: {e}");
                if let Err(e) = l.app.emit("cov-listener-error", CovListenerError { message }) {
                    eprintln!("cov-listener-error event not delivered: {e}");
                }
                break;
            }
        }
        l.expire_pending();
    }
    token.store(false, Ordering::Relaxed);
}

/// Bind the listener socket: BACnet's well-known port when free, otherwise an
/// ephemeral one (the device notifies the source address of the SubscribeCOV).
fn bind_cov_socket() -> Result<UdpSocket, String> {
    let socket = UdpSocket::bind("0.0.0.0:47808")
        .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
        .map_err(|e| format!("COV listener could not bind a UDP socket: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| format!("COV listener socket setup failed: {e}"))?;
    Ok(socket)
}

/// Signal the current COV listener (if any) to exit and forget it. A listener
/// started afterwards gets its own token, so a stop-then-start can never
/// re-arm the old thread.
async fn stop_cov_listener_inner(slot: &Mutex<Option<Arc<AtomicBool>>>) {
    if let Some(token) = slot.lock().await.take() {
        token.store(false, Ordering::Relaxed);
    }
}

#[tauri::command]
async fn start_cov_listener(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // The connection is checked while holding the listener slot; `disconnect` clears the
    // device before it takes the slot, so a racing start either fails here or is stopped.
    let mut slot = state.cov_listener.lock().await;
    if state.connected_device.lock().await.is_none() {
        return Err("Not connected".into());
    }
    if slot.as_ref().is_some_and(|t| t.load(Ordering::Relaxed)) {
        return Ok(());
    }

    let socket = bind_cov_socket()?;
    let token = Arc::new(AtomicBool::new(true));
    *slot = Some(token.clone());

    let listener = CovListener {
        app,
        socket,
        cov_subs: state.cov_subscriptions.clone(),
        sent: HashMap::new(),
        pending: HashMap::new(),
    };
    let conn_dev = state.connected_device.clone();
    tokio::task::spawn_blocking(move || run_cov_listener(listener, token, conn_dev));
    Ok(())
}

#[tauri::command]
async fn stop_cov_listener(state: tauri::State<'_, AppState>) -> Result<(), String> {
    stop_cov_listener_inner(&state.cov_listener).await;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mt_connect::setup(
        mt_store::setup(tauri::Builder::default(), include_str!("../store.json")),
        include_str!("../connect.json"),
    )
        // Native open/save panels + writes to the paths the user picked (the only
        // file access the sandboxed App Store build has).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            sessions: Arc::new(Mutex::new(Vec::new())),
            client: Arc::new(Mutex::new(None)),
            connected_device: Arc::new(Mutex::new(None)),
            cov_subscriptions: Arc::new(Mutex::new(Vec::new())),
            cov_listener: Arc::new(Mutex::new(None)),
            sim_device: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            connect, activate_device, close_device, read_tags, write_tag, discover, disconnect,
            browse_objects, read_object_properties, inspect_device_cmd,
            subscribe_cov_cmd, unsubscribe_cov_cmd, get_cov_subscriptions,
            get_alarm_summary, get_event_information, acknowledge_alarm_cmd,
            read_trend_log, read_trend_log_by_sequence, read_trend_log_count,
            read_schedule,
            start_cov_listener, stop_cov_listener,
            sim_start, sim_stop, sim_set_value, sim_set_bool, sim_status, open_device_panel,
            system::open_url, system::platform,
            mt_store::store_status, mt_store::store_purchase, mt_store::store_restore,
            mt_connect::connect_info, mt_connect::connect_signed_in, mt_connect::connect_sign_in, mt_connect::connect_sign_out,
            mt_connect::connect_me, mt_connect::connect_sync, mt_connect::connect_profile_save, mt_connect::connect_new_id,
            mt_connect::connect_ai_availability, mt_connect::connect_ai_status, mt_connect::connect_ai_run, mt_connect::connect_open_url, mt_connect::connect_teams, mt_connect::connect_team_create, mt_connect::connect_team_invite,
            mt_connect::connect_invite_accept, mt_connect::connect_team_remove_member, mt_connect::connect_remote_sites,
            mt_connect::connect_remote_join, mt_connect::connect_store_products, mt_connect::connect_subscribe,
            mt_connect::connect_link_purchases
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
