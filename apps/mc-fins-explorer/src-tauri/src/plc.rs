//! One connection to a PLC, Mitsubishi (MC protocol) or Omron (FINS), behind one word/bit interface.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mt_fins::{FinsAddress, FinsArea, FinsClient, FinsConfig};
use mt_melsec::{McAddress, McClient, McConfig, McDevice};

use crate::values::{self, Batch, DataType, Loc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    Mc(McDevice),
    Fins(FinsArea),
}

/// What the frontend sends to connect.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum ConnectConfig {
    Mc {
        #[serde(flatten)]
        config: McConfig,
        /// iQ-F (FX5): X and Y numbered in octal.
        #[serde(default)]
        xy_octal: bool,
    },
    Fins {
        #[serde(flatten)]
        config: FinsConfig,
    },
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PlcInfo {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// SA1/DA1 for FINS, the route for MC.
    pub route: String,
}

pub enum Plc {
    Mc { client: McClient, xy_octal: bool },
    Fins(FinsClient),
}

/// A parsed tag: where it is and how to read it.
#[derive(Debug, Clone, Copy)]
pub struct Tag {
    pub loc: Loc<Area>,
    pub dt: DataType,
    /// The address is a whole bit (a Mitsubishi bit device, or a word bit like D100.F / CIO0.05).
    pub is_bit: bool,
}

impl Plc {
    /// Words per batch read, and how far apart two ranges may be and still be read together.
    fn batch_limits(&self) -> (u64, u64) {
        match self {
            Plc::Mc { .. } => (16, mt_melsec::MAX_WORDS as u64),
            Plc::Fins(_) => (16, mt_fins::MAX_WORDS as u64),
        }
    }

    /// Parse an address with a data type into a location.
    pub fn tag(&self, address: &str, data_type: &str) -> Result<Tag, String> {
        let dt = DataType::from_name(data_type).ok_or_else(|| format!("unknown data type '{data_type}'"))?;
        let (area, start_bit, is_bit) = match self {
            Plc::Mc { xy_octal, .. } => {
                let a = mt_melsec::parse_address(address, *xy_octal)?;
                if a.device.is_bit() {
                    (Area::Mc(a.device), a.number as u64, true)
                } else {
                    (Area::Mc(a.device), a.number as u64 * 16 + a.bit.unwrap_or(0) as u64, a.bit.is_some())
                }
            }
            Plc::Fins(_) => {
                let a = mt_fins::parse_address(address)?;
                (Area::Fins(a.area), a.word as u64 * 16 + a.bit.unwrap_or(0) as u64, a.bit.is_some())
            }
        };
        let word_bit = is_bit && !matches!(area, Area::Mc(d) if d.is_bit());
        if dt == DataType::Bool && !is_bit {
            return Err(format!("{address} is a word: give a bit for Bool (for example {address}.0)"));
        }
        if dt != DataType::Bool && word_bit {
            return Err(format!("{address} is a single bit: use the Bool type, or the word address without .bit"));
        }
        Ok(Tag { loc: Loc { area, start_bit, nbits: dt.bits() }, dt, is_bit })
    }

    async fn read_batch(&self, b: &Batch<Area>) -> Result<Vec<u16>, String> {
        match (self, b.area) {
            (Plc::Mc { client, .. }, Area::Mc(d)) => {
                let head = if d.is_bit() { b.first * 16 } else { b.first };
                client.read_words(McAddress::new(d, head as u32), b.count as usize).await.map_err(|e| e.to_string())
            }
            (Plc::Fins(client), Area::Fins(a)) => {
                client.read_words(FinsAddress::new(a, b.first as u16), b.count as usize).await.map_err(|e| e.to_string())
            }
            _ => Err("address does not belong to this PLC's protocol".into()),
        }
    }

    /// Read many tags with as few requests as possible.
    pub async fn read(&self, tags: &[Tag]) -> Result<Vec<Value>, String> {
        let (gap, max) = self.batch_limits();
        let locs: Vec<Loc<Area>> = tags.iter().map(|t| t.loc).collect();
        let batches = values::plan(&locs, gap, max);
        let mut data: HashMap<(Area, u64), Vec<u16>> = HashMap::new();
        for b in &batches {
            data.insert((b.area, b.first), self.read_batch(b).await?);
        }
        tags.iter()
            .map(|t| {
                batches
                    .iter()
                    .filter(|b| b.area == t.loc.area)
                    .find_map(|b| values::value_at(&data[&(b.area, b.first)], b.first, t.loc.start_bit, t.dt))
                    .ok_or_else(|| "internal error: a tag fell outside the batches read".to_string())
            })
            .collect()
    }

