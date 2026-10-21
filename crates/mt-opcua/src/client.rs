use std::sync::Arc;
use tokio::sync::Mutex;

use mt_protocol::{ProtocolClient, ProtocolError};

use crate::opcua_client::OpcUaProtocolClient;
use crate::types::{OpcUaConfig, OpcUaNode, OpcUaValue};

pub struct OpcUaClient {
    inner: Arc<Mutex<OpcUaProtocolClient>>,
}

impl OpcUaClient {
    pub fn inner(&self) -> Arc<Mutex<OpcUaProtocolClient>> {
        self.inner.clone()
    }
}

impl ProtocolClient for OpcUaClient {
    type Config = OpcUaConfig;
    type Tag = OpcUaNode;
    type Value = OpcUaValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let mut inner = OpcUaProtocolClient::new();
        inner.connect(config).await.map_err(ProtocolError::ConnectionFailed)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let inner = self.inner.lock().await;
        inner.read_tags(tags).await.map_err(ProtocolError::ReadFailed)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let inner = self.inner.lock().await;
        inner.write_tag(tag, &value).await.map_err(ProtocolError::WriteFailed)
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        let mut inner = self.inner.lock().await;
        inner.disconnect().map_err(ProtocolError::Other)
    }
}
