//! Smoke test against the public MTConnect demo agent. Needs network access:
//! `cargo test -p mt-mtconnect --test live_demo -- --ignored --nocapture`

use std::collections::HashSet;
use std::time::Duration;

use mt_mtconnect::{AgentClient, StreamEvent, StreamTransport};

const DEMO: &str = "https://demo.mtconnect.org";

#[tokio::test]
#[ignore = "needs network access to https://demo.mtconnect.org"]
async fn demo_agent_probe_current_sample_assets_stream() {
    let client = AgentClient::from_url(DEMO).unwrap();

    let probe = client.probe().await.unwrap();
    let devices: Vec<&str> = probe.devices.iter().map(|d| d.name.as_str()).collect();
    println!("probe: instance {} devices {devices:?}", probe.header.instance_id);
    assert!(probe.devices.iter().any(|d| !d.is_agent));
    let ids: HashSet<&str> = probe
        .devices
        .iter()
        .flat_map(|d| d.flatten_data_items())
        .map(|(_, i)| i.id.as_str())
        .collect();
    assert!(ids.len() > 50);

    let current = client.current(None).await.unwrap();
    println!("current: {} observations, next {:?}", current.len(), current.header.next_sequence);
    assert_eq!(current.header.instance_id, probe.header.instance_id);
    assert!(current.len() > 50);
    let unknown = current.observations().filter(|o| !ids.contains(o.data_item_id.as_str())).count();
    assert_eq!(unknown, 0, "observations for data items missing from probe");

    let axes = client.current(Some("//Axes")).await.unwrap();
    println!("current //Axes: {} observations", axes.len());
    assert!(!axes.is_empty() && axes.len() < current.len());

    let next = current.header.next_sequence.unwrap();
    let sample = client.sample(next.saturating_sub(200), 20).await.unwrap();
    println!("sample: {} observations, next {:?}", sample.len(), sample.header.next_sequence);
    assert!(sample.len() <= 20);
    assert!(sample.observations().all(|o| o.sequence >= next.saturating_sub(200)));

    let range = client.sample(1, 10).await.unwrap_err();
    println!("sample from=1: {range}");
    assert!(range.is_out_of_range());

    let assets = client.assets().await.unwrap();
    println!("assets: {} listed, assetCount {:?}", assets.assets.len(), assets.header.asset_count);
    assert!(!assets.assets.is_empty());
    let first = &assets.assets[0];
    let one = client.asset(&first.asset_id).await.unwrap();
    assert_eq!(one.asset_id, first.asset_id);
    assert!(one.raw_xml.starts_with('<'));

    let mut stream = client.stream_sample(Duration::from_millis(500));
    match tokio::time::timeout(Duration::from_secs(20), stream.next()).await.unwrap() {
        Some(Ok(StreamEvent::Snapshot(s))) => assert!(!s.is_empty()),
        other => panic!("expected snapshot, got {other:?}"),
    }
    let mut observed = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while observed == 0 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(20), stream.next()).await {
            Ok(Some(Ok(StreamEvent::Observations(s)))) => observed += s.len(),
            Ok(Some(Ok(StreamEvent::Resync { .. }))) => {}
            other => panic!("unexpected stream item: {other:?}"),
        }
    }
    println!("stream: transport {:?}, {observed} observations", stream.transport());
    assert!(observed > 0);
    assert_ne!(stream.transport(), StreamTransport::Connecting);
}