    /// Raw words from a head address (the device monitor). On a Mitsubishi bit device each word is 16 devices.
    pub async fn read_block(&self, head: &str, count: u32) -> Result<Vec<u16>, String> {
        let probe = self.tag(head, "UInt16").or_else(|_| self.tag(head, "Bool"))?;
        if !matches!(probe.loc.area, Area::Mc(d) if d.is_bit()) && probe.is_bit {
            return Err("give a word address (without .bit) for the monitor".into());
        }
        let loc = Loc { area: probe.loc.area, start_bit: probe.loc.start_bit, nbits: count * 16 };
        let (_, max) = self.batch_limits();
        let first = loc.start_bit / 16;
        let last = (loc.start_bit + loc.nbits as u64 - 1) / 16;
        let mut words = Vec::new();
        let mut at = first;
        while at <= last {
            let n = (last - at + 1).min(max);
            words.extend(self.read_batch(&Batch { area: loc.area, first: at, count: n }).await?);
            at += n;
        }
        values::extract(&words, first, loc.start_bit, loc.nbits).ok_or_else(|| "short read".into())
    }

    pub async fn write(&self, address: &str, data_type: &str, value: &Value) -> Result<(), String> {
        let tag = self.tag(address, data_type)?;
        let words = values::encode(tag.dt, value)?;
        if tag.dt == DataType::Bool {
            return self.write_bit(tag.loc.area, tag.loc.start_bit, words[0] != 0).await;
        }
        self.write_words_at(tag.loc.area, tag.loc.start_bit, &words).await
    }

    /// Write words whose first bit is `start_bit` (word-aligned except on Mitsubishi bit devices).
    pub async fn write_words_at(&self, area: Area, start_bit: u64, words: &[u16]) -> Result<(), String> {
        match (self, area) {
            (Plc::Mc { client, .. }, Area::Mc(d)) if d.is_bit() => {
                // Bit units work for any head number, aligned or not.
                let bits: Vec<bool> = (0..words.len() * 16).map(|i| words[i / 16] & (1 << (i % 16)) != 0).collect();
                client.write_bits(McAddress::new(d, start_bit as u32), &bits).await.map_err(|e| e.to_string())
            }
            (Plc::Mc { client, .. }, Area::Mc(d)) => {
                client.write_words(McAddress::new(d, (start_bit / 16) as u32), words).await.map_err(|e| e.to_string())
            }
            (Plc::Fins(client), Area::Fins(a)) => {
                client.write_words(FinsAddress::new(a, (start_bit / 16) as u16), words).await.map_err(|e| e.to_string())
            }
            _ => Err("address does not belong to this PLC's protocol".into()),
        }
    }

    pub async fn write_bit(&self, area: Area, abs_bit: u64, on: bool) -> Result<(), String> {
        match (self, area) {
            (Plc::Mc { client, .. }, Area::Mc(d)) if d.is_bit() => {
                client.write_bits(McAddress::new(d, abs_bit as u32), &[on]).await.map_err(|e| e.to_string())
            }
            (Plc::Mc { client, .. }, Area::Mc(d)) => {
                // MC protocol has no Q-compatible bit write into a word device: read the word, change the bit, write it.
                // Anything the CPU changes in the other 15 bits between the two requests is overwritten.
                let at = McAddress::new(d, (abs_bit / 16) as u32);
                let w = client.read_words(at, 1).await.map_err(|e| e.to_string())?[0];
                let mask = 1u16 << (abs_bit % 16);
                let next = if on { w | mask } else { w & !mask };
                client.write_words(at, &[next]).await.map_err(|e| e.to_string())
            }
            (Plc::Fins(client), Area::Fins(a)) => {
                let at = FinsAddress { area: a, word: (abs_bit / 16) as u16, bit: Some((abs_bit % 16) as u8) };
                client.write_bits(at, &[on]).await.map_err(|e| e.to_string())
            }
            _ => Err("address does not belong to this PLC's protocol".into()),
        }
    }

