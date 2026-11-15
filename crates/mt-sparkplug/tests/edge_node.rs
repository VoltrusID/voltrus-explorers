//! EdgeNode state machine: births, aliases, report by exception, deaths,
//! Rebirth and DCMD, checked by decoding every payload it produces.

use mt_sparkplug::pb::payload::Metric;
use mt_sparkplug::pb::payload::metric::Value as V;
use mt_sparkplug::{EdgeNode, EdgeNodeConfig, Outgoing, Payload, SpValue};
use prost::Message;

fn node(dir: &tempfile::TempDir) -> EdgeNode {
    EdgeNode::new(EdgeNodeConfig {
        group_id: "Plant1".into(),
        edge_node_id: "Gateway1".into(),
        bd_seq_path: dir.path().join("bd_seq"),
        node_metrics: vec![(
            "Properties/Software".into(),
            SpValue::String("Voltrus Gateway".into()),
        )],
        max_interval_ms: 60_000,
    })
}

fn decode(m: &Outgoing) -> Payload {
    Payload::decode(m.payload.as_slice()).expect("payload decodes")
}

fn metric<'a>(p: &'a Payload, name: &str) -> &'a Metric {
    p.metrics
        .iter()
        .find(|m| m.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} in {p:?}"))
}

fn tags(speed: f64, count: u16, run: bool) -> Vec<(String, Option<SpValue>)> {
    vec![
        ("Speed".into(), Some(SpValue::Double(speed))),
        ("Count".into(), Some(SpValue::UInt16(count))),
        ("Running".into(), Some(SpValue::Boolean(run))),
    ]
}

#[test]
fn session_births_data_and_death() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = node(&dir);
    // Values sampled before the session are kept for the birth.
    assert!(
        n.update_device("Filler", 0.5, tags(10.0, 1, true), 1)
            .is_empty()
    );

    let (will, births) = n.open_session(1000).unwrap();
    assert_eq!(will.topic, "spBv1.0/Plant1/NDEATH/Gateway1");
    let w = decode(&will);
    assert_eq!(metric(&w, "bdSeq").value, Some(V::LongValue(1)));
    assert_eq!(w.seq, None);

    assert_eq!(births.len(), 2);
    assert_eq!(births[0].topic, "spBv1.0/Plant1/NBIRTH/Gateway1");
    let nb = decode(&births[0]);
    assert_eq!(nb.seq, Some(0));
    assert_eq!(metric(&nb, "bdSeq").value, Some(V::LongValue(1)));
    assert_eq!(
        metric(&nb, "Node Control/Rebirth").value,
        Some(V::BooleanValue(false))
    );
    assert_eq!(
        metric(&nb, "Properties/Software").value,
        Some(V::StringValue("Voltrus Gateway".into()))
    );

    assert_eq!(births[1].topic, "spBv1.0/Plant1/DBIRTH/Gateway1/Filler");
    let db = decode(&births[1]);
    assert_eq!(db.seq, Some(1));
    let speed_alias = metric(&db, "Speed").alias.unwrap();
    assert_eq!(metric(&db, "Count").datatype, Some(6)); // UInt16
    assert_eq!(metric(&db, "Count").value, Some(V::IntValue(1)));

    // Unchanged values: nothing. Change inside the deadband: nothing.
    assert!(
        n.update_device("Filler", 0.5, tags(10.0, 1, true), 2000)
            .is_empty()
    );
    assert!(
        n.update_device("Filler", 0.5, tags(10.3, 1, true), 3000)
            .is_empty()
    );

    // Change outside the deadband: DDATA with the alias only.
    let out = n.update_device("Filler", 0.5, tags(11.0, 2, true), 4000);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].topic, "spBv1.0/Plant1/DDATA/Gateway1/Filler");
    let dd = decode(&out[0]);
    assert_eq!(dd.seq, Some(2));
    assert_eq!(dd.metrics.len(), 2);
    assert!(dd.metrics.iter().all(|m| m.name.is_none()));
    let speed = dd
        .metrics
        .iter()
        .find(|m| m.alias == Some(speed_alias))
        .unwrap();
    assert_eq!(speed.value, Some(V::DoubleValue(11.0)));

    // A suppressed change is sent once the max interval has passed.
    assert!(
        n.update_device("Filler", 0.5, tags(11.2, 2, true), 5000)
            .is_empty()
    );
    let out = n.update_device("Filler", 0.5, tags(11.2, 2, true), 70_000);
    assert_eq!(decode(&out[0]).metrics.len(), 1);

    // Graceful death carries the same bdSeq as the Will.
    let d = n.death(80_000);
    assert_eq!(d.payload, {
        let mut p = decode(&will);
        p.timestamp = Some(80_000);
        p.metrics[0].timestamp = Some(80_000);
        p.encode_to_vec()
    });

    // A new session bumps bdSeq and restarts seq at 0.
    let (will2, births2) = n.open_session(90_000).unwrap();
    assert_eq!(
        metric(&decode(&will2), "bdSeq").value,
        Some(V::LongValue(2))
    );
    assert_eq!(decode(&births2[0]).seq, Some(0));
    // Aliases stay stable across sessions.
    assert_eq!(
        metric(&decode(&births2[1]), "Speed").alias,
        Some(speed_alias)
    );
}

