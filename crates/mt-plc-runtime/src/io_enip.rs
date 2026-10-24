use crate::io_manager::{IoAddress, IoArea, IoDriver, IoError, IoSize};
use crate::types::PlcValue;
use mt_enip::{CipDataType, CipTag, CipValue, EnipClient, EnipConfig};
use mt_protocol::ProtocolClient;

pub struct EnipIoDriver {
    client: Option<EnipClient>,
    host: String,
    port: u16,
}

impl EnipIoDriver {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            client: None,
            host,
            port,
        }
    }

    pub async fn connect(&mut self) -> Result<(), IoError> {
        let config = EnipConfig {
            id: format!("voltrus-{}", self.host),
            host: self.host.clone(),
            port: self.port,
            poll_interval_ms: 1000,
        };
        let client = EnipClient::connect(config).await.map_err(|e| IoError {
            message: format!("EtherNet/IP connect: {}", e),
            recoverable: true,
        })?;
        self.client = Some(client);
        Ok(())
    }

    fn io_to_tag(&self, addr: &IoAddress) -> CipTag {
        let area_prefix = match addr.area {
            IoArea::Input => "I",
            IoArea::Output => "O",
            IoArea::Memory => "M",
        };
        let tag_name = match addr.bit {
            Some(bit) => format!("{}{}_{}", area_prefix, addr.offset, bit),
            None => format!("{}{}", area_prefix, addr.offset),
        };
        CipTag {
            name: tag_name,
            data_type: match addr.size {
                IoSize::Bit => CipDataType::Bool,
                IoSize::Byte => CipDataType::Sint,
                IoSize::Word => CipDataType::Int,
                IoSize::Dword => CipDataType::Dint,
            },
            element_count: 1,
            template_id: None,
        }
    }

    fn value_to_plc(val: &CipValue) -> PlcValue {
        match val {
            CipValue::Bool(b) => PlcValue::Bool(*b),
            CipValue::Sint(i) => PlcValue::Byte(*i as u8),
            CipValue::Int(i) => PlcValue::Int(*i),
            CipValue::Dint(i) => PlcValue::Dint(*i),
            CipValue::Real(f) => PlcValue::Real(*f),
            CipValue::String(s) => PlcValue::String(s.clone()),
            CipValue::Bytes(_) => PlcValue::Dint(0),
        }
    }

    fn plc_to_cip(val: &PlcValue) -> CipValue {
        match val {
            PlcValue::Bool(b) => CipValue::Bool(*b),
            PlcValue::Byte(b) => CipValue::Sint(*b as i8),
            PlcValue::Word(w) => CipValue::Int(*w as i16),
            PlcValue::Int(i) => CipValue::Int(*i as i16),
            PlcValue::Dint(i) => CipValue::Dint(*i),
            PlcValue::Real(f) => CipValue::Real(*f),
            PlcValue::String(s) => CipValue::String(s.clone()),
            _ => CipValue::Dint(0),
        }
    }
}

impl IoDriver for EnipIoDriver {
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "EtherNet/IP not connected".into(),
            recoverable: true,
        })?;
        let tags: Vec<CipTag> = addresses.iter().map(|a| self.io_to_tag(a)).collect();
        let rt = tokio::runtime::Handle::current();
        let values = rt
            .block_on(async { client.read(&tags).await })
            .map_err(|e| IoError {
                message: format!("ENIP read: {}", e),
                recoverable: true,
            })?;
        Ok(values.iter().map(|v| Self::value_to_plc(v)).collect())
    }

    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "EtherNet/IP not connected".into(),
            recoverable: true,
        })?;
        let rt = tokio::runtime::Handle::current();
        for (addr, plc_val) in mappings {
            let tag = self.io_to_tag(addr);
            let cip_val = Self::plc_to_cip(plc_val);
            rt.block_on(async { client.write(&tag, cip_val).await })
                .map_err(|e| IoError {
                    message: format!("ENIP write: {}", e),
                    recoverable: true,
                })?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "enip"
    }

    fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    fn disconnect(&mut self) {
        if let Some(ref client) = self.client {
            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(async { client.disconnect().await });
        }
        self.client = None;
    }
}
