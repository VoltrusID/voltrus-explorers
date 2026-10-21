use crate::ProtocolError;

#[allow(async_fn_in_trait)]
pub trait ProtocolClient: Send + Sync + 'static {
    type Config: Send + Sync + serde::Serialize + serde::de::DeserializeOwned;
    type Tag: Send + Sync + serde::Serialize + serde::de::DeserializeOwned;
    type Value: Send + Sync + serde::Serialize + serde::de::DeserializeOwned + Clone;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError>
    where
        Self: Sized;
    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError>;
    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError>;
    async fn disconnect(&self) -> Result<(), ProtocolError>;
}