#[test]
fn device_death_rebirth_and_shape_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = node(&dir);
    n.open_session(0).unwrap();
    let out = n.update_device(
        "Pump",
        0.0,
        vec![("Flow".into(), Some(SpValue::Float(1.5)))],
        1,
    );
    assert!(out[0].topic.ends_with("/DBIRTH/Gateway1/Pump"));

    // Every tag failed: DDEATH with a seq and no metrics.
    let out = n.update_device("Pump", 0.0, vec![("Flow".into(), None)], 2);
    assert!(out[0].topic.ends_with("/DDEATH/Gateway1/Pump"));
    let dd = decode(&out[0]);
    assert_eq!(dd.seq, Some(2));
    assert!(dd.metrics.is_empty());
    // Still failing: no second death.
    assert!(
        n.update_device("Pump", 0.0, vec![("Flow".into(), None)], 3)
            .is_empty()
    );

    // Values return: DBIRTH again.
    let out = n.update_device(
        "Pump",
        0.0,
        vec![("Flow".into(), Some(SpValue::Float(2.0)))],
        4,
    );
    assert!(out[0].topic.ends_with("/DBIRTH/Gateway1/Pump"));

    // A new tag changes the metric set: DBIRTH, not DDATA.
    let out = n.update_device(
        "Pump",
        0.0,
        vec![
            ("Flow".into(), Some(SpValue::Float(2.0))),
            ("Pressure".into(), Some(SpValue::Int32(-3))),
        ],
        5,
    );
    assert!(out[0].topic.ends_with("/DBIRTH/Gateway1/Pump"));
    let db = decode(&out[0]);
    // Int32 -3 travels as the uint32 two's complement.
    assert_eq!(
        metric(&db, "Pressure").value,
        Some(V::IntValue((-3i32) as u32))
    );

    // One failing tag out of two: the device stays alive, the failed tag is skipped.
    let out = n.update_device(
        "Pump",
        0.0,
        vec![
            ("Flow".into(), Some(SpValue::Float(3.0))),
            ("Pressure".into(), None),
        ],
        6,
    );
    assert!(out[0].topic.ends_with("/DDATA/Gateway1/Pump"));
    assert_eq!(decode(&out[0]).metrics.len(), 1);

    let out = n.remove_device("Pump", 7);
    assert!(out.unwrap().topic.ends_with("/DDEATH/Gateway1/Pump"));
    assert!(n.devices().is_empty());
}

fn cmd(metrics: Vec<Metric>) -> Vec<u8> {
    Payload {
        timestamp: Some(1),
        metrics,
        seq: None,
        uuid: None,
        body: None,
    }
    .encode_to_vec()
}

