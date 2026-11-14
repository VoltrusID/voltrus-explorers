//! Moves EtherCrab's frames: to a [`Segment`] inside this process (the simulator), or to a network interface
//! through EtherCrab's raw socket task (BPF on macOS, `AF_PACKET` on Linux).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use ethercrab::{PduRx, PduTx};

use crate::sim::Segment;

/// Direction of a frame for the tap: `true` from the master, `false` back to it.
pub type Tap = Arc<dyn Fn(bool, &[u8]) + Send + Sync>;

/// Drives EtherCrab's PDU loop against a simulated segment. Finishes when the main device is released.
pub struct VirtualLink {
    seg: Arc<Mutex<Segment>>,
    tx: Option<PduTx<'static>>,
    rx: Option<PduRx<'static>>,
    tap: Option<Tap>,
}

impl VirtualLink {
    pub fn new(seg: Arc<Mutex<Segment>>, tx: PduTx<'static>, rx: PduRx<'static>, tap: Option<Tap>) -> Self {
        VirtualLink { seg, tx: Some(tx), rx: Some(rx), tap }
    }
}

impl Future for VirtualLink {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let (Some(tx), Some(rx)) = (this.tx.as_mut(), this.rx.as_mut()) else { return Poll::Ready(()) };
        tx.replace_waker(cx.waker());
        if tx.should_exit() {
            this.tx.take();
            this.rx.take();
            return Poll::Ready(());
        }
        let mut answers: Vec<Vec<u8>> = Vec::new();
        while let Some(frame) = tx.next_sendable_frame() {
            let _ = frame.send_blocking(|data| {
                if let Some(tap) = &this.tap {
                    tap(true, data);
                }
                let mut buf = data.to_vec();
                let ok = this.seg.lock().map(|mut s| s.process(&mut buf)).unwrap_or(false);
                if ok {
                    answers.push(buf);
                }
                Ok(data.len())
            });
        }
        for a in answers {
            if let Some(tap) = &this.tap {
                tap(false, &a);
            }
            let _ = rx.receive_frame(&a);
        }
        Poll::Pending
    }
}

/// Network interfaces that could carry EtherCAT: up, not loopback, with an Ethernet address.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetInterface {
    pub name: String,
    pub mac: Option<String>,
    pub up: bool,
    pub running: bool,
    /// What macOS calls it ("USB 10/100/1000 LAN", "Thunderbolt Ethernet Slot 1"), when known.
    pub label: Option<String>,
    /// Wi-Fi cannot carry EtherCAT frames.
    pub wireless: bool,
}

#[cfg(unix)]
pub fn interfaces() -> Vec<NetInterface> {
    use nix::net::if_::InterfaceFlags;
    use std::collections::BTreeMap;
    let mut out: BTreeMap<String, NetInterface> = BTreeMap::new();
    let Ok(addrs) = nix::ifaddrs::getifaddrs() else { return Vec::new() };
    for a in addrs {
        if a.flags.contains(InterfaceFlags::IFF_LOOPBACK) {
            continue;
        }
        let e = out.entry(a.interface_name.clone()).or_insert_with(|| NetInterface {
            name: a.interface_name.clone(),
            mac: None,
            up: a.flags.contains(InterfaceFlags::IFF_UP),
            running: a.flags.contains(InterfaceFlags::IFF_RUNNING),
            label: None,
            wireless: false,
        });
        if let Some(link) = a.address.as_ref().and_then(|s| s.as_link_addr()) {
            if let Some(mac) = link.addr() {
                if mac.iter().any(|b| *b != 0) {
                    e.mac = Some(mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"));
                }
            }
        }
    }
    let labels = hardware_ports();
    let mut v: Vec<NetInterface> = out
        .into_values()
        .filter(|i| i.mac.is_some())
        .map(|mut i| {
            i.label = labels.get(&i.name).cloned();
            i.wireless = i.label.as_deref().is_some_and(|l| l.contains("Wi-Fi") || l.contains("AirPort"))
                || i.name.starts_with("wl")
                || std::path::Path::new(&format!("/sys/class/net/{}/wireless", i.name)).exists();
            i
        })
        // Virtual interfaces macOS creates for AirDrop, Continuity and VPNs; never wired Ethernet.
        .filter(|i| !["awdl", "llw", "utun", "ap", "bridge", "anpi", "gif", "stf"].iter().any(|p| i.name.starts_with(p)))
        .collect();
    v.sort_by_key(|i| (i.wireless, !i.running, i.name.clone()));
    v
}

#[cfg(not(unix))]
pub fn interfaces() -> Vec<NetInterface> {
    Vec::new()
}

/// macOS hardware port names by device, from `networksetup -listallhardwareports`.
fn hardware_ports() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if !cfg!(target_os = "macos") {
        return map;
    }
    let Ok(out) = std::process::Command::new("/usr/sbin/networksetup").arg("-listallhardwareports").output() else {
        return map;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut port: Option<String> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port: ") {
            port = Some(p.trim().to_string());
        } else if let Some(d) = line.strip_prefix("Device: ") {
            if let Some(p) = port.take() {
                map.insert(d.trim().to_string(), p);
            }
        }
    }
    map
}

/// Why opening a raw socket failed, in words a user can act on.
pub fn open_error_hint(e: &std::io::Error) -> String {
    let denied = e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(1) || e.raw_os_error() == Some(13);
    if !denied {
        return format!("Could not open the interface: {e}");
    }
    if cfg!(target_os = "macos") {
        "macOS does not let this app send raw Ethernet frames yet (no access to /dev/bpf*). Grant access once from the \
         interface panel, or install Wireshark's ChmodBPF, then connect again."
            .into()
    } else {
        "Sending raw Ethernet frames needs the CAP_NET_RAW capability. Run once: \
         sudo setcap cap_net_raw,cap_net_admin=eip <path to ethercat-explorer>, then start the app again."
            .into()
    }
}