    pub async fn info(&self) -> Result<PlcInfo, String> {
        match self {
            Plc::Mc { client, .. } => {
                let (model, code) = client.read_cpu_model().await.map_err(|e| e.to_string())?;
                let r = client.config().route;
                Ok(PlcInfo {
                    model,
                    model_code: Some(format!("0x{code:04X}")),
                    route: format!("network {}, PC {:02X}, module I/O {:04X}, station {}", r.network, r.pc, r.io, r.station),
                    ..Default::default()
                })
            }
            Plc::Fins(client) => {
                let data = client.controller_data().await.map_err(|e| e.to_string())?;
                let st = client.controller_status().await.map_err(|e| e.to_string())?;
                let (sa1, da1) = client.nodes().await.unwrap_or_default();
                let mut errors = Vec::new();
                if st.fatal_error != 0 {
                    errors.push(format!("fatal error flags 0x{:04X}", st.fatal_error));
                }
                if st.non_fatal_error != 0 {
                    errors.push(format!("non-fatal error flags 0x{:04X}", st.non_fatal_error));
                }
                if !st.error_message.is_empty() {
                    errors.push(st.error_message.clone());
                }
                Ok(PlcInfo {
                    model: data.model,
                    version: Some(data.version),
                    status: Some(st.status),
                    mode: Some(st.mode),
                    error: (!errors.is_empty()).then(|| errors.join("; ")),
                    route: format!("SA1 {sa1} → DA1 {da1}, DA2 {}", client.config().dest_unit),
                    model_code: None,
                })
            }
        }
    }

