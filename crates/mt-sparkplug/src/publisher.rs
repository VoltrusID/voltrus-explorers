//! The publish state machine: NBIRTH on session start, report-by-exception
//! NDATA on change, NDEATH via the MQTT Will and on graceful shutdown.

use mt_plc_types::PlcValue;
use prost::Message;

use crate::bdseq;
use crate::pb::payload::metric;
use crate::pb::{DataType, Payload};
use crate::topic::{sparkplug_topic, SparkplugVerb};
use crate::transport::Will;
use crate::value::MetricValue;
use crate::{Metric, MqttTransport, PAYLOAD_UUID, SparkplugError};

/// One metric the edge node publishes. `get` is sampled on every scan.
pub struct MetricSource {
    pub name: String,
    /// Sample the metric. `None` = unavailable this scan (e.g. the backing
    /// variable does not currently exist): the scan skips the metric without
    /// touching its published state. A metric whose first-ever sample is
    /// `None` misses that session's NBIRTH and stays unpublished until the
    /// next birth — hosts reject unbirthed NDATA metrics.
    pub get: Box<dyn Fn() -> Option<MetricValue> + Send + Sync>,
}

impl MetricSource {
    /// Source reading a `PlcValue` from the PLC runtime (mapped onto
    /// [`MetricValue`]).
    pub fn plc<F>(name: impl Into<String>, get: F) -> Self
    where
        F: Fn() -> PlcValue + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            get: Box::new(move || Some(MetricValue::from(get()))),
        }
    }
}

/// Edge node identity and publish policy.
pub struct SparkplugConfig {
    /// Broker address, `tcp://host:port` (used by `RumqttcTransport`).
    pub broker_url: String,
    pub group_id: String,
    pub edge_node_id: String,
    /// File persisting the bdSeq counter across restarts: each new session
    /// increments bdSeq so hosts can tell sessions apart.
    pub bd_seq_path: std::path::PathBuf,
    pub metrics: Vec<MetricSource>,
    /// Longest matching name-prefix → deadband. Metrics outside the table use
    /// a deadband of 0 (publish every change).
    pub deadband_by_prefix: Vec<(String, f64)>,
    /// A metric suppressed by its deadband is still published once this much
    /// time has passed since its last publish.
    pub max_interval_ms: u64,
}

/// Sparkplug B edge publisher over any [`MqttTransport`].
///
/// Lifecycle: [`connect`](Self::connect) opens a session (bumps bdSeq,
/// registers the NDEATH Will, publishes NBIRTH), [`publish_changes`](Self::publish_changes)
/// is the per-scan RBE step, [`death`](Self::death) publishes NDEATH for
/// graceful shutdown. After a lost session, call `connect` again.
pub struct SparkplugPublisher<T: MqttTransport> {
    config: SparkplugConfig,
    transport: T,
    /// Payload seq of the next message; the spec keeps seq in 0..=255 with
    /// wraparound (NBIRTH starts at 0).
    seq: u8,
    /// bdSeq of the current session (0 until `connect`).
    bd_seq: u8,
    /// Last published value per metric (`config.metrics` order).
    last: Vec<Option<MetricValue>>,
    /// Millis timestamp of the last publish per metric.
    last_publish_ms: Vec<Option<u64>>,
    /// Whether the metric made it into this session's NBIRTH.
    birthed: Vec<bool>,
}

impl<T: MqttTransport> SparkplugPublisher<T> {
    pub fn new(config: SparkplugConfig, transport: T) -> Self {
        let metric_count = config.metrics.len();
        Self {
            config,
            transport,
            seq: 0,
            bd_seq: 0,
            last: vec![None; metric_count],
            last_publish_ms: vec![None; metric_count],
            birthed: vec![false; metric_count],
        }
    }

