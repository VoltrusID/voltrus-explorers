//! EtherCrab, an EtherCAT master used with real hardware, against the simulated segment: if the simulator were
//! wrong in a way real devices are not, EtherCrab's init, mailbox or cyclic exchange would fail here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ethercrab::std::ethercat_now;
use ethercrab::{MainDevice, MainDeviceConfig, PduStorage, Timeouts};
use mt_ethercat::link::VirtualLink;
use mt_ethercat::sim::{Segment, Template};

const MAX_FRAMES: usize = 16;
const MAX_PDU_DATA: usize = PduStorage::element_size(1100);

fn start(t: Template) -> (Arc<MainDevice<'static>>, Arc<Mutex<Segment>>) {
    let storage: &'static PduStorage<MAX_FRAMES, MAX_PDU_DATA> = Box::leak(Box::new(PduStorage::new()));
    let (tx, rx, pdu_loop) = storage.try_split().expect("split once");
    let seg = Arc::new(Mutex::new(Segment::template(t)));
    let link = VirtualLink::new(seg.clone(), tx, rx, None);
    std::thread::spawn(move || futures_lite::future::block_on(link));
    let md = MainDevice::new(
        pdu_loop,
        // Generous PDU and EEPROM timeouts: the simulator runs on a shared, possibly busy test machine.
        Timeouts {
            wait_loop_delay: Duration::from_millis(1),
            mailbox_response: Duration::from_millis(500),
            pdu: Duration::from_millis(500),
            eeprom: Duration::from_millis(500),
            ..Default::default()
        },
        MainDeviceConfig { dc_static_sync_iterations: 200, ..Default::default() },
    );
    (Arc::new(md), seg)
}

async fn sleep(ms: u64) {
    async_io::Timer::after(Duration::from_millis(ms)).await;
}

#[test]
fn ethercrab_runs_the_simulated_machine() {
    futures_lite::future::block_on(async {
        let (md, seg) = start(Template::Machine);
        let group = md.init_single_group::<16, 256>(ethercat_now).await.expect("init");
        let names: Vec<String> = group.iter(&md).map(|s| s.name().to_string()).collect();
        assert_eq!(names, ["EK1100", "EL1008", "EL2008", "EL3102", "EL4102", "VD400", "VD400"]);

        let el3102 = group.subdevice(&md, 3).unwrap();
        assert_eq!(el3102.identity().product_id, 0x0C1E_3052);
        assert_eq!(el3102.sdo_read::<u32>(0x1018, 2).await.unwrap(), 0x0C1E_3052);
        assert_eq!(el3102.sdo_read::<u8>(0x1C13, 0).await.unwrap(), 2);
        // Every device got its configured address and reports PRE-OP.
        assert!(seg.lock().unwrap().devices.iter().all(|d| d.state() == mt_ethercat::esc::AlState::PreOp));

        let group = group.into_op(&md).await.expect("PRE-OP -> OP");
        let sizes: Vec<(usize, usize)> = group.iter(&md).map(|s| (s.io_raw().inputs().len(), s.io_raw().outputs().len())).collect();
        assert_eq!(sizes, [(0, 0), (1, 0), (0, 1), (8, 0), (0, 4), (20, 14), (20, 14)]);

        // LRW working counter: +1 for every device with inputs, +2 for every device with outputs.
        let expected: u16 = sizes.iter().map(|(i, o)| (*i > 0) as u16 + 2 * (*o > 0) as u16).sum();
        assert_eq!(expected, 12);
        let cycle = |n: usize| {
            let group = &group;
            let md = &md;
            async move {
                for _ in 0..n {
                    let r = group.tx_rx(md).await.expect("tx/rx");
                    assert_eq!(r.working_counter, expected);
                    sleep(2).await;
                }
            }
        };

        // EL2008 outputs 1 and 3 come back on EL1008 inputs 1 and 3; EL4102 ch.1 at half scale reaches EL3102 ch.2.
        group.subdevice(&md, 2).unwrap().outputs_raw_mut()[0] = 0b0000_0101;
        group.subdevice(&md, 4).unwrap().outputs_raw_mut()[..2].copy_from_slice(&16384i16.to_le_bytes());
        cycle(20).await;
        assert_eq!(group.subdevice(&md, 1).unwrap().inputs_raw()[0] & 0x0F, 0b0101);
        let ai = group.subdevice(&md, 3).unwrap();
        let ch2 = i16::from_le_bytes([ai.inputs_raw()[6], ai.inputs_raw()[7]]);
        assert!((ch2 - 16384).abs() < 40, "EL3102 ch.2 reads {ch2}");

        // CiA 402: shut down, switch on + enable in cyclic sync velocity, then 60 rpm.
        let drive = group.subdevice(&md, 5).unwrap();
        let status = || u16::from_le_bytes([drive.inputs_raw()[0], drive.inputs_raw()[1]]);
        {
            let mut o = drive.outputs_raw_mut();
            o[12] = 9;
            o[..2].copy_from_slice(&0x0006u16.to_le_bytes());
        }
        cycle(5).await;
        assert_eq!(status() & 0x6F, 0x21, "ready to switch on");
        drive.outputs_raw_mut()[..2].copy_from_slice(&0x000Fu16.to_le_bytes());
        cycle(5).await;
        assert_eq!(status() & 0x6F, 0x27, "operation enabled");
        drive.outputs_raw_mut()[6..10].copy_from_slice(&131_072i32.to_le_bytes());
        cycle(150).await;
        let vel = i32::from_le_bytes(drive.inputs_raw()[6..10].try_into().unwrap());
        assert_eq!(vel, 131_072, "velocity actual");
        assert_eq!(drive.inputs_raw()[18] as i8, 9, "mode display CSV");
    });
}

#[test]
fn watchdog_drops_outputs_when_cycles_stop() {
    futures_lite::future::block_on(async {
        let (md, seg) = start(Template::IoStation);
        let group = md.init_single_group::<16, 64>(ethercat_now).await.expect("init");
        let group = group.into_op(&md).await.expect("OP");
        group.tx_rx(&md).await.unwrap();
        // No process data for longer than the 100 ms watchdog: the output terminals fall back to SAFE-OP with 0x001B.
        sleep(150).await;
        let _ = group.tx_rx(&md).await;
        let s = seg.lock().unwrap();
        assert_eq!(s.devices[2].state(), mt_ethercat::esc::AlState::SafeOp);
        assert_eq!(s.devices[2].al_error(), Some(0x001B));
        // EL1008 has no outputs and no watchdog: still OP.
        assert_eq!(s.devices[1].state(), mt_ethercat::esc::AlState::Op);
    });
}
