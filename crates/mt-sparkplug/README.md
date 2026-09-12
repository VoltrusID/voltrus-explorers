# mt-sparkplug

Minimal Sparkplug B edge publisher (Atur issue GW-3). Publish-only: one edge
node reports its metrics to a Sparkplug host over MQTT with the standard
birth/death session machinery.

## Honest scope

Implemented:

- `spBv1.0/{group}/NBIRTH|NDEATH|NDATA/{edge_node}` topics, protobuf B
  payloads (schema vendored at `proto/sparkplug_b.proto`, from Eclipse Tahu,
  EPL-2.0).
- Session state machine per the Sparkplug normative statements: connect with
  an NDEATH Will carrying the pending `bdSeq` (QoS 0, retain=false), publish
  NBIRTH with **all** metrics plus `bdSeq` at `seq = 0`, then NDATA with
  report-by-exception. `seq` and `bdSeq` both stay in 0..=255 and wrap at 256.
- `bdSeq` persisted to a file, incremented on every (re)connect so hosts can
  tell sessions apart; NDEATH payload never carries a `seq`.
- RBE with per-prefix deadbands: a metric whose numeric change is smaller than
  its deadband is suppressed until `max_interval_ms` has passed since its last
  publish. Bool/String metrics publish on any change (no deadband concept).
- Graceful shutdown publishes NDEATH directly (the broker suppresses the Will
  on a clean disconnect, so hosts see one death, not two).

Explicitly NOT implemented (by design for GW-3):

- No NCMD/DCMD subscription, so no REBIRTH handling.
- No metric aliases (every NDATA repeats metric names).
- No Templates, DataSets, property sets, or metadata.
- No DBIRTH/DDEATH/DDATA (no device sub-tree).
- No primary-host STATE handshake (`STATE/{host}` ONLINE/OFFLINE).
- Not TCK-conformant; see below.

## Layout

- `src/pb.rs` + `build.rs` — prost codegen from the vendored schema.
- `src/value.rs` — `MetricValue` (Bool/Int/Real/String) and the
  `From<mt_plc_types::PlcValue>` mapping. We depend on `mt-plc-types` (the
  crate that actually defines `PlcValue`; `mt-plc-runtime` only re-exports it)
  because `mt-plc-runtime` drags the modbus/opcua/s7/enip stacks with it.
  ARRAY/STRUCT PLC values fall back to their display form as strings.
- `src/topic.rs` — topic builder.
- `src/bdseq.rs` — bdSeq file persistence.
- `src/transport.rs` — `MqttTransport` trait (mockable) + `RumqttcTransport`.
- `src/publisher.rs` — `SparkplugConfig`, `MetricSource`, and the
  `SparkplugPublisher` state machine.

Transport note: we intentionally did not adapt `mt-mqtt`'s `MqttPubClient` —
it is subscription/RTT-oriented for the explorer apps and would drag the
embedded `rumqttd` broker into this crate. A publish-only edge node needs very
little rumqttc directly.

Two QoS details where intuition says the opposite of the spec: all non-STATE
Sparkplug messages MUST use **QoS 0** (not 1), and the clean-session flag MUST
be **true**. Both are pinned in `RumqttcTransport`.

## Integration sketch (future Linux runtime)

```rust
use mt_sparkplug::{MetricSource, RumqttcTransport, SparkplugConfig, SparkplugPublisher};

let config = SparkplugConfig {
    broker_url: "tcp://broker.local:1883".into(),
    group_id: "plant".into(),
    edge_node_id: "gateway-1".into(),
    bd_seq_path: "/var/lib/voltrus/bd_seq".into(),
    metrics: vec![MetricSource::plc("line/count", || runtime.read("count"))],
    deadband_by_prefix: vec![("tank/level".into(), 0.5)],
    max_interval_ms: 60_000,
};
let mut publisher = SparkplugPublisher::new(config, RumqttcTransport::new(/* url, id */)?);
publisher.connect(now_ms()).await?;
loop {
    publisher.publish_changes(now_ms()).await?; // per scan tick
    // on connection loss: call publisher.connect(now_ms()).await? again
}
publisher.death(now_ms()).await?; // graceful shutdown
```

Known runtime caveat: once `RumqttcTransport::connect` succeeds, a background
task drains the rumqttc event loop and stops at the first network error (logged
via `tracing`) so a dead session surfaces as publish failures instead of silent
loss. Detecting and recovering from that loss — calling `connect` again — is
the integration's job; `connect` is safe to call repeatedly and re-births with
a fresh `bdSeq` each time.

## What TCK conformance would add

Running the Eclipse Sparkplug TCK (edge-node suite) against this publisher
would require, at minimum:

- NCMD handling including REBIRTH (republish NBIRTH with `seq = 0`).
- Metric aliases on NBIRTH and alias-only NDATA metrics.
- DBIRTH/DDEATH/DDATA for device sub-trees, including device death Wills.
- Property sets / metadata on metrics, Templates for complex data.
- The primary-host STATE handshake if we ever act as a host application.
- TLS/mTLS broker options and persistent-session edge cases the TCK pokes at
  (our sessions are deliberately clean, per the spec).