    /// Open a session: bump + persist bdSeq, register the NDEATH Will,
    /// connect, then publish NBIRTH carrying every available metric and `seq = 0`.
    pub async fn connect(&mut self, now_ms: u64) -> Result<(), SparkplugError> {
        let bd_seq = bdseq::load(&self.config.bd_seq_path)?.wrapping_add(1);
        bdseq::store(&self.config.bd_seq_path, bd_seq)?;
        self.bd_seq = bd_seq;

        let death = Payload {
            timestamp: Some(now_ms),
            // NDEATH carries no seq: it is authored outside sequence
            // bookkeeping (the Will fires when the session is gone).
            seq: None,
            uuid: Some(PAYLOAD_UUID.to_string()),
            metrics: vec![bd_seq_metric(bd_seq, now_ms)],
            body: None,
        };
        self.transport
            .connect(Will {
                topic: self.topic(SparkplugVerb::NDeath),
                payload: death.encode_to_vec(),
            })
            .await?;

        // NBIRTH: every metric that has a value right now, plus bdSeq. A
        // metric with no sample misses this birth and stays unpublished
        // until the next session.
        let mut metrics = Vec::with_capacity(self.config.metrics.len() + 1);
        metrics.push(bd_seq_metric(bd_seq, now_ms));
        let mut last = vec![None; self.config.metrics.len()];
        let mut last_publish_ms = vec![None; self.config.metrics.len()];
        for (i, source) in self.config.metrics.iter().enumerate() {
            let Some(value) = (source.get)() else {
                continue;
            };
            metrics.push(value_metric(&source.name, &value, now_ms));
            last[i] = Some(value);
            last_publish_ms[i] = Some(now_ms);
        }
        let birth = Payload {
            timestamp: Some(now_ms),
            seq: Some(0),
            uuid: Some(PAYLOAD_UUID.to_string()),
            metrics,
            body: None,
        };
        let topic = self.topic(SparkplugVerb::NBirth);
        self.transport.publish(&topic, &birth.encode_to_vec()).await?;

        self.seq = 1;
        self.birthed = last_publish_ms.iter().map(|slot| slot.is_some()).collect();
        self.last = last;
        self.last_publish_ms = last_publish_ms;
        Ok(())
    }

    /// Sample every metric and publish an NDATA with the changed ones
    /// (report-by-exception). Returns whether anything was published.
    pub async fn publish_changes(&mut self, now_ms: u64) -> Result<bool, SparkplugError> {
        let mut metrics = Vec::new();
        for (i, source) in self.config.metrics.iter().enumerate() {
            if !self.birthed[i] {
                // Missed this session's NBIRTH: hosts reject unbirthed NDATA
                // metrics, so it stays quiet until the next session.
                continue;
            }
            let Some(value) = (source.get)() else {
                // Unavailable this scan: keep the last published state.
                continue;
            };
            let Some(last) = self.last[i].as_ref() else {
                continue;
            };
            let deadband = deadband_for(&self.config.deadband_by_prefix, &source.name);
            let elapsed_ms = now_ms.saturating_sub(self.last_publish_ms[i].unwrap_or(now_ms));
            if should_publish(&value, last, deadband, elapsed_ms, self.config.max_interval_ms) {
                metrics.push(value_metric(&source.name, &value, now_ms));
                self.last[i] = Some(value);
                self.last_publish_ms[i] = Some(now_ms);
            }
        }
        if metrics.is_empty() {
            return Ok(false);
        }
        let seq = self.seq;
        let data = Payload {
            timestamp: Some(now_ms),
            seq: Some(u64::from(seq)),
            uuid: Some(PAYLOAD_UUID.to_string()),
            metrics,
            body: None,
        };
        let topic = self.topic(SparkplugVerb::NData);
        self.transport.publish(&topic, &data.encode_to_vec()).await?;
        self.seq = seq.wrapping_add(1);
        Ok(true)
    }

    /// Graceful shutdown: publish NDEATH directly. The Will payload carries
    /// the same bdSeq, and the broker suppresses the Will on a clean
    /// disconnect, so hosts see exactly one death.
    pub async fn death(&mut self, now_ms: u64) -> Result<(), SparkplugError> {
        let death = Payload {
            timestamp: Some(now_ms),
            seq: None,
            uuid: Some(PAYLOAD_UUID.to_string()),
            metrics: vec![bd_seq_metric(self.bd_seq, now_ms)],
            body: None,
        };
        let topic = self.topic(SparkplugVerb::NDeath);
        self.transport.publish(&topic, &death.encode_to_vec()).await
    }

    fn topic(&self, verb: SparkplugVerb) -> String {
        sparkplug_topic(&self.config.group_id, &self.config.edge_node_id, verb)
    }
}

