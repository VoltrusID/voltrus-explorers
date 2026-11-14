//! The explorer's session API end to end on the simulated machine.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_ethercat::esc::AlState;
use mt_ethercat::master::{LinkSpec, Session, SessionEvent, extract_bits};
use mt_ethercat::sim::Template;

fn sink() -> (Arc<dyn Fn(SessionEvent) + Send + Sync>, Arc<Mutex<Vec<SessionEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (Arc::new(move |e| s.lock().unwrap().push(e)), seen)
}

async fn sleep(ms: u64) {
    async_io::Timer::after(Duration::from_millis(ms)).await;
}

#[test]
fn scan_describe_and_browse() {
    futures_lite::future::block_on(async {
        let (emit, _) = sink();
        let (s, scan) = Session::open(LinkSpec::Simulator { template: Template::Machine }, emit).await.expect("open");
        assert!(scan.init_error.is_none());
        assert_eq!(scan.devices.len(), 7);
        let ek = &scan.devices[0];
        assert_eq!((ek.name.as_str(), ek.vendor_name, ek.esc_name), ("EK1100", Some("Beckhoff Automation"), "ET1100"));
        assert_eq!(ek.description, "EK1100 EtherCAT Coupler (2A E-Bus)");
        assert_eq!(ek.state, "PRE-OP");
        assert!(ek.ports[0].communication && ek.ports[1].link);
        let last = &scan.devices[6];
        assert!(!last.ports[1].link && last.ports[1].loop_closed, "end of the line closes port 1");
        assert_eq!(scan.devices[3].mailbox, vec!["CoE"]);
        assert!(scan.devices[3].sdo_info);
        assert_eq!(scan.devices[5].dc, "64-bit");
        assert_eq!(scan.devices[1].dc, "reference only");
        // Propagation delays grow along the line.
        assert!(scan.devices[6].propagation_delay_ns > scan.devices[1].propagation_delay_ns);

        // Registers: station address and AL status decoded.
        let regs = s.registers(3).await.unwrap();
        let station = regs.registers.iter().find(|r| r.addr == 0x0010).unwrap();
        assert_eq!(station.value, scan.devices[3].address as u64);
        assert_eq!(regs.registers.iter().find(|r| r.addr == 0x0130).unwrap().decoded, "PRE-OP");
        assert!(regs.sync_managers[0].enabled && regs.sync_managers[0].mode == "mailbox");

        // PDO layouts: EL1008 from SII, EL3102 and the drive from CoE.
        let di = s.pdo_map(1).await.unwrap();
        assert_eq!((di.source, di.inputs.len()), ("SII", 8));
        let ai = s.pdo_map(3).await.unwrap();
        assert_eq!(ai.source, "CoE");
        let value = ai.inputs[1].entries.iter().find(|e| e.name == "Value").unwrap();
        // Offsets run over the device's whole input image: channel 2's value follows 32 bits of channel 1.
        assert_eq!((value.bit_offset, value.bits, value.type_name.as_str()), (48, 16, "INT"));
        let drive = s.pdo_map(5).await.unwrap();
        assert_eq!(drive.outputs[0].entries[0].name, "Controlword");
        assert_eq!(drive.inputs[0].entries.iter().map(|e| e.bits as usize).sum::<usize>(), 160);

        // Object dictionary with names over SDO Info (the list takes several mailbox fragments on the drive).
        let objs = s.object_list(5).await.unwrap();
        assert!(objs.len() > 25);
        assert_eq!(objs.iter().find(|o| o.index == 0x6041).unwrap().name, "Statusword");
        let ident = s.object_entries(5, 0x1018).await.unwrap();
        assert_eq!(ident[2].name, "Product code");
        assert_eq!(ident[2].value.as_deref(), Some("1024"));
        // A segmented upload: the device name is longer than an expedited transfer.
        let name = s.sdo_upload(3, 0x1008, 0).await.unwrap();
        assert_eq!(name, b"EL3102");
        // Abort codes come back as text.
        let err = s.sdo_upload(3, 0x2222, 0).await.unwrap_err();
        assert!(err.contains("0x06020000"), "{err}");
        let err = s.sdo_download(3, 0x1018, 2, &[1, 2, 3, 4]).await.unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        s.close();
    });
}

#[test]
fn long_sdo_transfers_segment() {
    futures_lite::future::block_on(async {
        let (emit, _) = sink();
        let (s, _) = Session::open(LinkSpec::Simulator { template: Template::Drives }, emit).await.expect("open");
        // Rewrite the drive's alternative TxPDO mapping with a 7-entry list (28 bytes of data through a
        // 128-byte mailbox goes in one normal transfer) and read it back.
        s.sdo_download(0, 0x1A01, 0, &[0]).await.unwrap();
        s.sdo_download(0, 0x1A01, 0, &[5]).await.unwrap();
        assert_eq!(s.sdo_upload(0, 0x1A01, 0).await.unwrap(), vec![5]);
        assert!(s.sdo_download(0, 0x1A01, 0, &[9]).await.unwrap_err().contains("0x06090031"));
        s.close();
    });
}

