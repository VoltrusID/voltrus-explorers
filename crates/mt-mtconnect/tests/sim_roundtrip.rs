//! The client against the in-process simulator, plus SHDR adapter behaviour.

use std::collections::HashSet;
use std::time::Duration;

use mt_mtconnect::{
    AgentClient, AgentConfig, AgentErrorCode, AgentSimulator, Category, ConditionLevel, DeviceTemplate,
    MtConnectError, ResyncReason, SampleQuery, SimConfig, StreamEvent, StreamOptions, StreamTransport, Streams,
    Value,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

async fn start(template: DeviceTemplate, buffer_size: usize) -> (AgentSimulator, AgentClient) {
    let sim = AgentSimulator::start(SimConfig {
        bind_addr: "127.0.0.1".into(),
        http_port: 0,
        shdr_port: Some(0),
        template,
        buffer_size,
        // The program is advanced by hand in these tests.
        tick_ms: 3_600_000,
        shdr_heartbeat_ms: 5000,
    })
    .await
    .unwrap();
    let client = AgentClient::from_url(&sim.base_url()).unwrap();
    (sim, client)
}

async fn next_event(s: &mut mt_mtconnect::SampleStream) -> StreamEvent {
    tokio::time::timeout(Duration::from_secs(10), s.next())
        .await
        .expect("stream event within 10s")
        .expect("stream still open")
        .expect("no stream error")
}

fn has(streams: &Streams, id: &str) -> bool {
    streams.observations().any(|o| o.data_item_id == id)
}

#[tokio::test]
async fn probe_returns_the_template_device() {
    for template in [DeviceTemplate::Vmc3Axis, DeviceTemplate::Lathe2Axis] {
        let (sim, client) = start(template, 1000).await;
        let probe = client.probe().await.unwrap();
        assert_eq!(probe.header.buffer_size, Some(1000));
        assert_eq!(probe.header.instance_id, sim.model().snapshot().instance_id);
        assert_eq!(probe.devices.len(), 1);
        assert_eq!(probe.devices[0], sim.model().device(), "XML round trip is lossless");
        let items = probe.devices[0].flatten_data_items();
        let ids: HashSet<&str> = items.iter().map(|(_, i)| i.id.as_str()).collect();
        for needed in ["avail", "estop", "Xpos", "Zpos", "Sspeed", "Sload", "pfeed", "exec", "mode", "program", "pcount"] {
            assert!(ids.contains(needed), "{template:?} lacks {needed}");
        }
        assert_eq!(ids.contains("Ypos"), template == DeviceTemplate::Vmc3Axis);
        let x = probe.data_item("Xpos").unwrap();
        assert_eq!((x.units.as_deref(), x.sub_type.as_deref()), (Some("MILLIMETER"), Some("ACTUAL")));
        sim.stop().await;
    }
}

#[tokio::test]
async fn current_with_filters_and_conditions() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 1000).await;
    let model = sim.model();
    for _ in 0..20 {
        model.tick(0.25);
    }
    let all = client.current(None).await.unwrap();
    let total = model.device().flatten_data_items().len();
    assert_eq!(all.len(), total, "one observation per data item");
    assert_eq!(all.header.instance_id, model.snapshot().instance_id);
    let avail = all.observations().find(|o| o.data_item_id == "avail").unwrap();
    assert_eq!(avail.value, Value::Text("AVAILABLE".into()));
    assert_eq!(avail.element, "Availability");

    let axes = client.current(Some("//Axes")).await.unwrap();
    let ids: HashSet<&str> = axes.observations().map(|o| o.data_item_id.as_str()).collect();
    assert_eq!(ids, HashSet::from(["Xpos", "Ypos", "Zpos", "Sspeed", "Sload"]));

    let conds = client.current(Some("//DataItem[@category=\"CONDITION\"]")).await.unwrap();
    assert_eq!(conds.len(), 2);
    assert!(conds.observations().all(|o| o.category == Category::Condition));

    // A fault appears with its native code and message, then clears.
    model.raise_condition("motion_cond", ConditionLevel::Fault, "E42", "Servo overload").unwrap();
    let c = client.current(Some("//Path")).await.unwrap();
    let fault = c.observations().find(|o| o.data_item_id == "motion_cond").unwrap();
    let cond = fault.condition.as_ref().unwrap();
    assert_eq!(cond.level, ConditionLevel::Fault);
    assert_eq!(cond.native_code.as_deref(), Some("E42"));
    assert_eq!(cond.message.as_deref(), Some("Servo overload"));
    assert_eq!(fault.data_item_type.as_deref(), Some("MOTION_PROGRAM"));
    model.clear_condition("motion_cond").unwrap();
    let c = client.current(Some("//Path")).await.unwrap();
    let normal = c.observations().find(|o| o.data_item_id == "motion_cond").unwrap();
    assert_eq!(normal.condition.as_ref().unwrap().level, ConditionLevel::Normal);

    // Device-scoped client.
    let scoped = AgentClient::new(AgentConfig {
        device: Some("VMC".into()),
        ..AgentConfig::new(sim.base_url())
    })
    .unwrap();
    assert_eq!(scoped.current(None).await.unwrap().len(), total);
    sim.stop().await;
}

