//! Replay of recorded sessions.
//!
//! A capture (`tests/fixtures/NAME.capture.json`) holds the raw frames of one probe run; its
//! report (`NAME.report.json`) holds what the decoder made of them, as checked against the CNC
//! screen. The test starts a [`ReplayServer`] on the capture, runs the probe again through the
//! real client, and requires the same decoded result, every request found in the capture and
//! every recorded exchange played. A capture from a real controller therefore stays a regression
//! test for the decoder. See the README ("Field captures as regression tests").
//!
//! `UPDATE_FIXTURES=1 cargo test -p focas-proto --test replay` rewrites the reports from the
//! current decoder (review the diff against the CNC screen before committing it).
//! `cargo test -p focas-proto --test replay -- --ignored record_sim_fixtures` re-records the
//! simulator fixtures.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use focas_proto::probe::{self, ProbeOptions, ProbeReport};
use focas_proto::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The part of a report a replay must reproduce.
fn essence(report: &ProbeReport) -> Value {
    json!({
        "options": report.options,
        "aborted": report.aborted,
        "calls": report.calls,
        "screen": report.screen,
    })
}

/// Record a probe run against a live endpoint, as the `focas-probe` command does.
async fn record(host: &str, port: u16, options: &ProbeOptions) -> (ProbeReport, Capture) {
    let client = FocasClient::connect_with_capture(FocasConfig::new(host).with_port(port))
        .await
        .expect("connects");
    let report = probe::run(&client, options).await;
    client.set_capture_label("close");
    client.disconnect().await.expect("disconnects");
    let meta = BTreeMap::from([("tool".to_string(), "focas-proto test".to_string())]);
    (report, Capture::new(meta, client.take_exchanges()))
}

struct Replayed {
    report: ProbeReport,
    misses: Vec<String>,
    unplayed: usize,
}

/// Probe a [`ReplayServer`] serving `capture`.
async fn replay(capture: &Capture, options: &ProbeOptions) -> Replayed {
    let server = ReplayServer::start(capture).await.expect("replay starts");
    let client = FocasClient::connect(FocasConfig::new("127.0.0.1").with_port(server.port()))
        .await
        .expect("connects to the replay");
    let report = probe::run(&client, options).await;
    client.disconnect().await.expect("disconnects");
    let (misses, unplayed) = (server.misses(), server.unplayed());
    server.stop().await;
    Replayed {
        report,
        misses,
        unplayed,
    }
}

async fn start_sim(profile: CncProfile) -> FocasSimulator {
    FocasSimulator::start(SimulatorConfig {
        port: 0,
        profile,
        tick: None,
        ..Default::default()
    })
    .await
    .expect("simulator starts")
}

/// The machine states a field capture should cover, set up on the simulator.
fn scenarios() -> Vec<(&'static str, CncProfile, fn(&mut CncModel))> {
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
        m.raise_alarm(
            417,
            AlarmType::Servo,
            2,
            "SERVO ALARM: Y AXIS DIGITAL SERVO SYSTEM ERROR",
        );
    }
    fn emergency(m: &mut CncModel) {
        m.set_emergency(true);
    }
    fn mdi(m: &mut CncModel) {
        m.set_mode(AutoMode::Mdi);
    }
    vec![
        ("sim-31i-idle", CncProfile::Series31iB, idle),
        ("sim-31i-running", CncProfile::Series31iB, running),
        ("sim-31i-feed-hold", CncProfile::Series31iB, feed_hold),
        ("sim-31i-alarm", CncProfile::Series31iB, alarm),
        ("sim-31i-emergency", CncProfile::Series31iB, emergency),
        ("sim-31i-mdi", CncProfile::Series31iB, mdi),
        ("sim-0i-idle", CncProfile::Series0iF, idle),
    ]
}

fn assert_replays(name: &str, original: &ProbeReport, capture: &Capture, played: &Replayed) {
    assert_eq!(
        capture.problems(),
        Vec::<String>::new(),
        "{name}: malformed capture"
    );
    assert_eq!(
        played.misses,
        Vec::<String>::new(),
        "{name}: requests missing from the capture"
    );
    assert_eq!(
        played.unplayed, 0,
        "{name}: recorded exchanges the client did not ask for"
    );
    assert_eq!(
        serde_json::to_string_pretty(&essence(&played.report)).unwrap(),
        serde_json::to_string_pretty(&essence(original)).unwrap(),
        "{name}: replayed report differs"
    );
}

