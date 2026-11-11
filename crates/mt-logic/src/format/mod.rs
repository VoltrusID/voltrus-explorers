//! Capture files.
//!
//! - `.vla` (Voltrus Logic Analyzer): our own format, read and written. Magic line, a JSON header
//!   (sample rate, channel names and colours, trigger, and the app's session: decoders, cursors),
//!   then the samples deflate-compressed.
//! - `.sr` (sigrok session, version 2 and 1): read and written, so captures move between this
//!   app and PulseView / sigrok-cli. Only the file format is implemented here, no sigrok code.
//! - `.vcd` (Value Change Dump): written, for GTKWave and simulators.

pub mod sigrok;
pub mod vcd;

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde::{Deserialize, Serialize};

use crate::capture::{Capture, ChannelInfo};
use crate::error::{LogicError, Result};

pub const VLA_MAGIC: &[u8] = b"VOLTRUS-LOGIC\n";
pub const VLA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VlaHeader {
    version: u32,
    sample_rate: u64,
    samples: u64,
    channels: Vec<ChannelInfo>,
    #[serde(default)]
    trigger: Option<u64>,
    /// Anything the app wants back when the file is opened (decoder stack, cursors, view).
    #[serde(default)]
    session: serde_json::Value,
    #[serde(default)]
    created_by: String,
}

/// Writes a `.vla` file.
pub fn write_vla(w: &mut impl Write, cap: &Capture, session: &serde_json::Value) -> Result<()> {
    let header = VlaHeader {
        version: VLA_VERSION,
        sample_rate: cap.sample_rate,
        samples: cap.len(),
        channels: cap.channels.clone(),
        trigger: cap.trigger,
        session: session.clone(),
        created_by: format!("Voltrus Logic Analyzer (mt-logic {})", env!("CARGO_PKG_VERSION")),
    };
    let json = serde_json::to_vec(&header).map_err(|e| LogicError::Format(e.to_string()))?;
    w.write_all(VLA_MAGIC)?;
    w.write_all(&(json.len() as u32).to_le_bytes())?;
    w.write_all(&json)?;
    let mut enc = DeflateEncoder::new(w, Compression::fast());
    enc.write_all(cap.data())?;
    enc.finish()?;
    Ok(())
}

/// Reads a `.vla` file: the capture and the saved session.
pub fn read_vla(r: &mut impl Read) -> Result<(Capture, serde_json::Value)> {
    let mut magic = vec![0u8; VLA_MAGIC.len()];
    r.read_exact(&mut magic).map_err(|_| LogicError::Format("not a Voltrus Logic Analyzer file".into()))?;
    if magic != VLA_MAGIC {
        return Err(LogicError::Format("not a Voltrus Logic Analyzer file".into()));
    }
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 16 << 20 {
        return Err(LogicError::Format("the file header is damaged".into()));
    }
    let mut json = vec![0u8; len];
    r.read_exact(&mut json)?;
    let h: VlaHeader = serde_json::from_slice(&json).map_err(|e| LogicError::Format(format!("the file header is damaged: {e}")))?;
    if h.version > VLA_VERSION {
        return Err(LogicError::Format(format!("this file is from a newer version of the app (format {})", h.version)));
    }
    if h.samples > crate::engine::MAX_SAMPLES {
        return Err(LogicError::Format("the capture is larger than the app can hold".into()));
    }
    let mut data = Vec::with_capacity(h.samples as usize);
    DeflateDecoder::new(r).take(h.samples).read_to_end(&mut data)?;
    if data.len() as u64 != h.samples {
        return Err(LogicError::Format(format!("the file is cut short: {} of {} samples", data.len(), h.samples)));
    }
    let mut channels = h.channels;
    channels.truncate(crate::capture::MAX_CHANNELS);
    Ok((Capture::new(h.sample_rate, channels, data).with_trigger(h.trigger), h.session))
}

/// What kind of file `bytes` (its start) is.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(VLA_MAGIC) {
        Some("vla")
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some("sr")
    } else {
        None
    }
}

/// Opens any supported capture file.
pub fn open(bytes: &[u8]) -> Result<(Capture, serde_json::Value, Vec<String>)> {
    match sniff(bytes) {
        Some("vla") => {
            let (c, s) = read_vla(&mut std::io::Cursor::new(bytes))?;
            Ok((c, s, vec![]))
        }
        Some("sr") => {
            let (c, warnings) = sigrok::read_sr(std::io::Cursor::new(bytes))?;
            Ok((c, serde_json::Value::Null, warnings))
        }
        _ => Err(LogicError::Format("unknown file type: open a .vla capture or a sigrok .sr session".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::default_channels;

    #[test]
    fn vla_round_trip() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i / 777) as u8).collect();
        let mut ch = default_channels(8);
        ch[0] = ChannelInfo { name: "RS-485 RO".into(), color: Some("#e5a50a".into()) };
        let cap = Capture::new(1_000_000, ch, data.clone()).with_trigger(Some(1234));
        let session = serde_json::json!({ "decoders": [{ "kind": "modbusRtu" }] });
        let mut buf = Vec::new();
        write_vla(&mut buf, &cap, &session).unwrap();
        assert!(buf.len() < data.len() / 20, "{} bytes", buf.len());
        let (back, s, warn) = open(&buf).unwrap();
        assert!(warn.is_empty());
        assert_eq!(back.data(), &data[..]);
        assert_eq!(back.sample_rate, 1_000_000);
        assert_eq!(back.trigger, Some(1234));
        assert_eq!(back.channels[0].name, "RS-485 RO");
        assert_eq!(back.channels[0].color.as_deref(), Some("#e5a50a"));
        assert_eq!(s, session);
    }

    #[test]
    fn vla_rejects_damage() {
        let cap = Capture::new(1000, default_channels(8), vec![1; 5000]);
        let mut buf = Vec::new();
        write_vla(&mut buf, &cap, &serde_json::Value::Null).unwrap();
        assert!(open(&buf[..buf.len() - 4]).is_err());
        assert!(open(b"hello").is_err());
        let mut bad = buf.clone();
        bad[VLA_MAGIC.len() + 5] = b'#';
        assert!(open(&bad).is_err());
    }
}
