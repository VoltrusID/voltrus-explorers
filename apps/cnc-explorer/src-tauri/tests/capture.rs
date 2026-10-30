//! "Save capture" and the extended poll against the CNC simulator, driven through the same `Core`
//! functions the Tauri commands call.
//!
//! The saved files are the ones opex-flow's `crates/focas-proto/tests/replay.rs` replays. Set
//! `CNC_EXPLORER_CAPTURE_OUT=<dir>` to keep what this test records (`explorer-sim-31i-<state>.*`),
//! for copying into that crate's `tests/fixtures`.

use std::path::PathBuf;
use std::sync::Arc;

use cnc_explorer_lib::capture::{self, MachineState};
use cnc_explorer_lib::core::Core;
use focas_proto::probe::ProbeReport;
use focas_proto::{AlarmType, AutoMode, Capture, CncModel, CncProfile, FocasConfig, FocasSimulator, SimulatorConfig};

fn core() -> Core {
    Core::new(Arc::new(|_, _| {}))
}

async fn sim(setup: fn(&mut CncModel)) -> FocasSimulator {
    let sim = FocasSimulator::start(SimulatorConfig {
        port: 0,
        profile: CncProfile::Series31iB,
        tick: None,
        ..Default::default()
    })
    .await
    .expect("simulator starts");
    sim.with_model(setup);
    sim
}

async fn connect(core: &Core, sim: &FocasSimulator) {
    core.fanuc_connect(FocasConfig::new("127.0.0.1").with_port(sim.port()))
        .await
        .expect("connects");
}

fn states() -> Vec<(MachineState, fn(&mut CncModel))> {
    fn idle(_: &mut CncModel) {}
    fn running(m: &mut CncModel) {
        assert!(m.cycle_start());
        m.advance(0.5);
    }
    fn feed_hold(m: &mut CncModel) {
        assert!(m.cycle_start());
        m.advance(0.5);
        m.feed_hold();
    }
    fn alarm(m: &mut CncModel) {
        m.raise_alarm(100, AlarmType::ForegroundPs, 0, "PARAMETER WRITE ENABLE");
    }
    fn estop(m: &mut CncModel) {
        m.set_emergency(true);
    }
    fn mdi(m: &mut CncModel) {
        m.set_mode(AutoMode::Mdi);
    }
    vec![
        (MachineState::Idle, idle),
        (MachineState::Running, running),
        (MachineState::FeedHold, feed_hold),
        (MachineState::Alarm, alarm),
        (MachineState::EStop, estop),
        (MachineState::Mdi, mdi),
    ]
}

#[tokio::test]
async fn saved_captures_replay_in_every_machine_state() {
    let keep = std::env::var_os("CNC_EXPLORER_CAPTURE_OUT").map(PathBuf::from);
    if let Some(dir) = &keep {
        std::fs::create_dir_all(dir).unwrap();
    }
    for (state, setup) in states() {
        let sim = sim(setup).await;
        let core = core();
        connect(&core, &sim).await;
        let bundle = core
            .fanuc_capture(state, vec![500, 501], "1.2.3")
            .await
            .unwrap_or_else(|e| panic!("{state:?}: {e}"));
        sim.stop().await;

        let name = state.slug();
        assert_eq!(bundle.base_name, format!("127.0.0.1-{name}"));
        assert!(bundle.replay_ok, "{name}: {:?}", bundle.notes);
        assert!(bundle.notes.is_empty(), "{name}: {:?}", bundle.notes);
        assert_eq!(bundle.failed_calls, 0, "{name}: {}", bundle.summary);
        assert_eq!(bundle.aborted, None);
        assert!(bundle.capture_json.ends_with("}\n") && bundle.report_json.ends_with("}\n"));

        // the files are what the harness loads
        let capture = Capture::from_json(&bundle.capture_json).unwrap();
        let report = ProbeReport::from_json(&bundle.report_json).unwrap();
        assert_eq!(capture.problems(), Vec::<String>::new());
        assert_eq!(capture.exchanges.first().unwrap().label, "open");
        assert_eq!(capture.exchanges.last().unwrap().label, "close");
        assert_eq!(capture.exchanges.len(), bundle.exchanges);
        assert_eq!(capture.meta["machine_state"], name);
        assert_eq!(capture.meta["host"], "127.0.0.1");
        assert!(capture.meta["probed_at"].ends_with('Z'));
        assert_eq!(capture.meta["tool"], "CNC Explorer 1.2.3 (focas-proto)");
        assert!(capture.meta["controller"].contains("series G421"));
        assert!(report.options.macros.starts_with(&[100, 500, 501]));
        assert_eq!(report.options.macros.iter().filter(|n| **n == 500).count(), 1);
        capture::replays(&capture, &report).await.unwrap();

        // the summary carries what to compare with the screen, the state and the privacy note
        assert!(bundle.summary.contains("Compare with the CNC screen"));
        assert!(bundle.summary.contains(&format!("Machine state: {}", state.label())));
        assert!(bundle.summary.contains("Privacy"));
        assert!(bundle.summary.contains("program"));

        if let Some(dir) = &keep {
            let stem = format!("explorer-sim-31i-{name}");
            std::fs::write(dir.join(format!("{stem}.capture.json")), &bundle.capture_json).unwrap();
            std::fs::write(dir.join(format!("{stem}.report.json")), &bundle.report_json).unwrap();
            std::fs::write(dir.join(format!("{stem}.summary.txt")), &bundle.summary).unwrap();
        }
    }
}

#[tokio::test]
async fn a_wrong_label_is_flagged() {
    fn idle(_: &mut CncModel) {}
    let sim = sim(idle).await;
    let core = core();
    connect(&core, &sim).await;
    let bundle = core.fanuc_capture(MachineState::Running, vec![], "1.2.3").await.unwrap();
    sim.stop().await;
    assert!(bundle.replay_ok);
    assert_eq!(bundle.notes.len(), 1, "{:?}", bundle.notes);
    assert!(bundle.notes[0].contains("Labelled \"Running\""), "{}", bundle.notes[0]);
    assert!(bundle.summary.contains("Labelled \"Running\""));
}

#[tokio::test]
async fn capture_needs_a_connection() {
    let err = core().fanuc_capture(MachineState::Idle, vec![], "1.2.3").await.unwrap_err();
    assert_eq!(err, "Not connected");
}

#[tokio::test]
async fn poll_reads_servo_load_program_name_and_watched_macros_in_one_extended_read() {
    fn idle(_: &mut CncModel) {}
    let sim = sim(idle).await;
    sim.with_model(|m| m.set_macro(700, 12.5));
    let core = core();
    connect(&core, &sim).await;

    let poll = core.fanuc_poll(vec![700, 0, 701]).await.unwrap();
    assert!(poll.program.name.is_some(), "program name");
    assert!(poll.axes.iter().all(|a| a.servo_load.is_some()), "servo load");
    // 0 is dropped; 700 has a value, 701 does not exist in the model and is refused or empty
    let numbers: Vec<u32> = poll.macros.iter().map(|m| m.number).collect();
    assert_eq!(numbers, [700, 701]);
    assert_eq!(poll.macros[0].value, Some(12.5));
    assert_eq!(poll.macros[1].value, None);

    let none = core.fanuc_poll(vec![]).await.unwrap();
    assert!(none.macros.is_empty());
    sim.stop().await;
}
