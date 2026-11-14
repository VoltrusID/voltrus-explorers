//! Everything the Tauri commands do, without Tauri: one open EtherCAT session (a network interface or the
//! simulator), forwarding its events to the UI (`ecat-io` about ten times a second while process data runs,
//! `ecat-log`, `ecat-frames`), and value parsing for SDO and output writes.

use std::sync::Arc;

use mt_ethercat::coe;
use mt_ethercat::esc::AlState;
use mt_ethercat::link::{self, NetInterface};
use mt_ethercat::master::{
    CycleStats, EntryView, EventSink, IoImage, LinkSpec, ObjectView, PdoMapView, RegistersView, ScanResult, Session, SessionEvent,
    StateView,
};
use mt_ethercat::sii::Sii;
use mt_ethercat::sim::{SimAction, SimState};
use serde::Serialize;
use serde_json::{Value as Json, json};
use tokio::sync::RwLock;

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Json) + Send + Sync>;

pub struct Core {
    emit: Emit,
    session: RwLock<Option<(Session, LinkSpec)>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdoValue {
    pub raw: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAccess {
    /// Whether this computer may open raw sockets right now, as far as can be told without trying.
    pub ok: bool,
    pub detail: String,
    /// macOS: the app can ask for an administrator password to open /dev/bpf* for this session of macOS.
    pub can_grant: bool,
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let h: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if h.is_empty() || !h.len().is_multiple_of(2) {
        return Err("Enter whole bytes in hex, e.g. 01 00".into());
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}

impl Core {
    pub fn new(emit: Emit) -> Self {
        Core { emit, session: RwLock::new(None) }
    }

    fn sink(&self) -> EventSink {
        let emit = self.emit.clone();
        Arc::new(move |e: SessionEvent| match &e {
            SessionEvent::Io { .. } => emit("ecat-io", serde_json::to_value(&e).unwrap_or(Json::Null)),
            SessionEvent::Log { level, message } => emit("ecat-log", json!({ "direction": level, "message": message })),
            SessionEvent::Frames { frames } => emit("ecat-frames", serde_json::to_value(frames).unwrap_or(Json::Null)),
        })
    }

    fn log(&self, direction: &str, message: impl Into<String>) {
        (self.emit)("ecat-log", json!({ "direction": direction, "message": message.into() }));
    }

    pub fn interfaces(&self) -> Vec<NetInterface> {
        link::interfaces()
    }

    pub fn raw_access(&self) -> RawAccess {
        raw_access()
    }

    pub fn grant_raw_access(&self) -> Result<(), String> {
        grant_raw_access()
    }

    async fn session(&self) -> Result<Session, String> {
        self.session.read().await.as_ref().map(|(s, _)| s.clone()).ok_or_else(|| "Not connected".to_string())
    }

    pub async fn connect(&self, spec: LinkSpec) -> Result<ScanResult, String> {
        self.disconnect().await;
        self.log("info", match &spec {
            LinkSpec::Simulator { template } => format!("Starting the simulator: {}", template.label()),
            LinkSpec::Interface { name } => format!("Opening {name} and scanning"),
        });
        let (session, scan) = Session::open(spec.clone(), self.sink()).await?;
        *self.session.write().await = Some((session, spec));
        Ok(scan)
    }

    pub async fn disconnect(&self) {
        if let Some((s, _)) = self.session.write().await.take() {
            s.close();
        }
    }

    /// Scans again on the same link (a new init: addresses, mailboxes, PDO configuration).
    pub async fn rescan(&self) -> Result<ScanResult, String> {
        let spec = self.session.read().await.as_ref().map(|(_, l)| l.clone()).ok_or("Not connected")?;
        // The link opens again; a simulated machine starts fresh.
        self.connect(spec).await
    }

    pub async fn states(&self) -> Result<Vec<StateView>, String> {
        Ok(self.session().await?.states().await)
    }

    pub async fn request_state(&self, state: String, positions: Vec<usize>) -> Result<Vec<StateView>, String> {
        let target = match state.as_str() {
            "INIT" => AlState::Init,
            "PRE-OP" => AlState::PreOp,
            "SAFE-OP" => AlState::SafeOp,
            "OP" => AlState::Op,
            other => return Err(format!("Unknown state {other}")),
        };
        self.session().await?.request_state(target, positions).await
    }

    pub async fn registers(&self, position: usize) -> Result<RegistersView, String> {
        self.session().await?.registers(position).await
    }

    pub async fn write_register(&self, position: usize, addr: u16, hex: String) -> Result<(), String> {
        let data = parse_hex(&hex)?;
        self.session().await?.write_register(position, addr, &data).await
    }

    pub async fn reset_counters(&self, position: usize) -> Result<(), String> {
        self.session().await?.reset_error_counters(position).await?;
        self.log("info", format!("Device {position}: error counters cleared"));
        Ok(())
    }

    pub async fn sii(&self, position: usize) -> Result<Sii, String> {
        self.session().await?.sii(position).await
    }

    pub async fn eeprom(&self, position: usize) -> Result<Vec<u8>, String> {
        self.session().await?.eeprom(position).await
    }

    pub async fn pdo_map(&self, position: usize) -> Result<PdoMapView, String> {
        self.session().await?.pdo_map(position).await
    }

    pub async fn objects(&self, position: usize) -> Result<Vec<ObjectView>, String> {
        self.session().await?.object_list(position).await
    }

    pub async fn object_entries(&self, position: usize, index: u16) -> Result<Vec<EntryView>, String> {
        self.session().await?.object_entries(position, index).await
    }

    pub async fn sdo_read(&self, position: usize, index: u16, sub: u8, data_type: u16) -> Result<SdoValue, String> {
        let b = self.session().await?.sdo_upload(position, index, sub).await?;
        Ok(SdoValue { raw: b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" "), value: coe::format_value(data_type, &b) })
    }

    pub async fn sdo_write(&self, position: usize, index: u16, sub: u8, data_type: u16, bits: u16, text: String) -> Result<(), String> {
        let data = coe::parse_value(data_type, bits, &text)?;
        self.session().await?.sdo_download(position, index, sub, &data).await
    }

    pub async fn write_output(&self, position: usize, bit_offset: usize, bits: usize, data_type: u16, text: String) -> Result<(), String> {
        let data = coe::parse_value(data_type, bits as u16, &text)?;
        let v = data.iter().take(8).enumerate().fold(0u64, |a, (i, &b)| a | (b as u64) << (8 * i));
        self.session().await?.write_output(position, bit_offset, bits, v)
    }

    pub async fn set_cycle(&self, ms: f64) -> Result<(), String> {
        self.session().await?.set_cycle_ms(ms);
        Ok(())
    }

    pub async fn cycle(&self, run: bool) -> Result<(), String> {
        let s = self.session().await?;
        if run {
            s.start_cycle();
        } else {
            s.stop_cycle();
        }
        Ok(())
    }

    pub async fn io(&self) -> Result<(CycleStats, Vec<IoImage>), String> {
        let s = self.session().await?;
        Ok((s.stats(), s.images()))
    }

    pub async fn capture(&self, frames: usize) -> Result<(), String> {
        self.session().await?.capture(frames)
    }

    pub async fn sim_state(&self) -> Option<SimState> {
        let s = self.session().await.ok()?;
        let seg = s.segment.as_ref()?;
        Some(seg.lock().ok()?.state())
    }

    pub async fn sim_action(&self, action: SimAction) -> Result<(), String> {
        let s = self.session().await?;
        let seg = s.segment.as_ref().ok_or("Not connected to the simulator")?;
        seg.lock().map_err(|e| e.to_string())?.action(&action)?;
        self.log("info", format!("Simulator: {}", describe_action(&action)));
        Ok(())
    }
}

fn describe_action(a: &SimAction) -> String {
    match a {
        SimAction::BreakCable { after: Some(k) } => format!("cable pulled after device {k}"),
        SimAction::BreakCable { after: None } => "cable plugged back in".into(),
        SimAction::RxErrors { device, port, count } => format!("{count} RX errors on device {device} port {port}"),
        SimAction::Device { device, action } => format!("device {device}: {action}"),
    }
}

#[cfg(target_os = "macos")]
fn raw_access() -> RawAccess {
    use std::fs::OpenOptions;
    let ok = (0..8).any(|i| OpenOptions::new().read(true).write(true).open(format!("/dev/bpf{i}")).is_ok());
    RawAccess {
        ok,
        detail: if ok {
            "This Mac lets the app send and receive raw Ethernet frames (BPF).".into()
        } else {
            "macOS keeps raw Ethernet (/dev/bpf*) for administrators. Grant access once (asks for your password; macOS \
             resets it at restart), or install Wireshark, whose ChmodBPF helper grants it at every start."
                .into()
        },
        can_grant: !ok,
    }
}

#[cfg(target_os = "macos")]
fn grant_raw_access() -> Result<(), String> {
    // The admin group gets read/write on the BPF devices until the next restart, as Wireshark's ChmodBPF does for
    // its own group.
    let script = r#"do shell script "chgrp admin /dev/bpf* && chmod g+rw /dev/bpf*" with administrator privileges"#;
    let out = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.contains("-128") { "Cancelled".into() } else { format!("Could not grant access: {msg}") })
    }
}