/// Report-by-exception decision: publish on any change, unless the numeric
/// change is still inside the deadband and the max interval has not elapsed.
fn should_publish(
    current: &MetricValue,
    last: &MetricValue,
    deadband: f64,
    elapsed_ms: u64,
    max_interval_ms: u64,
) -> bool {
    if current == last {
        return false;
    }
    let within_deadband = match (current, last) {
        (MetricValue::Int(current), MetricValue::Int(last)) => {
            ((*current - *last).abs() as f64) < deadband
        }
        (MetricValue::Real(current), MetricValue::Real(last)) => (current - last).abs() < deadband,
        // Bool/String have no magnitude: any change publishes.
        _ => false,
    };
    !within_deadband || elapsed_ms > max_interval_ms
}

/// Longest matching prefix wins; no match → 0 (publish every change).
fn deadband_for(deadband_by_prefix: &[(String, f64)], name: &str) -> f64 {
    deadband_by_prefix
        .iter()
        .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, deadband)| *deadband)
        .unwrap_or(0.0)
}

/// The `bdSeq` metric (Int64) carried by NBIRTH and both NDEATH forms.
fn bd_seq_metric(bd_seq: u8, timestamp_ms: u64) -> Metric {
    Metric {
        name: Some("bdSeq".to_string()),
        timestamp: Some(timestamp_ms),
        datatype: Some(DataType::Int64 as u32),
        value: Some(metric::Value::LongValue(u64::from(bd_seq))),
        ..Default::default()
    }
}