#[test]
fn ncmd_rebirth_and_dcmd_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = node(&dir);
    n.open_session(0).unwrap();
    n.update_device("Filler", 0.0, tags(1.0, 1, false), 1);
    n.update_device("Filler", 0.0, tags(2.0, 1, false), 2);
    assert_eq!(n.seq(), 3);

    // Rebirth: NBIRTH seq 0 with the same bdSeq, then every DBIRTH.
    let bd = n.bd_seq();
    let rebirth = cmd(vec![Metric {
        name: Some("Node Control/Rebirth".into()),
        datatype: Some(11),
        value: Some(V::BooleanValue(true)),
        ..Default::default()
    }]);
    let out = n.handle_command("spBv1.0/Plant1/NCMD/Gateway1", &rebirth, 10);
    assert_eq!(out.publish.len(), 2);
    let nb = decode(&out.publish[0]);
    assert_eq!(nb.seq, Some(0));
    assert_eq!(
        metric(&nb, "bdSeq").value,
        Some(V::LongValue(u64::from(bd)))
    );
    assert_eq!(decode(&out.publish[1]).seq, Some(1));
    assert_eq!(
        metric(&decode(&out.publish[1]), "Speed").value,
        Some(V::DoubleValue(2.0))
    );

    // DCMD by name and by alias.
    let alias = {
        let db = decode(&out.publish[1]);
        metric(&db, "Count").alias.unwrap()
    };
    let dcmd = cmd(vec![
        Metric {
            name: Some("Speed".into()),
            datatype: Some(10),
            value: Some(V::DoubleValue(42.5)),
            ..Default::default()
        },
        Metric {
            alias: Some(alias),
            datatype: Some(6),
            value: Some(V::IntValue(7)),
            ..Default::default()
        },
        Metric {
            name: Some("Nope".into()),
            datatype: Some(11),
            value: Some(V::BooleanValue(true)),
            ..Default::default()
        },
        Metric {
            name: Some("Running".into()),
            datatype: Some(11),
            value: Some(V::StringValue("x".into())),
            ..Default::default()
        },
    ]);
    let out = n.handle_command("spBv1.0/Plant1/DCMD/Gateway1/Filler", &dcmd, 11);
    assert!(out.publish.is_empty());
    assert_eq!(out.writes.len(), 2);
    assert_eq!(out.writes[0].metric, "Speed");
    assert_eq!(out.writes[0].value, SpValue::Double(42.5));
    assert_eq!(out.writes[1].metric, "Count");
    assert_eq!(out.writes[1].value, SpValue::UInt16(7));
    assert_eq!(out.rejected.len(), 2, "{:?}", out.rejected);

    // Other edge nodes' commands and garbage are not ours.
    let out = n.handle_command("spBv1.0/Plant1/DCMD/Gateway1/Unknown", &dcmd, 12);
    assert!(out.writes.is_empty());
    let out = n.handle_command("spBv1.0/Plant1/NCMD/Gateway1", b"\xff\xff", 12);
    assert_eq!(out.rejected.len(), 1);
}

#[test]
fn seq_wraps_after_255() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = node(&dir);
    n.open_session(0).unwrap();
    let mut last = 0;
    for i in 0..300u32 {
        let out = n.update_device(
            "D",
            0.0,
            vec![("V".into(), Some(SpValue::UInt32(i)))],
            u64::from(i) + 1,
        );
        last = decode(&out[0]).seq.unwrap();
        assert!(last <= 255);
    }
    // NBIRTH 0, DBIRTH 1, then 299 DDATA: (1 + 299) % 256.
    assert_eq!(last, 300 % 256);
}

#[test]
fn spvalue_round_trips_through_the_wire() {
    for v in [
        SpValue::Boolean(true),
        SpValue::Int8(-8),
        SpValue::Int16(-300),
        SpValue::Int32(-70_000),
        SpValue::Int64(-9_000_000_000),
        SpValue::UInt8(200),
        SpValue::UInt16(65_000),
        SpValue::UInt32(4_000_000_000),
        SpValue::UInt64(u64::MAX),
        SpValue::Float(1.25),
        SpValue::Double(-2.5),
        SpValue::String("AUTO".into()),
        SpValue::Bytes(vec![1, 2, 3]),
    ] {
        let back = SpValue::from_wire(v.data_type() as u32, &v.to_wire());
        assert_eq!(back, Some(v));
    }
}

#[test]
fn nan_is_not_republished_on_every_sample() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = node(&dir);
    n.open_session(1_000).unwrap();
    let nan = |t: u64, n: &mut EdgeNode| {
        n.update_device(
            "Filler",
            0.0,
            vec![("Speed".into(), Some(SpValue::Double(f64::NAN)))],
            t,
        )
    };
    assert_eq!(nan(1_100, &mut n).len(), 1, "DBIRTH");
    assert!(nan(1_200, &mut n).is_empty(), "NaN after NaN is no change");
    let out = n.update_device(
        "Filler",
        0.0,
        vec![("Speed".into(), Some(SpValue::Double(5.0)))],
        1_300,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].topic.ends_with("/DDATA/Gateway1/Filler"));
}