#[tokio::test]
async fn sample_pages_are_contiguous_and_errors_are_typed() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 1000).await;
    let model = sim.model();
    for _ in 0..200 {
        model.tick(0.25);
    }
    let snap = model.snapshot();

    let page1 = client.sample(snap.first_sequence, 10).await.unwrap();
    let seqs: Vec<u64> = page1.observations_by_sequence().iter().map(|o| o.sequence).collect();
    assert_eq!(seqs.len(), 10);
    assert_eq!(seqs[0], snap.first_sequence);
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(page1.header.first_sequence, Some(snap.first_sequence));
    assert_eq!(page1.header.last_sequence, Some(snap.last_sequence));
    let next = page1.header.next_sequence.unwrap();
    assert_eq!(next, seqs[9] + 1);

    let page2 = client.sample(next, 10).await.unwrap();
    assert_eq!(page2.observations_by_sequence()[0].sequence, next);

    // From the end: an empty document whose nextSequence stays put.
    let empty = client.sample(snap.next_sequence, 10).await.unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.header.next_sequence, Some(snap.next_sequence));

    // Path-filtered sample only carries matching items.
    let filtered = client
        .sample_with(&SampleQuery { path: Some("//Linear".into()), from: Some(snap.first_sequence), count: Some(500) })
        .await
        .unwrap();
    assert!(filtered.observations().all(|o| ["Xpos", "Ypos", "Zpos"].contains(&o.data_item_id.as_str())));
    assert!(!filtered.is_empty());

    // Typed agent errors.
    let e = client.sample_with(&SampleQuery { count: Some(0), ..Default::default() }).await.unwrap_err();
    assert!(e.is_out_of_range());
    let e = client.current(Some("//Axes[")).await.unwrap_err();
    match e {
        MtConnectError::Agent(a) => assert!(a.has_code(&AgentErrorCode::InvalidXPath), "{a}"),
        other => panic!("{other:?}"),
    }
    let missing = AgentClient::new(AgentConfig { device: Some("nope".into()), ..AgentConfig::new(sim.base_url()) }).unwrap();
    match missing.probe().await.unwrap_err() {
        MtConnectError::Agent(a) => assert_eq!(a.errors[0].code, AgentErrorCode::NoDevice),
        other => panic!("{other:?}"),
    }
    sim.stop().await;
}

#[tokio::test]
async fn sample_out_of_range_after_buffer_wraps() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 32).await;
    let model = sim.model();
    for _ in 0..400 {
        model.tick(0.25);
    }
    let snap = model.snapshot();
    assert_eq!(snap.last_sequence - snap.first_sequence + 1, 32);
    let err = client.sample(1, 10).await.unwrap_err();
    assert!(err.is_out_of_range(), "{err}");
    match err {
        MtConnectError::Agent(a) => {
            assert_eq!(a.header.first_sequence, None, "error header has no sequence info");
            assert!(a.errors[0].message.contains("'from'"));
        }
        other => panic!("{other:?}"),
    }
    // Re-sync from current works.
    let cur = client.current(None).await.unwrap();
    assert_eq!(cur.header.first_sequence, Some(snap.first_sequence));
    assert!(client.sample(cur.header.next_sequence.unwrap(), 5).await.is_ok());
    sim.stop().await;
}

#[tokio::test]
async fn assets_list_and_raw_xml() {
    let (sim, client) = start(DeviceTemplate::Lathe2Axis, 100).await;
    let list = client.assets().await.unwrap();
    assert_eq!(list.assets.len(), 3);
    assert_eq!(list.header.asset_count, Some(3));
    for a in &list.assets {
        assert_eq!(a.asset_type, "CuttingTool");
        assert!(a.raw_xml.starts_with("<CuttingTool assetId=\""));
        assert!(a.raw_xml.ends_with("</CuttingTool>"));
    }
    let one = client.asset("TOOL.2").await.unwrap();
    assert_eq!(one, list.assets[1]);
    match client.asset("TOOL.99").await.unwrap_err() {
        MtConnectError::Agent(a) => assert_eq!(a.errors[0].code, AgentErrorCode::AssetNotFound),
        other => panic!("{other:?}"),
    }
    assert_eq!(client.assets_filtered(Some("File"), None, false).await.unwrap().assets.len(), 0);
    assert!(sim.model().remove_asset("TOOL.1"));
    assert_eq!(client.assets().await.unwrap().assets.len(), 2);
    sim.stop().await;
}