/// Encode a [`MetricValue`] as a Sparkplug B metric.
///
/// `Int` rides in `long_value` (proto uint64): the wire encoding is two's
/// complement, so a host reading the metric as Int64 recovers the signed
/// value. This matches what Sparkplug tooling expects for Int64 metrics.
fn value_metric(name: &str, value: &MetricValue, timestamp_ms: u64) -> Metric {
    let (datatype, value) = match value {
        MetricValue::Bool(v) => (DataType::Boolean as u32, metric::Value::BooleanValue(*v)),
        MetricValue::Int(v) => (DataType::Int64 as u32, metric::Value::LongValue(*v as u64)),
        MetricValue::Real(v) => (DataType::Double as u32, metric::Value::DoubleValue(*v)),
        MetricValue::String(v) => (DataType::String as u32, metric::Value::StringValue(v.clone())),
    };
    Metric {
        name: Some(name.to_string()),
        timestamp: Some(timestamp_ms),
        datatype: Some(datatype),
        value: Some(value),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MqttTransport;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Records every Will and publish so assertions decode payloads.
    #[derive(Default)]
    struct MockTransport {
        wills: Vec<(String, Vec<u8>)>,
        publishes: Vec<(String, Vec<u8>)>,
    }

    impl MockTransport {
        /// Decode the payloads published to topics ending in `verb`.
        fn payloads(&self, verb: &str) -> Vec<Payload> {
            self.publishes
                .iter()
                .filter(|(topic, _)| topic.ends_with(verb))
                .map(|(_, bytes)| Payload::decode(bytes.as_slice()).expect("decodable payload"))
                .collect()
        }
    }

    impl MqttTransport for MockTransport {
        async fn connect(&mut self, will: Will) -> Result<(), SparkplugError> {
            self.wills.push((will.topic, will.payload));
            Ok(())
        }

        async fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), SparkplugError> {
            self.publishes.push((topic.to_string(), payload.to_vec()));
            Ok(())
        }
    }

    fn int_source(name: &str, value: &Arc<Mutex<i64>>) -> MetricSource {
        let value = value.clone();
        MetricSource {
            name: name.to_string(),
            get: Box::new(move || Some(MetricValue::Int(*value.lock()))),
        }
    }

    fn real_source(name: &str, value: &Arc<Mutex<f64>>) -> MetricSource {
        let value = value.clone();
        MetricSource {
            name: name.to_string(),
            get: Box::new(move || Some(MetricValue::Real(*value.lock()))),
        }
    }

    fn bool_source(name: &str, value: &Arc<Mutex<bool>>) -> MetricSource {
        let value = value.clone();
        MetricSource {
            name: name.to_string(),
            get: Box::new(move || Some(MetricValue::Bool(*value.lock()))),
        }
    }

    fn config(
        dir: &TempDir,
        metrics: Vec<MetricSource>,
        deadbands: Vec<(&str, f64)>,
    ) -> SparkplugConfig {
        SparkplugConfig {
            broker_url: "tcp://localhost:1883".to_string(),
            group_id: "plant".to_string(),
            edge_node_id: "edge1".to_string(),
            bd_seq_path: dir.path().join("state/bd_seq"),
            metrics,
            deadband_by_prefix: deadbands
                .into_iter()
                .map(|(prefix, deadband)| (prefix.to_string(), deadband))
                .collect(),
            max_interval_ms: 1000,
        }
    }

    #[tokio::test]
    async fn connect_births_all_metrics_and_registers_death_will() {
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(Mutex::new(42i64));
        let running = Arc::new(Mutex::new(true));
        let mut publisher = SparkplugPublisher::new(
            config(
                &dir,
                vec![
                    int_source("line/count", &count),
                    bool_source("line/running", &running),
                ],
                vec![],
            ),
            MockTransport::default(),
        );

        publisher.connect(1_000).await.unwrap();

        // bdSeq incremented and persisted for crash recovery.
        assert_eq!(bdseq::load(&publisher.config.bd_seq_path).unwrap(), 1);

        // Will = NDEATH topic, payload carries the pending bdSeq, no seq.
        assert_eq!(publisher.transport.wills.len(), 1);
        let (will_topic, will_payload) = &publisher.transport.wills[0];
        assert_eq!(will_topic, "spBv1.0/plant/NDEATH/edge1");
        let will = Payload::decode(will_payload.as_slice()).unwrap();
        assert_eq!(will.seq, None);
        assert_eq!(will.metrics.len(), 1);
        assert_eq!(will.metrics[0].name.as_deref(), Some("bdSeq"));
        assert_eq!(will.metrics[0].value, Some(metric::Value::LongValue(1)));

        // NBIRTH: seq 0, every metric plus bdSeq.
        let births = publisher.transport.payloads("NBIRTH/edge1");
        assert_eq!(births.len(), 1);
        let birth = &births[0];
        assert_eq!(birth.seq, Some(0));
        assert_eq!(birth.metrics.len(), 3);
        assert_eq!(birth.metrics[0].name.as_deref(), Some("bdSeq"));
        assert_eq!(
            birth.metrics[1].name.as_deref(),
            Some("line/count"),
            "count metric birthed at 42"
        );
        assert_eq!(birth.metrics[1].value, Some(metric::Value::LongValue(42)));
        assert_eq!(birth.metrics[2].name.as_deref(), Some("line/running"));
        assert_eq!(birth.metrics[2].value, Some(metric::Value::BooleanValue(true)));

        // No change since birth → no NDATA.
        assert!(!publisher.publish_changes(1_500).await.unwrap());
        assert!(publisher.transport.payloads("NDATA/edge1").is_empty());
    }

    #[tokio::test]
    async fn sequence_numbers_increment_wrap_and_reset_on_birth() {
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(Mutex::new(0i64));
        let mut publisher = SparkplugPublisher::new(
            config(&dir, vec![int_source("line/count", &count)], vec![]),
            MockTransport::default(),
        );

        publisher.connect(0).await.unwrap();
        let assert_next_seq = |publisher: &SparkplugPublisher<MockTransport>, expected: u8| {
            let data = publisher.transport.payloads("NDATA/edge1");
            assert_eq!(
                data.last().and_then(|payload| payload.seq),
                Some(u64::from(expected))
            );
        };

        *count.lock() = 1;
        publisher.publish_changes(1).await.unwrap();
        assert_next_seq(&publisher, 1);

        *count.lock() = 2;
        publisher.publish_changes(2).await.unwrap();
        assert_next_seq(&publisher, 2);

        // seq wraps at 256 (0..=255 per spec).
        publisher.seq = 255;
        *count.lock() = 3;
        publisher.publish_changes(3).await.unwrap();
        assert_next_seq(&publisher, 255);
        *count.lock() = 4;
        publisher.publish_changes(4).await.unwrap();
        assert_next_seq(&publisher, 0);

        // Reconnect: new session — bdSeq bumps, NBIRTH resets seq to 0.
        publisher.connect(5).await.unwrap();
        assert_eq!(bdseq::load(&publisher.config.bd_seq_path).unwrap(), 2);
        assert_eq!(publisher.transport.wills.len(), 2);
        let will = Payload::decode(publisher.transport.wills[1].1.as_slice()).unwrap();
        assert_eq!(will.metrics[0].value, Some(metric::Value::LongValue(2)));
        let births = publisher.transport.payloads("NBIRTH/edge1");
        assert_eq!(births.len(), 2);
        assert_eq!(births[1].seq, Some(0));

        // The next NDATA continues from the new birth.
        *count.lock() = 5;
        publisher.publish_changes(6).await.unwrap();
        assert_next_seq(&publisher, 1);
    }

    #[tokio::test]
    async fn rbe_respects_deadband_until_max_interval() {
        let dir = tempfile::tempdir().unwrap();
        let level = Arc::new(Mutex::new(10.0f64));
        let mut publisher = SparkplugPublisher::new(
            config(
                &dir,
                vec![real_source("tank/level", &level)],
                vec![("tank", 2.0)],
            ),
            MockTransport::default(),
        );
        publisher.connect(0).await.unwrap();

        // |Δ| = 1.0 < deadband 2.0, interval not elapsed → suppressed.
        *level.lock() = 11.0;
        assert!(!publisher.publish_changes(100).await.unwrap());

        // |Δ| = 2.5 ≥ deadband → published.
        *level.lock() = 12.5;
        assert!(publisher.publish_changes(200).await.unwrap());
        let published = publisher.transport.payloads("NDATA/edge1");
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].metrics[0].name.as_deref(), Some("tank/level"));
        assert_eq!(published[0].metrics[0].value, Some(metric::Value::DoubleValue(12.5)));

        // Small wobble inside the deadband → suppressed.
        *level.lock() = 13.0;
        assert!(!publisher.publish_changes(300).await.unwrap());

        // Still inside the deadband, but the max interval since the last
        // publish (t=200) has elapsed → forced publish.
        *level.lock() = 13.4;
        assert!(!publisher.publish_changes(1_100).await.unwrap());
        *level.lock() = 13.5;
        assert!(publisher.publish_changes(1_201).await.unwrap());
        let published = publisher.transport.payloads("NDATA/edge1");
        assert_eq!(published.len(), 2);
        assert_eq!(
            published[1].metrics[0].value,
            Some(metric::Value::DoubleValue(13.5))
        );
    }

    #[tokio::test]
    async fn deadband_applies_by_prefix_only() {
        let dir = tempfile::tempdir().unwrap();
        let amps = Arc::new(Mutex::new(10i64));
        let running = Arc::new(Mutex::new(true));
        let mut publisher = SparkplugPublisher::new(
            config(
                &dir,
                vec![
                    int_source("motor/amps", &amps),
                    bool_source("motor/running", &running),
                ],
                vec![("motor/amps", 5.0)],
            ),
            MockTransport::default(),
        );
        publisher.connect(0).await.unwrap();

        // Amps wobble inside its deadband, bool unchanged → nothing.
        *amps.lock() = 13;
        assert!(!publisher.publish_changes(1).await.unwrap());

        // Bool change publishes immediately: no deadband for non-numerics.
        *running.lock() = false;
        assert!(publisher.publish_changes(2).await.unwrap());
        let published = publisher.transport.payloads("NDATA/edge1");
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].metrics.len(), 1);
        assert_eq!(published[0].metrics[0].name.as_deref(), Some("motor/running"));

        // Amps jump beyond the deadband → published on its own.
        *amps.lock() = 20;
        assert!(publisher.publish_changes(3).await.unwrap());
        let published = publisher.transport.payloads("NDATA/edge1");
        assert_eq!(published.len(), 2);
        assert_eq!(published[1].metrics.len(), 1);
        assert_eq!(published[1].metrics[0].name.as_deref(), Some("motor/amps"));
        assert_eq!(published[1].metrics[0].value, Some(metric::Value::LongValue(20)));
    }

    #[tokio::test]
    async fn metric_type_mapping_in_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let b = Arc::new(Mutex::new(true));
        let i = Arc::new(Mutex::new(-5i64));
        let r = Arc::new(Mutex::new(1.5f64));
        let s = Arc::new(Mutex::new("ok".to_string()));
        let string_source = {
            let s = s.clone();
            MetricSource {
                name: "misc/label".to_string(),
                get: Box::new(move || Some(MetricValue::String(s.lock().clone()))),
            }
        };
        let mut publisher = SparkplugPublisher::new(
            config(
                &dir,
                vec![
                    bool_source("misc/enabled", &b),
                    int_source("misc/offset", &i),
                    real_source("misc/gain", &r),
                    string_source,
                ],
                vec![],
            ),
            MockTransport::default(),
        );

        publisher.connect(0).await.unwrap();
        let birth = &publisher.transport.payloads("NBIRTH/edge1")[0];

        let by_name = |name: &str| {
            birth
                .metrics
                .iter()
                .find(|metric| metric.name.as_deref() == Some(name))
                .expect("metric in birth")
        };
        let enabled = by_name("misc/enabled");
        assert_eq!(enabled.datatype, Some(DataType::Boolean as u32));
        assert_eq!(enabled.value, Some(metric::Value::BooleanValue(true)));

        // Negative Int survives the uint64 long_value wire encoding.
        let offset = by_name("misc/offset");
        assert_eq!(offset.datatype, Some(DataType::Int64 as u32));
        assert_eq!(offset.value, Some(metric::Value::LongValue((-5i64) as u64)));

        let gain = by_name("misc/gain");
        assert_eq!(gain.datatype, Some(DataType::Double as u32));
        assert_eq!(gain.value, Some(metric::Value::DoubleValue(1.5)));

        let label = by_name("misc/label");
        assert_eq!(label.datatype, Some(DataType::String as u32));
        assert_eq!(
            label.value,
            Some(metric::Value::StringValue("ok".to_string()))
        );
    }

    #[tokio::test]
    async fn graceful_death_publishes_ndeath_with_matching_bd_seq() {
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(Mutex::new(0i64));
        let mut publisher = SparkplugPublisher::new(
            config(&dir, vec![int_source("line/count", &count)], vec![]),
            MockTransport::default(),
        );
        publisher.connect(0).await.unwrap();

        publisher.death(9_999).await.unwrap();
        let deaths = publisher.transport.payloads("NDEATH/edge1");
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].seq, None);
        assert_eq!(deaths[0].metrics.len(), 1);
        assert_eq!(deaths[0].metrics[0].name.as_deref(), Some("bdSeq"));
        assert_eq!(deaths[0].metrics[0].value, Some(metric::Value::LongValue(1)));
    }

    #[tokio::test]
    async fn absent_metric_misses_birth_and_stays_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let maybe = Arc::new(Mutex::new(None::<i64>));
        let source = {
            let maybe = maybe.clone();
            MetricSource {
                name: "line/gap".to_string(),
                get: Box::new(move || (*maybe.lock()).map(MetricValue::Int)),
            }
        };
        let mut publisher = SparkplugPublisher::new(
            config(&dir, vec![source], vec![]),
            MockTransport::default(),
        );
        publisher.connect(0).await.unwrap();

        // Absent at birth → not in the NBIRTH.
        let birth = &publisher.transport.payloads("NBIRTH/edge1")[0];
        assert!(birth.metrics.iter().all(|m| m.name.as_deref() != Some("line/gap")));

        // ...and never published mid-session, even once it becomes available.
        *maybe.lock() = Some(7);
        assert!(!publisher.publish_changes(1).await.unwrap());
        assert!(publisher.transport.payloads("NDATA/edge1").is_empty());

        // The next session births it with its current value.
        publisher.connect(2).await.unwrap();
        let birth = &publisher.transport.payloads("NBIRTH/edge1")[1];
        let gap = birth
            .metrics
            .iter()
            .find(|m| m.name.as_deref() == Some("line/gap"))
            .expect("gap metric birthed in second session");
        assert_eq!(gap.value, Some(metric::Value::LongValue(7)));
    }

    #[tokio::test]
    async fn unavailable_scan_keeps_published_state() {
        let dir = tempfile::tempdir().unwrap();
        let maybe = Arc::new(Mutex::new(Some(1i64)));
        let source = {
            let maybe = maybe.clone();
            MetricSource {
                name: "tank/level".to_string(),
                get: Box::new(move || (*maybe.lock()).map(MetricValue::Int)),
            }
        };
        let mut publisher = SparkplugPublisher::new(
            config(&dir, vec![source], vec![]),
            MockTransport::default(),
        );
        publisher.connect(0).await.unwrap();

        // A None scan is not an error and publishes nothing.
        *maybe.lock() = None;
        assert!(!publisher.publish_changes(1).await.unwrap());

        // State survives the gap: back to the same value → still no publish.
        *maybe.lock() = Some(1);
        assert!(!publisher.publish_changes(2).await.unwrap());

        // A real change after the gap publishes normally.
        *maybe.lock() = Some(3);
        assert!(publisher.publish_changes(3).await.unwrap());
        let published = publisher.transport.payloads("NDATA/edge1");
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].metrics[0].value, Some(metric::Value::LongValue(3)));
    }
}