    pub async fn run(&self) -> Result<(), String> {
        match self {
            Plc::Mc { client, .. } => client.remote_run(false).await.map_err(|e| e.to_string()),
            Plc::Fins(c) => c.run(mt_fins::RunMode::Monitor).await.map_err(|e| e.to_string()),
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        match self {
            Plc::Mc { client, .. } => client.remote_stop().await.map_err(|e| e.to_string()),
            Plc::Fins(c) => c.stop().await.map_err(|e| e.to_string()),
        }
    }

    pub async fn disconnect(&self) {
        match self {
            Plc::Mc { client, .. } => client.disconnect().await,
            Plc::Fins(c) => c.disconnect().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_fins::{FinsServer, Transport as FinsTransport};
    use mt_melsec::{Encoding, FrameKind, McServer, Transport as McTransport};
    use serde_json::json;

    async fn mc(port: u16, transport: McTransport, frame: FrameKind, encoding: Encoding) -> Plc {
        let mut cfg = McConfig::new("127.0.0.1", port);
        cfg.transport = transport;
        cfg.frame = frame;
        cfg.encoding = encoding;
        Plc::Mc { client: McClient::connect(cfg, None).await.unwrap(), xy_octal: false }
    }

    async fn fins(port: u16, transport: FinsTransport) -> Plc {
        let mut cfg = FinsConfig::new("127.0.0.1", port);
        cfg.transport = transport;
        Plc::Fins(FinsClient::connect(cfg, None).await.unwrap())
    }

    async fn round_trip(plc: &Plc, cases: &[(&str, &str, Value)]) {
        for (addr, dt, v) in cases {
            plc.write(addr, dt, v).await.unwrap_or_else(|e| panic!("write {addr} {dt}: {e}"));
        }
        let tags: Vec<Tag> = cases.iter().map(|(a, d, _)| plc.tag(a, d).unwrap()).collect();
        let got = plc.read(&tags).await.unwrap();
        for ((addr, dt, want), got) in cases.iter().zip(got) {
            assert_eq!(&got, want, "{addr} {dt}");
        }
    }

    #[tokio::test]
    async fn mitsubishi_tags_round_trip_through_the_simulator() {
        let sim = McServer::start(0, None).await.unwrap();
        for (t, f, e) in [
            (McTransport::Tcp, FrameKind::E3, Encoding::Binary),
            (McTransport::Udp, FrameKind::E4, Encoding::Ascii),
        ] {
            let plc = mc(sim.port(), t, f, e).await;
            round_trip(
                &plc,
                &[
                    ("D100", "Int16", json!(-42)),
                    ("D101", "UInt32", json!(123456)),
                    ("D103", "Float32", json!(1450.5)),
                    ("D105.F", "Bool", json!(true)),
                    ("D105.0", "Bool", json!(true)),
                    ("M101", "Bool", json!(true)),
                    ("M5", "UInt16", json!(0xABCD)),
                    ("X1F", "Bool", json!(true)),
                    ("W1A", "Hex16", json!("0xBEEF")),
                    ("R2000", "BCD16", json!(1234)),
                    ("ZR60000", "UInt16", json!(9)),
                ],
            )
            .await;
        }
    }

    #[tokio::test]
    async fn omron_tags_round_trip_through_the_simulator() {
        let sim = FinsServer::start(0, 10, None).await.unwrap();
        for t in [FinsTransport::Udp, FinsTransport::Tcp] {
            let plc = fins(sim.port(), t).await;
            round_trip(
                &plc,
                &[
                    ("D100", "Int16", json!(-42)),
                    ("D101", "Int32", json!(-100000)),
                    ("D103", "Float32", json!(2.5)),
                    ("CIO0.05", "Bool", json!(true)),
                    ("W10.15", "Bool", json!(true)),
                    ("H0", "UInt16", json!(7)),
                    ("A448", "UInt16", json!(1)),
                    ("E0_10", "Hex16", json!("0x00FF")),
                    ("T5", "BCD16", json!(250)),
                ],
            )
            .await;
        }
    }

    #[tokio::test]
    async fn word_bit_write_keeps_the_other_bits() {
        let sim = McServer::start(0, None).await.unwrap();
        let plc = mc(sim.port(), McTransport::Tcp, FrameKind::E3, Encoding::Binary).await;
        plc.write("D10", "UInt16", &json!(0x00F0)).await.unwrap();
        plc.write("D10.0", "Bool", &json!(true)).await.unwrap();
        plc.write("D10.4", "Bool", &json!(false)).await.unwrap();
        let v = plc.read(&[plc.tag("D10", "Hex16").unwrap()]).await.unwrap();
        assert_eq!(v, vec![json!("0x00E1")]);
    }

    #[tokio::test]
    async fn block_read_of_bit_and_word_devices() {
        let sim = McServer::start(0, None).await.unwrap();
        let plc = mc(sim.port(), McTransport::Tcp, FrameKind::E3, Encoding::Binary).await;
        plc.write("D0", "UInt16", &json!(1)).await.unwrap();
        plc.write("D1", "UInt16", &json!(2)).await.unwrap();
        assert_eq!(plc.read_block("D0", 3).await.unwrap(), vec![1, 2, 0]);
        plc.write("M3", "Bool", &json!(true)).await.unwrap();
        assert_eq!(plc.read_block("M0", 2).await.unwrap(), vec![1 << 3, 0]);
        // An unaligned head on a bit device: M3 becomes bit 0.
        assert_eq!(plc.read_block("M3", 1).await.unwrap(), vec![1]);
        assert!(plc.read_block("D0.1", 1).await.is_err());
    }

    #[tokio::test]
    async fn info_and_run_stop() {
        let sim = FinsServer::start(0, 10, None).await.unwrap();
        let plc = fins(sim.port(), FinsTransport::Udp).await;
        plc.stop().await.unwrap();
        let info = plc.info().await.unwrap();
        assert_eq!((info.model.as_str(), info.mode.as_deref()), ("CJ2M-CPU33", Some("PROGRAM")));
        plc.run().await.unwrap();
        assert_eq!(plc.info().await.unwrap().mode.as_deref(), Some("MONITOR"));

        let msim = McServer::start(0, None).await.unwrap();
        let m = mc(msim.port(), McTransport::Tcp, FrameKind::E3, Encoding::Binary).await;
        assert_eq!(m.info().await.unwrap().model, "Q03UDVCPU");
        m.stop().await.unwrap();
        assert!(!msim.memory().lock().unwrap().running);
    }

    #[test]
    fn type_and_address_must_agree() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sim = McServer::start(0, None).await.unwrap();
            let plc = mc(sim.port(), McTransport::Tcp, FrameKind::E3, Encoding::Binary).await;
            assert!(plc.tag("D100", "Bool").unwrap_err().contains("D100.0"));
            assert!(plc.tag("D100.1", "Int16").is_err());
            assert!(plc.tag("M0", "Bool").is_ok());
            assert!(plc.tag("M0", "Int32").is_ok());
            assert!(plc.tag("D100", "Real").is_err());
        });
    }
}