#[tokio::test]
async fn stream_round_trip_over_multipart() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 2000).await;
    let model = sim.model().clone();
    let mut stream = client.stream_sample(Duration::from_millis(50));

    let StreamEvent::Snapshot(snapshot) = next_event(&mut stream).await else { panic!("expected snapshot") };
    assert_eq!(snapshot.len(), model.device().flatten_data_items().len());

    // Wait until the multipart connection is up, then change the machine.
    for _ in 0..100 {
        if stream.transport() == StreamTransport::Streaming {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(stream.transport(), StreamTransport::Streaming);

    model.set_value("Sspeed", Value::Number(4321.0)).unwrap();
    model.raise_condition("sys_cond", ConditionLevel::Warning, "W7", "Coolant low").unwrap();
    let mut got_speed = false;
    let mut got_warning = false;
    while !(got_speed && got_warning) {
        let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("unexpected resync") };
        for o in s.observations() {
            got_speed |= o.data_item_id == "Sspeed" && o.value == Value::Number(4321.0);
            got_warning |= o.condition.as_ref().is_some_and(|c| c.level == ConditionLevel::Warning && c.native_code.as_deref() == Some("W7"));
            assert!(o.sequence >= snapshot.header.next_sequence.unwrap(), "no replay of the snapshot");
        }
    }

    // Running program values keep flowing.
    // (the program leaves its 3 s idle phase after 12 quarter-second ticks)
    for _ in 0..20 {
        model.tick(0.25);
    }
    let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("unexpected resync") };
    assert!(!s.is_empty());
    sim.stop().await;
}

#[tokio::test]
async fn stream_falls_back_to_polling() {
    let (sim, client) = start(DeviceTemplate::Lathe2Axis, 2000).await;
    let model = sim.model().clone();
    let mut opts = StreamOptions::new(Duration::from_millis(30));
    opts.prefer_streaming = false;
    let mut stream = client.stream_sample_with(opts);
    assert!(matches!(next_event(&mut stream).await, StreamEvent::Snapshot(_)));
    model.set_value("Sload", Value::Number(55.5)).unwrap();
    let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("unexpected resync") };
    assert!(has(&s, "Sload"));
    assert_eq!(stream.transport(), StreamTransport::Polling);

    // More than one page of backlog is drained without waiting for the interval.
    for _ in 0..150 {
        model.tick(0.25);
    }
    let mut seen = 0;
    while seen < 50 {
        let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("unexpected resync") };
        seen += s.len();
    }
    sim.stop().await;
}

#[tokio::test]
async fn stream_resyncs_on_out_of_range() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 32).await;
    let model = sim.model().clone();
    for _ in 0..400 {
        model.tick(0.25);
    }
    // Ask for a sequence that left the buffer, with both transports.
    for prefer_streaming in [true, false] {
        let mut opts = StreamOptions::new(Duration::from_millis(30));
        opts.from = Some(1);
        opts.prefer_streaming = prefer_streaming;
        let mut stream = client.stream_sample_with(opts);
        let StreamEvent::Resync { reason, snapshot } = next_event(&mut stream).await else { panic!("expected resync") };
        assert_eq!(reason, ResyncReason::OutOfRange);
        let resume = snapshot.header.next_sequence.unwrap();
        assert!(resume > 1);

        model.set_value("Sload", Value::Number(10.0 + resume as f64)).unwrap();
        let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("expected observations") };
        assert!(s.observations().all(|o| o.sequence >= resume), "continues from the snapshot, no gap fill");
        assert!(has(&s, "Sload"));
    }
    sim.stop().await;
}

#[tokio::test]
async fn stream_resyncs_after_agent_restart() {
    for prefer_streaming in [true, false] {
        let (sim, client) = start(DeviceTemplate::Vmc3Axis, 2000).await;
        let model = sim.model().clone();
        let mut opts = StreamOptions::new(Duration::from_millis(30));
        opts.prefer_streaming = prefer_streaming;
        let mut stream = client.stream_sample_with(opts);
        let StreamEvent::Snapshot(first) = next_event(&mut stream).await else { panic!("expected snapshot") };
        let old_instance = first.header.instance_id;
        model.set_value("Sspeed", Value::Number(100.0)).unwrap();
        assert!(matches!(next_event(&mut stream).await, StreamEvent::Observations(_)));

        model.restart_agent();
        let new_instance = model.snapshot().instance_id;
        assert_ne!(new_instance, old_instance);

        let StreamEvent::Resync { reason, snapshot } = next_event(&mut stream).await else { panic!("expected resync") };
        assert_eq!(reason, ResyncReason::InstanceChanged { previous: old_instance, current: new_instance });
        assert_eq!(snapshot.header.instance_id, new_instance);
        assert_eq!(snapshot.header.first_sequence, Some(1));
        // The hand-set value survived the agent restart in the new buffer's current state.
        let speed = snapshot.observations().find(|o| o.data_item_id == "Sspeed").unwrap();
        assert_eq!(speed.value, Value::Number(100.0));

        model.set_value("Sspeed", Value::Number(200.0)).unwrap();
        let StreamEvent::Observations(s) = next_event(&mut stream).await else { panic!("expected observations") };
        assert_eq!(s.header.instance_id, new_instance);
        assert!(s.observations().any(|o| o.value == Value::Number(200.0)));
        sim.stop().await;
    }
}

