//! Glue between the gRPC runtime service and the Sparkplug publisher.
//!
//! The gRPC server owns the `PlcRuntime` behind a tokio `RwLock` and exposes
//! variables through an async accessor, while [`MetricSource::get`] is a sync
//! closure. [`VariableCache`] bridges the two: the publisher task refreshes
//! it from the server each scan, and the metric closures read it without
//! blocking the async runtime.

use std::sync::Arc;

use mt_plc_types::PlcValue;
use mt_sparkplug::{MetricSource, MetricValue};

/// Synchronized snapshot of the sampled VM variables.
pub struct VariableCache {
    names: Vec<String>,
    values: parking_lot::Mutex<Vec<Option<PlcValue>>>,
}

impl VariableCache {
    /// A cache for `names`, initially empty (every metric unavailable).
    pub fn new(names: Vec<String>) -> Self {
        let count = names.len();
        Self {
            names,
            values: parking_lot::Mutex::new(vec![None; count]),
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Atomically replace the snapshot with the latest server sample.
    pub fn refresh(&self, snapshot: Vec<Option<PlcValue>>) {
        *self.values.lock() = snapshot;
    }

    /// One metric source per name; sampling reads the latest snapshot.
    /// Variables absent from the snapshot sample as `None`.
    pub fn metric_sources(self: &Arc<Self>) -> Vec<MetricSource> {
        self.names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let cache = Arc::clone(self);
                MetricSource {
                    name: name.clone(),
                    get: Box::new(move || {
                        cache.values.lock().get(i).and_then(|slot| slot.as_ref()).map(MetricValue::from)
                    }),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_snapshot_by_name() {
        let cache = Arc::new(VariableCache::new(vec![
            "pump".to_string(),
            "level".to_string(),
            "ghost".to_string(),
        ]));
        let sources = cache.metric_sources();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0].name, "pump");

        // Nothing refreshed yet: every sample is None.
        assert_eq!((sources[0].get)(), None);

        cache.refresh(vec![
            Some(PlcValue::Bool(true)),
            Some(PlcValue::Real(2.5)),
            None,
        ]);
        assert_eq!((sources[0].get)(), Some(MetricValue::Bool(true)));
        assert_eq!((sources[1].get)(), Some(MetricValue::Real(2.5)));
        assert_eq!((sources[2].get)(), None);

        // A stale variable drops out on the next refresh.
        cache.refresh(vec![None, Some(PlcValue::Real(2.75)), None]);
        assert_eq!((sources[0].get)(), None);
        assert_eq!((sources[1].get)(), Some(MetricValue::Real(2.75)));
    }
}