#[test]
fn states_process_data_and_capture() {
    futures_lite::future::block_on(async {
        let (emit, seen) = sink();
        let (s, _) = Session::open(LinkSpec::Simulator { template: Template::Machine }, emit).await.expect("open");
        s.set_cycle_ms(2.0);
        let v = s.request_state(AlState::SafeOp, vec![]).await.unwrap();
        assert!(v.iter().all(|d| d.state == "SAFE-OP" && d.al_code.is_none()), "{v:?}");
        assert!(s.cycling());
        let v = s.request_state(AlState::Op, vec![]).await.unwrap();
        assert!(v.iter().all(|d| d.state == "OP"), "{v:?}");

        // EL2008 output 2 (bit 1) loops back to EL1008 input 2.
        s.write_output(2, 1, 1, 1).unwrap();
        sleep(60).await;
        let img = s.images();
        assert_eq!(extract_bits(&img[1].inputs, 1, 1), 1);
        let stats = s.stats();
        assert!(stats.cycles > 5 && stats.wkc == stats.expected_wkc && stats.expected_wkc == 12, "{stats:?}");

        // A device that refuses a state shows why: EL3102 back to PRE-OP, then its TxPDO assignment broken.
        s.request_state(AlState::PreOp, vec![3]).await.unwrap();
        s.sdo_download(3, 0x1C13, 1, &0x1A07u16.to_le_bytes()).await.unwrap();
        let v = s.request_state(AlState::SafeOp, vec![3]).await.unwrap();
        assert_eq!(v[0].al_code, Some(0x0024), "{v:?}");
        assert!(v[0].al_text.as_deref().unwrap().contains("Invalid input mapping"));

        // Frame capture on the simulator link.
        s.capture(10).unwrap();
        sleep(50).await;
        let frames: usize = seen
            .lock()
            .unwrap()
            .iter()
            .map(|e| if let SessionEvent::Frames { frames } = e { frames.len() } else { 0 })
            .sum();
        assert_eq!(frames, 10);
        assert!(seen.lock().unwrap().iter().any(|e| matches!(e, SessionEvent::Io { .. })));
        s.close();
    });
}

#[test]
fn pulled_cable_is_seen() {
    futures_lite::future::block_on(async {
        let (emit, _) = sink();
        let (s, _) = Session::open(LinkSpec::Simulator { template: Template::IoStation }, emit).await.expect("open");
        s.set_cycle_ms(2.0);
        // Straight from PRE-OP: the session steps through SAFE-OP.
        let v = s.request_state(AlState::Op, vec![]).await.unwrap();
        assert!(v.iter().all(|d| d.state == "OP"), "{v:?}");
        sleep(20).await;
        s.segment.as_ref().unwrap().lock().unwrap().action(&mt_ethercat::sim::SimAction::BreakCable { after: Some(2) }).unwrap();
        sleep(40).await;
        let st = s.stats();
        assert!(st.wkc < st.expected_wkc, "{st:?}");
        let states = s.states().await;
        assert_eq!(states[4].state, "no answer");
        assert_eq!(states[2].state, "OP");
        let regs = s.registers(2).await.unwrap();
        assert_eq!(regs.error_counters.iter().find(|(n, _)| n == "Port 1 lost links").unwrap().1, 1);
        s.reset_error_counters(2).await.unwrap();
        let regs = s.registers(2).await.unwrap();
        assert!(regs.error_counters.iter().all(|(_, v)| *v == 0));
        s.close();
    });
}

/// The app reads every device's PDO mapping over CoE right after a scan while the user may already ask for OP:
/// EtherCrab's own CoE reads (mapping process data) and the explorer's SDO reads must not take each other's answers.
#[test]
fn mailbox_users_do_not_collide() {
    futures_lite::future::block_on(async {
        let (emit, _) = sink();
        let (s, _) = Session::open(LinkSpec::Simulator { template: Template::Machine }, emit).await.expect("open");
        s.set_cycle_ms(2.0);
        let reads = async {
            for p in 0..7 {
                s.pdo_map(p).await.unwrap();
            }
        };
        let (_, states) = futures_lite::future::zip(reads, s.request_state(AlState::Op, vec![])).await;
        let states = states.unwrap();
        assert!(states.iter().all(|d| d.state == "OP"), "{states:?}");
        s.close();
    });
}

/// The app's order: read every device's PDO mapping (the explorer's own CoE traffic), then ask for OP. EtherCrab
/// keeps its own mailbox counter; a device drops a message whose counter repeats the last one it saw.
#[test]
fn op_after_reading_mappings() {
    futures_lite::future::block_on(async {
        let (emit, _) = sink();
        let (s, _) = Session::open(LinkSpec::Simulator { template: Template::Machine }, emit).await.expect("open");
        s.set_cycle_ms(2.0);
        for p in 0..7 {
            s.pdo_map(p).await.unwrap();
        }
        let states = s.request_state(AlState::Op, vec![]).await.unwrap();
        assert!(states.iter().all(|d| d.state == "OP"), "{states:?}");
        s.close();
    });
}
