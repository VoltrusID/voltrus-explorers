use crate::io_manager::{IoDriver, IoAddress, IoError, IoArea, IoSize};
use crate::types::PlcValue;
use mt_s7::{S7Client, S7Config, S7Tag, S7Area, S7DataType, S7Value};
use mt_protocol::ProtocolClient;

pub struct S7IoDriver {
    client: Option<S7Client>,
    host: String,
    port: u16,
    rack: u8,
    slot: u8,
}

impl S7IoDriver {
    pub fn new(host: String, port: u16, rack: u8, slot: u8) -> Self {
        Self { client: None, host, port, rack, slot }
    }

    pub async fn connect(&mut self) -> Result<(), IoError> {
        let config = S7Config {
            id: "voltrus-plc".into(),
            host: self.host.clone(),
            port: self.port,
            rack: self.rack,
            slot: self.slot,
            poll_interval_ms: 100,
        };
        let client = S7Client::connect(config).await.map_err(|e| IoError {
            message: format!("S7 connect: {}", e),
            recoverable: true,
        })?;
        self.client = Some(client);
        Ok(())
    }

    fn io_to_tag(&self, addr: &IoAddress) -> S7Tag {
        let area = match addr.area {
            IoArea::Input => S7Area::Input,
            IoArea::Output => S7Area::Output,
            IoArea::Memory => S7Area::Marker,
        };
        let data_type = match addr.size {
            IoSize::Bit => S7DataType::Bool,
            IoSize::Byte => S7DataType::Byte,
            IoSize::Word => S7DataType::Word,
            IoSize::Dword => S7DataType::DWord,
        };
        S7Tag {
            area,
            db_number: 0,
            offset: addr.offset as u16,
            bit_offset: addr.bit.unwrap_or(0) as u8,
            data_type,
            length: 1,
            name: None,
        }
    }

    fn s7_to_plc(val: &S7Value, _size: IoSize) -> PlcValue {
        match val {
            S7Value::Bool(b) => PlcValue::Bool(*b),
            S7Value::Byte(b) => PlcValue::Byte(*b),
            S7Value::Word(w) => PlcValue::Word(*w),
            S7Value::DWord(d) => PlcValue::Dint(*d as i32),
            S7Value::Real(r) => PlcValue::Real(*r),
            S7Value::String(_) => PlcValue::Dint(0),
        }
    }

    fn plc_to_s7(val: &PlcValue) -> S7Value {
        match val {
            PlcValue::Bool(b) => S7Value::Bool(*b),
            PlcValue::Byte(b) => S7Value::Byte(*b),
            PlcValue::Word(w) => S7Value::Word(*w),
            PlcValue::Int(i) => S7Value::Word(*i as u16),
            PlcValue::Dint(i) => S7Value::DWord(*i as u32),
            PlcValue::Real(f) => S7Value::Real(*f),
            _ => S7Value::Byte(0),
        }
    }
}

impl IoDriver for S7IoDriver {
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "S7 not connected".into(),
            recoverable: true,
        })?;
        let tags: Vec<S7Tag> = addresses.iter().map(|a| self.io_to_tag(a)).collect();
        let rt = tokio::runtime::Handle::current();
        let values = rt.block_on(async { client.read(&tags).await })
            .map_err(|e| IoError { message: format!("S7 read: {}", e), recoverable: true })?;
        Ok(values.iter().zip(addresses.iter()).map(|(v, a)| Self::s7_to_plc(v, a.size)).collect())
    }

    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError> {
        let client = self.client.as_ref().ok_or(IoError {
            message: "S7 not connected".into(),
            recoverable: true,
        })?;
        let rt = tokio::runtime::Handle::current();
        for (addr, val) in mappings {
            let tag = self.io_to_tag(addr);
            let s7_val = Self::plc_to_s7(val);
            rt.block_on(async { client.write(&tag, s7_val).await })
                .map_err(|e| IoError { message: format!("S7 write: {}", e), recoverable: true })?;
        }
        Ok(())
    }

    fn name(&self) -> &str { "s7" }

    fn is_connected(&self) -> bool { self.client.is_some() }

    fn disconnect(&mut self) {
        if let Some(ref client) = self.client {
            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(async { client.disconnect().await });
        }
        self.client = None;
    }
}