#[cfg(target_os = "linux")]
fn raw_access() -> RawAccess {
    // Opening an AF_PACKET socket is the honest test of CAP_NET_RAW.
    let fd = unsafe { libc_socket() };
    let ok = fd >= 0;
    RawAccess {
        ok,
        detail: if ok {
            "This process may send raw Ethernet frames.".into()
        } else {
            "Raw Ethernet needs CAP_NET_RAW: run once `sudo setcap cap_net_raw,cap_net_admin=eip` on the app's binary, \
             then start it again."
                .into()
        },
        can_grant: false,
    }
}

#[cfg(target_os = "linux")]
unsafe fn libc_socket() -> i32 {
    unsafe extern "C" {
        fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
        fn close(fd: i32) -> i32;
    }
    // AF_PACKET = 17, SOCK_RAW = 3, ETH_P_ALL (big endian) = 0x0300.
    let fd = unsafe { socket(17, 3, 0x0300) };
    if fd >= 0 {
        unsafe { close(fd) };
    }
    fd
}

#[cfg(target_os = "linux")]
fn grant_raw_access() -> Result<(), String> {
    Err("On Linux, give the binary CAP_NET_RAW with setcap (see the note).".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn raw_access() -> RawAccess {
    RawAccess { ok: false, detail: "Network interfaces are supported on macOS and Linux; the simulator works everywhere.".into(), can_grant: false }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn grant_raw_access() -> Result<(), String> {
    Err("Not supported on this system".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test(flavor = "multi_thread")]
    async fn simulator_through_the_core() {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let core = Core::new(Arc::new(move |ev: &str, _| s2.lock().unwrap().push(ev.to_string())));
        let scan = core.connect(LinkSpec::Simulator { template: mt_ethercat::sim::Template::IoStation }).await.unwrap();
        assert_eq!(scan.devices.len(), 5);
        core.set_cycle(2.0).await.unwrap();
        let st = core.request_state("OP".into(), vec![]).await.unwrap();
        assert!(st.iter().all(|s| s.state == "OP"), "{st:?}");
        // EL4102 ch.1 (bit 0, INT) at 8192 (2.5 V) shows up on EL3102 ch.2 after a few cycles.
        core.write_output(4, 0, 16, 0x0003, "8192".into()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let (_, imgs) = core.io().await.unwrap();
        let ch2 = i16::from_le_bytes([imgs[3].inputs[6], imgs[3].inputs[7]]);
        assert!((ch2 - 8192).abs() < 40, "{ch2}");
        core.sdo_write(3, 0x8000, 0x0A, 0x0001, 1, "true".into()).await.unwrap();
        assert_eq!(core.sdo_read(3, 0x8000, 0x0A, 0x0001).await.unwrap().value, "TRUE");
        assert!(core.sdo_write(3, 0x8000, 0x11, 0x0003, 16, "99999".into()).await.unwrap_err().contains("outside"));
        core.sim_action(SimAction::Device { device: 1, action: "start".into() }).await.unwrap();
        assert!(core.sim_state().await.unwrap().devices[1].summary.iter().any(|(k, v)| k.starts_with("Start") && v == "pressed"));
        core.disconnect().await;
        assert!(seen.lock().unwrap().iter().any(|e| e == "ecat-io"));
    }
}
