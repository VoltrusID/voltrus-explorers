//! Parsing of real responses captured from https://demo.mtconnect.org (agent 2.8.0.4, schema 2.8).
//! The files under `tests/fixtures/demo_*.xml` are unmodified responses, except `demo_assets.xml`
//! which keeps only the first two of the 67 assets.

use std::collections::HashSet;

use mt_mtconnect::{
    AgentErrorCode, Category, ConditionLevel, Document, MtConnectError, Value, parse_assets, parse_devices,
    parse_document, parse_streams,
};

const PROBE: &str = include_str!("fixtures/demo_probe.xml");
const CURRENT: &str = include_str!("fixtures/demo_current.xml");
const SAMPLE: &str = include_str!("fixtures/demo_sample.xml");
const ASSETS: &str = include_str!("fixtures/demo_assets.xml");
const ERR_RANGE: &str = include_str!("fixtures/demo_error_out_of_range.xml");
const ERR_DEVICE: &str = include_str!("fixtures/demo_error_no_device.xml");

#[test]
fn probe_device_tree() {
    let probe = parse_devices(PROBE).unwrap();
    assert_eq!(probe.header.instance_id, 1791887049);
    assert_eq!(probe.header.buffer_size, Some(131072));
    assert_eq!(probe.header.version.as_deref(), Some("2.8.0.4"));
    assert_eq!(probe.header.asset_buffer_size, Some(1024));

    let names: Vec<_> = probe.devices.iter().map(|d| (d.name.as_str(), d.is_agent)).collect();
    assert_eq!(names, [("Agent", true), ("OKUMA", false), ("Mazak", false)]);

    let okuma = probe.device("OKUMA").unwrap();
    assert_eq!(okuma.uuid.as_deref(), Some("OKUMA.123456"));
    assert!(okuma.components.iter().any(|c| c.element == "Axes"), "Axes component present");

    // A position sample with units and subType, nested inside Axes/Linear.
    let items = okuma.flatten_data_items();
    let yact = items.iter().find(|(_, d)| d.id == "LYI1actm").expect("Y actual position");
    assert_eq!(yact.1.category, Category::Sample);
    assert_eq!(yact.1.data_type, "POSITION");
    assert_eq!(yact.1.sub_type.as_deref(), Some("ACTUAL"));
    assert_eq!(yact.1.units.as_deref(), Some("MILLIMETER"));
    assert!(yact.0.contains("Y"), "component path {}", yact.0);

    let (mut samples, mut events, mut conditions) = (0, 0, 0);
    for d in &probe.devices {
        for (_, i) in d.flatten_data_items() {
            match i.category {
                Category::Sample => samples += 1,
                Category::Event => events += 1,
                Category::Condition => conditions += 1,
            }
        }
    }
    assert!(samples > 20 && events > 20 && conditions > 5, "{samples}/{events}/{conditions}");

    let ds = probe.data_item("Lp1CommonVariable").unwrap();
    assert_eq!(ds.representation.as_deref(), Some("DATA_SET"));
}

#[test]
fn current_observations_match_probe() {
    let probe = parse_devices(PROBE).unwrap();
    let ids: HashSet<&str> = probe
        .devices
        .iter()
        .flat_map(|d| d.flatten_data_items())
        .map(|(_, i)| i.id.as_str())
        .collect();

    let current = parse_streams(CURRENT).unwrap();
    assert_eq!(current.header.instance_id, 1791887049);
    assert_eq!(current.header.first_sequence, Some(62050915));
    assert_eq!(current.header.last_sequence, Some(62181986));
    assert_eq!(current.header.next_sequence, Some(62181987));
    assert!(current.len() > 100);

    let mut unavailable = 0;
    let mut seen_levels = HashSet::new();
    for o in current.observations() {
        assert!(ids.contains(o.data_item_id.as_str()), "{} not in probe", o.data_item_id);
        assert!(o.sequence > 0 && !o.timestamp.is_empty());
        assert!(o.sequence <= 62181986);
        match o.category {
            Category::Condition => {
                let c = o.condition.as_ref().expect("condition details");
                seen_levels.insert(c.level);
            }
            _ => assert!(o.condition.is_none()),
        }
        if o.is_unavailable() {
            unavailable += 1;
        }
    }
    assert!(unavailable > 0);
    assert!(seen_levels.contains(&ConditionLevel::Normal));
    assert!(seen_levels.contains(&ConditionLevel::Unavailable));

    // A numeric sample, a text event, and a data set.
    let pos = current.observations().find(|o| o.data_item_id == "LYI1actm").unwrap();
    assert!(matches!(pos.value, Value::Number(_) | Value::Unavailable), "{:?}", pos.value);
    assert_eq!(pos.sub_type.as_deref(), Some("ACTUAL"));
    let count = current.observations().find(|o| o.data_item_id == "OKUMA.123456_asset_count").unwrap();
    match &count.value {
        Value::DataSet(e) => assert_eq!(e[0].key, "CuttingTool"),
        v => panic!("{v:?}"),
    }
}

#[test]
fn sample_document() {
    let s = parse_streams(SAMPLE).unwrap();
    assert_eq!(s.header.next_sequence, Some(62051053));
    assert_eq!(s.header.first_sequence, Some(62051033));
    assert!(!s.is_empty());
    let seqs: Vec<u64> = s.observations().map(|o| o.sequence).collect();
    assert!(seqs.iter().all(|q| (62051033..62051053).contains(q)), "{seqs:?}");
}

#[test]
fn error_documents_are_typed() {
    let err = parse_streams(ERR_RANGE).unwrap_err();
    assert!(err.is_out_of_range());
    match err {
        MtConnectError::Agent(e) => {
            assert_eq!(e.errors.len(), 1);
            assert_eq!(e.errors[0].code, AgentErrorCode::OutOfRange);
            assert_eq!(e.errors[0].message, "'from' must be greater than 62071138");
            assert_eq!(e.header.instance_id, 1791887049);
        }
        other => panic!("{other:?}"),
    }
    match parse_document(ERR_DEVICE).unwrap() {
        Document::Error(e) => {
            assert_eq!(e.errors[0].code, AgentErrorCode::NoDevice);
            assert!(e.errors[0].message.contains("nonexistent"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn assets_with_raw_xml() {
    let a = parse_assets(ASSETS).unwrap();
    assert_eq!(a.header.asset_count, Some(67));
    assert_eq!(a.assets.len(), 2);
    let first = &a.assets[0];
    assert_eq!(first.asset_type, "CuttingTool");
    assert_eq!(first.asset_id, "M8015P520ZN1.44");
    assert_eq!(first.device_uuid.as_deref(), Some("Mazak"));
    assert!(first.raw_xml.starts_with("<CuttingTool assetId=\"M8015P520ZN1.44\""));
    assert!(first.raw_xml.ends_with("</CuttingTool>"));
    assert!(ASSETS.contains(&first.raw_xml), "raw_xml is an exact slice of the response");
    assert!(a.assets[1].raw_xml.contains("<ProgramToolNumber>2201</ProgramToolNumber>"));
}