#[tokio::test]
async fn stream_reports_connection_failure() {
    let (sim, client) = start(DeviceTemplate::Vmc3Axis, 100).await;
    sim.stop().await;
    let mut stream = client.stream_sample(Duration::from_millis(30));
    let item = tokio::time::timeout(Duration::from_secs(10), stream.next()).await.unwrap().unwrap();
    assert!(matches!(item, Err(MtConnectError::Http(_) | MtConnectError::Timeout(_))), "{item:?}");
    assert!(stream.next().await.is_none(), "stream ends after a terminal error");
}

// ── SHDR ───────────────────────────────────────────────────────────────────

async fn read_line(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> String {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), r.read_line(&mut line))
        .await
        .expect("SHDR line within 10s")
        .unwrap();
    line.trim_end().to_string()
}

#[tokio::test]
async fn shdr_adapter_dump_updates_and_heartbeat() {
    let (sim, _client) = start(DeviceTemplate::Vmc3Axis, 1000).await;
    let model = sim.model().clone();
    let sock = TcpStream::connect(sim.shdr_addr().unwrap()).await.unwrap();
    let (rd, mut wr) = sock.into_split();
    let mut rd = BufReader::new(rd);

    // Initial dump: one line per data item.
    let expected = model.device().flatten_data_items().len();
    let mut lines = Vec::new();
    for _ in 0..expected {
        lines.push(read_line(&mut rd).await);
    }
    for l in &lines {
        let f: Vec<&str> = l.split('|').collect();
        assert!(f.len() == 3 || f.len() == 7, "bad field count in '{l}'");
        assert!(f[0].ends_with('Z') && f[0].contains('T'), "timestamp in '{l}'");
    }
    assert!(lines.iter().any(|l| l.ends_with("|avail|AVAILABLE")), "{lines:?}");
    assert!(lines.iter().any(|l| l.ends_with("|estop|ARMED")));
    assert!(lines.iter().any(|l| l.ends_with("|sys_cond|NORMAL||||")));

    // Heartbeat.
    wr.write_all(b"* PING\n").await.unwrap();
    assert_eq!(read_line(&mut rd).await, "* PONG 5000");

    // Live updates.
    model.raise_condition("motion_cond", ConditionLevel::Fault, "E42", "Servo overload").unwrap();
    let l = read_line(&mut rd).await;
    assert!(l.ends_with("|motion_cond|FAULT|E42|1000||Servo overload"), "{l}");
    model.set_value("Xpos", Value::Number(12.5)).unwrap();
    let l = read_line(&mut rd).await;
    assert!(l.ends_with("|Xpos|12.5"), "{l}");
    sim.stop().await;
}

// ── Raw HTTP behaviour ─────────────────────────────────────────────────────

async fn raw_get(addr: std::net::SocketAddr, target: &str) -> String {
    use tokio::io::AsyncReadExt;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut out)).await.unwrap().unwrap();
    out
}

#[tokio::test]
async fn http_error_statuses_and_codes() {
    let (sim, _client) = start(DeviceTemplate::Vmc3Axis, 100).await;
    let a = sim.http_addr();
    let r = raw_get(a, "/sample?from=abc").await;
    assert!(r.starts_with("HTTP/1.1 400"), "{r}");
    assert!(r.contains("errorCode=\"INVALID_PARAMETER_VALUE\""));
    let r = raw_get(a, "/VMC/nope/x").await;
    assert!(r.starts_with("HTTP/1.1 404") && r.contains("errorCode=\"INVALID_URI\""), "{r}");
    let r = raw_get(a, "/bogus/probe").await;
    assert!(r.starts_with("HTTP/1.1 404") && r.contains("errorCode=\"NO_DEVICE\""), "{r}");
    let r = raw_get(a, "/current?path=%2F%2FAxes%5B").await;
    assert!(r.starts_with("HTTP/1.1 400") && r.contains("errorCode=\"INVALID_XPATH\""), "{r}");
    let r = raw_get(a, "/").await;
    assert!(r.starts_with("HTTP/1.1 200") && r.contains("<MTConnectDevices"), "bare / is probe");
    assert!(r.contains("Content-Type: text/xml"));
    sim.stop().await;
}
