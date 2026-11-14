//! Sparkplug B decoding against payloads encoded by `protoc` from the Eclipse
//! Tahu schema (see `tests/fixtures/sparkplug/regen.sh`), replayed in the
//! order an edge node publishes them.

use std::sync::Arc;

use mt_mqtt::sparkplug::{ncmd_topic, rebirth_payload};
use mt_mqtt::{DecodeKind, PayloadDecoder};
use serde_json::Value;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/sparkplug/{name}.bin",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn decode(d: &mut PayloadDecoder, topic: &str, name: &str) -> Value {
    let out = d
        .decode(topic, &fixture(name))
        .expect("sparkplug topic is decoded");
    assert_eq!(out.kind, DecodeKind::Sparkplug);
    out.result.unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn checks(v: &Value) -> Vec<String> {
    v.get("checks")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| c.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn metric<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("metric {name} in {v}"))
}

#[test]
fn edge_node_session_from_birth_to_death() {
    let mut d = PayloadDecoder::new(Arc::default());

    let nbirth = decode(&mut d, "spBv1.0/Plant1/NBIRTH/Edge1", "nbirth");
    assert_eq!(nbirth["seq"], 0);
    assert_eq!(nbirth["bd_seq"], 3);
    assert_eq!(nbirth["timestamp"], 1_731_571_200_000u64);
    assert_eq!(metric(&nbirth, "bdSeq")["datatype"], "Int64");
    assert_eq!(metric(&nbirth, "Line/Speed")["alias"], 1);
    assert!(checks(&nbirth).is_empty(), "{:?}", checks(&nbirth));

    let dbirth = decode(&mut d, "spBv1.0/Plant1/DBIRTH/Edge1/Filler", "dbirth");
    assert_eq!(metric(&dbirth, "Temperature")["value"], 71.5);
    assert_eq!(metric(&dbirth, "Mode")["value"], "AUTO");
    // Int32 -5 travels as the uint32 4294967291.
    assert_eq!(metric(&dbirth, "Delta")["value"], -5);
    assert!(checks(&dbirth).is_empty(), "{:?}", checks(&dbirth));

    // NDATA carries aliases only; names come from the NBIRTH.
    let ndata = decode(&mut d, "spBv1.0/Plant1/NDATA/Edge1", "ndata");
    let speed = metric(&ndata, "Line/Speed");
    assert_eq!(speed["value"], 1450.5);
    assert_eq!(speed["name_from_alias"], true);
    assert_eq!(metric(&ndata, "Line/Running")["value"], true);
    assert!(checks(&ndata).is_empty(), "{:?}", checks(&ndata));

    // DDATA aliases come from that device's DBIRTH.
    let ddata = decode(&mut d, "spBv1.0/Plant1/DDATA/Edge1/Filler", "ddata");
    assert_eq!(metric(&ddata, "Temperature")["value"], 72.25);
    assert_eq!(metric(&ddata, "Count")["value"], 1201);
    assert!(checks(&ddata).is_empty(), "{:?}", checks(&ddata));

    // seq 4 never arrived, and alias 99 was never declared.
    let gap = decode(&mut d, "spBv1.0/Plant1/DDATA/Edge1/Filler", "ddata_gap");
    let c = checks(&gap);
    assert!(
        c.iter().any(|m| m.starts_with("seq 5, expected 4")),
        "{c:?}"
    );
    assert!(c.iter().any(|m| m.contains("Alias 99")), "{c:?}");
    assert_eq!(metric(&gap, "Count")["value"], 1203);

    // A death certificate from an older session must not take the node offline.
    let stale = decode(&mut d, "spBv1.0/Plant1/NDEATH/Edge1", "ndeath_stale");
    assert_eq!(stale["bd_seq"], 2);
    let c = checks(&stale);
    assert!(
        c.iter()
            .any(|m| m.contains("does not match the NBIRTH bdSeq 3")),
        "{c:?}"
    );

    let ndata_again = decode(&mut d, "spBv1.0/Plant1/NDATA/Edge1", "ndata");
    let c = checks(&ndata_again);
    assert!(!c.iter().any(|m| m.contains("offline")), "{c:?}");

    let death = decode(&mut d, "spBv1.0/Plant1/NDEATH/Edge1", "ndeath");
    assert_eq!(death["bd_seq"], 3);
    assert!(death.get("seq").is_none());
    assert!(checks(&death).is_empty(), "{:?}", checks(&death));

    // Data after the death is flagged.
    let late = decode(&mut d, "spBv1.0/Plant1/NDATA/Edge1", "ndata");
    assert!(checks(&late).iter().any(|m| m.contains("offline")));
}

#[test]
fn data_without_a_birth_asks_for_a_rebirth() {
    let mut d = PayloadDecoder::new(Arc::default());
    let ndata = decode(&mut d, "spBv1.0/Plant1/NDATA/Edge1", "ndata");
    // No names without a BIRTH, but values and aliases still show.
    assert!(ndata["metrics"][0].get("name").is_none());
    assert_eq!(ndata["metrics"][0]["alias"], 1);
    assert_eq!(ndata["metrics"][0]["value"], 1450.5);
    let c = checks(&ndata);
    assert!(
        c.iter()
            .any(|m| m.contains("No NBIRTH seen") && m.contains("Rebirth")),
        "{c:?}"
    );

    // The first DATA sets the baseline, so the next in-order seq is clean of seq errors.
    let ddata = decode(&mut d, "spBv1.0/Plant1/DDATA/Edge1/Filler", "ddata");
    assert!(
        !checks(&ddata).iter().any(|m| m.starts_with("seq")),
        "{:?}",
        checks(&ddata)
    );
}

#[test]
fn rebirth_command_matches_the_reference_encoding() {
    assert_eq!(rebirth_payload(1_731_571_300_000), fixture("ncmd_rebirth"));
    assert_eq!(ncmd_topic("Plant1", "Edge1"), "spBv1.0/Plant1/NCMD/Edge1");

    let mut d = PayloadDecoder::new(Arc::default());
    let v = decode(&mut d, "spBv1.0/Plant1/NCMD/Edge1", "ncmd_rebirth");
    let m = metric(&v, "Node Control/Rebirth");
    assert_eq!(m["value"], true);
    assert_eq!(m["datatype"], "Boolean");
    assert!(checks(&v).is_empty());
}
