//! The bus simulator on a real SocketCAN interface: it runs on one `vcan0` socket and a second socket (as CAN Explorer, or
//! candump, would) sees its J1939 traffic and the 64-byte CAN FD battery pack, and gets its OBD-II answers. Skipped (passes
//! with a note) when the host has no `vcan0` that is up and CAN FD capable. Create one with
//! `sudo ip link add vcan0 type vcan && sudo ip link set vcan0 mtu 72 up`.
#![cfg(target_os = "linux")]

use std::time::Duration;

use mt_can::dbc::Dbc;
use mt_can::sim::{SIM_DBC, SimAction, Simulator};
use mt_can::{BusEvent, CanFrame, Direction, InterfaceConfig, VirtualBus};

const IFACE: &str = "vcan0";

fn iface_mtu() -> Option<u32> {
    let base = format!("/sys/class/net/{IFACE}");
    let flags = std::fs::read_to_string(format!("{base}/flags")).ok()?;
    let up = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).ok()? & 1 != 0;
    if !up {
        return None;
    }
    std::fs::read_to_string(format!("{base}/mtu")).ok()?.trim().parse().ok()
}

#[tokio::test]
async fn simulator_on_vcan0() {
    match iface_mtu() {
        Some(mtu) if mtu >= 72 => {}
        Some(mtu) => {
            eprintln!("skipped: {IFACE} has MTU {mtu}, not CAN FD capable (needs 72)");
            return;
        }
        None => {
            eprintln!("skipped: no {IFACE} that is up");
            return;
        }
    }
    let cfg = InterfaceConfig::Socketcan { ifname: IFACE.into() };
    let vbus = VirtualBus::new();
    let sim = Simulator::start_on(mt_can::open(&cfg, &vbus).await.expect("simulator socket"));
    let mut me = mt_can::open(&cfg, &vbus).await.expect("listener socket");
    let mut ev = me.take_events().unwrap();
    let dbc = Dbc::parse(SIM_DBC).unwrap();

    me.send(CanFrame::new(0x7DF, &[2, 1, 0x0C, 0, 0, 0, 0, 0]).unwrap()).await.unwrap();
    let (mut eec1, mut obd, mut pages) = (false, false, std::collections::BTreeSet::new());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline && !(eec1 && obd && pages.len() == 4) {
        let Ok(Some(BusEvent::Frame(f))) = tokio::time::timeout(Duration::from_millis(500), ev.recv()).await else { continue };
        if f.dir != Direction::Rx {
            continue;
        }
        let fr = &f.frame;
        if fr.extended && fr.id == 0x0CF0_0400 {
            assert!(!fr.fd, "J1939 stays classic CAN on the wire");
            eec1 = true;
        }
        if !fr.extended && fr.id == 0x7E8 {
            assert_eq!(&fr.data[..3], &[4, 0x41, 0x0C]);
            obd = true;
        }
        if !fr.extended && fr.id == 0x4A0 {
            assert!(fr.fd && fr.brs && fr.data.len() == 64, "battery pack frame went through the kernel as FD + BRS, 64 bytes");
            let (_, sigs) = dbc.decode(fr).unwrap();
            let page = sigs.iter().find(|s| s.name == "CellPage").unwrap().value as usize;
            pages.insert(page);
        }
    }
    assert!(eec1 && obd, "eec1 {eec1} obd {obd}");
    assert_eq!(pages.len(), 4, "all cell pages on {IFACE}: {pages:?}");

    sim.apply(SimAction::SetFdBattery { enabled: false }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    while ev.try_recv().is_ok() {}
    let until = tokio::time::Instant::now() + Duration::from_millis(400);
    while let Ok(Some(e)) = tokio::time::timeout_at(until, ev.recv()).await {
        if let BusEvent::Frame(f) = e {
            assert!(f.frame.id != 0x4A0 || f.frame.extended, "battery frame on {IFACE} while switched off");
        }
    }
    sim.stop().await;
    me.close().await;
}
