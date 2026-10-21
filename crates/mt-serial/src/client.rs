use mt_protocol::{ProtocolClient, ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::SerialStream;

use crate::types::{SerialConfig, SerialTag, SerialValue};

/// Async serial port client implementing the [`ProtocolClient`] trait.
///
/// Uses `tokio_serial::SerialStream` for non-blocking I/O on RS232 / RS485
/// serial ports.
pub struct SerialClient {
    port: tokio::sync::Mutex<SerialStream>,
    timeout_ms: u64,
}

impl ProtocolClient for SerialClient {
    type Config = SerialConfig;
    type Tag = SerialTag;
    type Value = SerialValue;

    async fn connect(config: Self::Config) -> Result<Self, ProtocolError> {
        let builder = config
            .to_serial_port_builder()
            .map_err(|e| ProtocolError::InvalidConfig(e))?;

        let stream = SerialStream::open(&builder)
            .map_err(|e| ProtocolError::ConnectionFailed(format!("{e}")))?;

        Ok(Self {
            port: tokio::sync::Mutex::new(stream),
            timeout_ms: config.timeout_ms,
        })
    }

    async fn read(&self, tags: &[Self::Tag]) -> Result<Vec<Self::Value>, ProtocolError> {
        let mut port = self.port.lock().await;
        let timeout = std::time::Duration::from_millis(self.timeout_ms);
        let mut results = Vec::with_capacity(tags.len());

        for tag in tags {
            // For raw serial, each tag read means: write the tag name/address
            // as a request, then read back the response.  In many RS485 setups
            // the "tag" is a query command so we send the tag name as bytes
            // and read whatever comes back.
            let request = format!("{}\r\n", tag.name);
            tokio::time::timeout(timeout, port.write_all(request.as_bytes()))
                .await
                .map_err(|_| ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name)))?
                .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?;

            // Read response into a buffer.  We read up to 1024 bytes or until
            // the timeout elapses.
            let mut buf = vec![0u8; 1024];
            let n = tokio::time::timeout(timeout, port.read(&mut buf))
                .await
                .map_err(|_| ProtocolError::Timeout(format!("Read timeout for tag '{}'", tag.name)))?
                .map_err(|e| ProtocolError::ReadFailed(format!("{e}")))?;

            buf.truncate(n);
            results.push(SerialValue::new(buf));
        }

        Ok(results)
    }

    async fn write(&self, tag: &Self::Tag, value: Self::Value) -> Result<(), ProtocolError> {
        let mut port = self.port.lock().await;
        let timeout = std::time::Duration::from_millis(self.timeout_ms);

        // For raw serial, a write sends the tag name followed by the raw data
        // bytes to the serial port.
        let mut payload = Vec::with_capacity(tag.name.len() + 1 + value.data.len());
        payload.extend_from_slice(tag.name.as_bytes());
        payload.push(b'=');
        payload.extend_from_slice(&value.data);

        tokio::time::timeout(timeout, port.write_all(&payload))
            .await
            .map_err(|_| ProtocolError::Timeout(format!("Write timeout for tag '{}'", tag.name)))?
            .map_err(|e| ProtocolError::WriteFailed(format!("{e}")))?;

        // Flush to ensure data is sent.
        tokio::time::timeout(timeout, port.flush())
            .await
            .map_err(|_| ProtocolError::Timeout("Flush timeout".into()))?
            .map_err(|e| ProtocolError::WriteFailed(format!("Flush failed: {e}")))?;

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), ProtocolError> {
        // SerialStream closes on drop.  We lock and explicitly drop to close now.
        let mut port = self.port.lock().await;
        let _ = port.flush().await;
        Ok(())
    }
}

/// List all available serial ports on the system.
pub fn list_ports() -> Result<Vec<crate::types::PortInfo>, ProtocolError> {
    serialport::available_ports()
        .map(|ports| ports.iter().map(crate::types::PortInfo::from).collect())
        .map_err(|e| ProtocolError::Other(format!("Failed to enumerate ports: {e}")))
}

/// Perform a single read from a serial device without keeping a persistent client.
pub async fn read_tags(
    config: &SerialConfig,
    tags: &[SerialTag],
) -> Result<Vec<SerialValue>, ProtocolError> {
    let client = SerialClient::connect(config.clone()).await?;
    let values = client.read(tags).await?;
    client.disconnect().await?;
    Ok(values)
}

/// Perform a single write to a serial device without keeping a persistent client.
pub async fn write_tag(
    config: &SerialConfig,
    tag: &SerialTag,
    value: SerialValue,
) -> Result<(), ProtocolError> {
    let client = SerialClient::connect(config.clone()).await?;
    client.write(tag, value).await?;
    client.disconnect().await?;
    Ok(())
}
