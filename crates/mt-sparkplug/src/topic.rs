//! Sparkplug topic construction: `spBv1.0/{group}/{verb}/{edge_node}`.

/// The edge-node-level Sparkplug verbs this publisher emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparkplugVerb {
    NBirth,
    NDeath,
    NData,
}

impl SparkplugVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            SparkplugVerb::NBirth => "NBIRTH",
            SparkplugVerb::NDeath => "NDEATH",
            SparkplugVerb::NData => "NDATA",
        }
    }
}

/// Build `spBv1.0/{group_id}/{verb}/{edge_node_id}` (the Device-ID token is
/// never present for node-level verbs).
pub fn sparkplug_topic(group_id: &str, edge_node_id: &str, verb: SparkplugVerb) -> String {
    format!("spBv1.0/{group_id}/{}/{}", verb.as_str(), edge_node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_layout() {
        assert_eq!(
            sparkplug_topic("plant", "edge1", SparkplugVerb::NBirth),
            "spBv1.0/plant/NBIRTH/edge1"
        );
        assert_eq!(
            sparkplug_topic("plant", "edge1", SparkplugVerb::NDeath),
            "spBv1.0/plant/NDEATH/edge1"
        );
        assert_eq!(
            sparkplug_topic("plant", "edge1", SparkplugVerb::NData),
            "spBv1.0/plant/NDATA/edge1"
        );
    }
}