#[tokio::test]
async fn simulator_sessions_replay_exactly() {
    for (name, profile, setup) in scenarios() {
        let sim = start_sim(profile).await;
        sim.with_model(setup);
        let options = ProbeOptions {
            list_programs: true,
            ..Default::default()
        };
        let (report, capture) = record("127.0.0.1", sim.port(), &options).await;
        sim.stop().await;

        assert_eq!(report.aborted, None, "{name}");
        assert_eq!(report.failed(), 0, "{name}: {}", report.to_text());
        // open, sysinfo, every call, close
        assert!(capture.exchanges.len() > report.calls.len(), "{name}");
        assert_eq!(capture.exchanges[0].label, "open");
        assert_eq!(capture.exchanges.last().unwrap().label, "close");

        let played = replay(&capture, &options).await;
        assert_replays(name, &report, &capture, &played);

        // and through the JSON files a field capture travels in
        let capture = Capture::from_json(&capture.to_json()).unwrap();
        let reloaded = ProbeReport::from_json(&report.to_json()).unwrap();
        let played = replay(&capture, &reloaded.options).await;
        assert_replays(name, &reloaded, &capture, &played);
    }
}

#[tokio::test]
async fn simulator_states_show_in_the_report() {
    let get = |report: &ProbeReport, item: &str| -> String {
        report
            .screen
            .iter()
            .find(|l| l.item == item)
            .unwrap_or_else(|| panic!("no screen line '{item}'"))
            .value
            .clone()
    };
    let mut seen = BTreeMap::new();
    for (name, profile, setup) in scenarios() {
        let sim = start_sim(profile).await;
        sim.with_model(setup);
        let (report, _) = record("127.0.0.1", sim.port(), &ProbeOptions::default()).await;
        sim.stop().await;
        seen.insert(name, report);
    }
    assert_eq!(get(&seen["sim-31i-idle"], "run state"), "****");
    assert_eq!(get(&seen["sim-31i-running"], "run state"), "STRT");
    assert_eq!(get(&seen["sim-31i-feed-hold"], "run state"), "HOLD");
    assert_eq!(get(&seen["sim-31i-emergency"], "emergency"), "EMG");
    assert_eq!(get(&seen["sim-31i-mdi"], "mode"), "MDI");
    assert!(get(&seen["sim-31i-alarm"], "alarm PS 100").contains("PARAMETER WRITE ENABLE"));
    assert!(get(&seen["sim-31i-alarm"], "alarm SV 417").starts_with("axis 2"));
    assert!(get(&seen["sim-0i-idle"], "controller").contains("series 0 "));
}

/// Every capture in `tests/fixtures` against its report.
#[tokio::test]
async fn committed_captures_replay() {
    let update = std::env::var_os("UPDATE_FIXTURES").is_some();
    let mut names: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("tests/fixtures exists")
        .filter_map(|e| {
            let file = e.ok()?.file_name().into_string().ok()?;
            file.strip_suffix(".capture.json").map(str::to_string)
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no *.capture.json in tests/fixtures");

    for name in names {
        let capture = Capture::load(&fixtures_dir().join(format!("{name}.capture.json")))
            .unwrap_or_else(|e| panic!("{e}"));
        let report_path = fixtures_dir().join(format!("{name}.report.json"));
        let expected = std::fs::read_to_string(&report_path)
            .map_err(|e| format!("{}: {e}", report_path.display()))
            .and_then(|t| ProbeReport::from_json(&t));
        // A new capture without a report gets its options from the defaults.
        let options = match &expected {
            Ok(r) => r.options.clone(),
            Err(_) if update => ProbeOptions::default(),
            Err(e) => panic!("{name}: {e}"),
        };
        let played = replay(&capture, &options).await;
        if update {
            let mut report = played.report.clone();
            report.host = capture.meta.get("host").cloned().unwrap_or_default();
            report.port = capture
                .meta
                .get("port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(DEFAULT_PORT);
            std::fs::write(&report_path, report.to_json() + "\n").expect("report written");
            continue;
        }
        assert_replays(&name, &expected.unwrap(), &capture, &played);
    }
}

/// A request the capture does not hold is reported, not answered.
#[tokio::test]
async fn unknown_requests_are_misses() {
    let sim = start_sim(CncProfile::Series31iB).await;
    let (_, mut capture) = record("127.0.0.1", sim.port(), &ProbeOptions::default()).await;
    sim.stop().await;
    capture.exchanges.retain(|e| e.label != "statinfo");

    let played = replay(&capture, &ProbeOptions::default()).await;
    assert!(!played.misses.is_empty());
    let statinfo = played
        .report
        .calls
        .iter()
        .find(|c| c.name == "statinfo")
        .unwrap();
    assert!(!statinfo.ok);
    assert!(
        played.report.aborted.is_some(),
        "the dropped connection stops the probe"
    );
}

/// A recorded dropped connection plays back as one.
#[tokio::test]
async fn silence_in_a_capture_drops_the_connection() {
    let sim = start_sim(CncProfile::Series31iB).await;
    let (_, mut capture) = record("127.0.0.1", sim.port(), &ProbeOptions::default()).await;
    sim.stop().await;
    for e in capture
        .exchanges
        .iter_mut()
        .filter(|e| e.label == "actual feed")
    {
        e.response = None;
        e.error = Some("no answer within 5000 ms".into());
    }
    let played = replay(&capture, &ProbeOptions::default()).await;
    let feed = played
        .report
        .calls
        .iter()
        .find(|c| c.name == "actual feed")
        .unwrap();
    assert!(!feed.ok);
    assert!(
        played
            .report
            .aborted
            .as_deref()
            .unwrap()
            .contains("actual feed")
    );
}

/// A controller that answers the open request with something else still yields a capture.
#[tokio::test]
async fn failed_connect_keeps_the_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 12];
        let _ = s.read(&mut buf).await;
        s.write_all(b"HTTP/1.0 400 Bad Request\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let failure = FocasClient::connect_with_capture(FocasConfig::new("127.0.0.1").with_port(port))
        .await
        .err()
        .expect("the connect fails");
    assert!(
        matches!(failure.error, FocasError::ConnectionFailed(_)),
        "{failure}"
    );
    assert_eq!(failure.exchanges.len(), 1);
    let open = &failure.exchanges[0];
    assert_eq!(open.label, "open");
    assert_eq!(open.request, "a0 a0 a0 a0 00 01 01 01 00 02 00 02");
    // the bytes that were not a frame are kept: the header-sized prefix
    assert_eq!(
        open.response.as_deref(),
        Some("48 54 54 50 2f 31 2e 30 20 34")
    );
    assert!(open.error.is_some());
}

/// Re-record the simulator fixtures (needs `--ignored`).
#[tokio::test]
#[ignore = "rewrites tests/fixtures; run on purpose"]
async fn record_sim_fixtures() {
    std::fs::create_dir_all(fixtures_dir()).unwrap();
    for (name, profile, setup) in scenarios() {
        let sim = start_sim(profile).await;
        sim.with_model(setup);
        let port = sim.port();
        let (mut report, mut capture) = record("127.0.0.1", port, &ProbeOptions::default()).await;
        sim.stop().await;
        report.host = "simulator".into();
        report.port = DEFAULT_PORT;
        for (k, v) in [
            ("source", "focas-proto simulator"),
            ("scenario", name),
            ("host", "simulator"),
            ("port", "8193"),
        ] {
            capture.meta.insert(k.into(), v.into());
        }
        // timings of the recording machine mean nothing in a fixture
        for e in &mut capture.exchanges {
            e.elapsed_ms = 0;
        }
        std::fs::write(
            fixtures_dir().join(format!("{name}.capture.json")),
            capture.to_json() + "\n",
        )
        .unwrap();
        std::fs::write(
            fixtures_dir().join(format!("{name}.report.json")),
            report.to_json() + "\n",
        )
        .unwrap();
    }
}
