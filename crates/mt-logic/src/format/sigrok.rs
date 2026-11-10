//! sigrok session files (`.sr`): a zip with `version` ("2"), an INI-style `metadata` and the
//! logic samples in `<capturefile>-1`, `-2`, ... chunks (version 2) or one `<capturefile>` entry
//! (version 1), `unitsize` bytes per sample, probe N in bit N-1.
//!
//! Written from the file format alone; no sigrok code is used.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::capture::{Capture, ChannelInfo, MAX_CHANNELS};
use crate::error::{LogicError, Result};
use crate::{fmt_rate, parse_rate};

/// Bytes per data entry when writing (libsigrok uses chunks of a few MB too).
const CHUNK: usize = 4 << 20;

fn ini(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with(';') || l.starts_with('#') {
            continue;
        }
        if let Some(s) = l.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.trim().to_string();
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            out.entry(section.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn fmt_err(e: impl std::fmt::Display) -> LogicError {
    LogicError::Format(format!("sigrok session: {e}"))
}

/// Reads a `.sr` file. Returns the capture and warnings (channels beyond 8 dropped, analog data ignored).
pub fn read_sr(r: impl Read + Seek) -> Result<(Capture, Vec<String>)> {
    let mut zip = ZipArchive::new(r).map_err(fmt_err)?;
    let mut warnings = Vec::new();
    let mut version = String::new();
    if let Ok(mut f) = zip.by_name("version") {
        f.read_to_string(&mut version)?;
    }
    let version = version.trim();
    if !version.is_empty() && version != "1" && version != "2" {
        return Err(fmt_err(format!("unsupported version {version}")));
    }
    let mut meta = String::new();
    zip.by_name("metadata").map_err(|_| fmt_err("no metadata in the file"))?.read_to_string(&mut meta)?;
    let m = ini(&meta);
    let (_, dev) = m
        .iter()
        .find(|(k, v)| k.starts_with("device") && v.contains_key("capturefile"))
        .ok_or_else(|| fmt_err("the file has no logic data (only analog channels?)"))?;
    let rate = dev.get("samplerate").and_then(|s| parse_rate(s)).ok_or_else(|| fmt_err("no sample rate"))?;
    let unitsize: usize = dev.get("unitsize").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let total: usize = dev.get("total probes").and_then(|s| s.parse().ok()).unwrap_or(unitsize * 8);
    if dev.get("total analog").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) > 0 {
        warnings.push("Analog channels in the file are not shown.".into());
    }
    if total > MAX_CHANNELS {
        warnings.push(format!("The file has {total} logic channels; the first {MAX_CHANNELS} are shown."));
    }
    let channels: Vec<ChannelInfo> = (1..=total.min(MAX_CHANNELS))
        .map(|i| ChannelInfo::named(dev.get(&format!("probe{i}")).cloned().unwrap_or_else(|| format!("D{}", i - 1))))
        .collect();
    let base = dev.get("capturefile").cloned().unwrap();

    // Data entries: `<base>` alone (v1) or `<base>-<n>` in numeric order (v2).
    let mut parts: Vec<(u64, String)> = zip
        .file_names()
        .filter_map(|n| {
            if n == base {
                Some((0, n.to_string()))
            } else {
                n.strip_prefix(&base).and_then(|r| r.strip_prefix('-')).and_then(|r| r.parse::<u64>().ok()).map(|i| (i, n.to_string()))
            }
        })
        .collect();
    parts.sort();
    if parts.is_empty() {
        return Err(fmt_err("no logic data in the file"));
    }
    let mut data = Vec::new();
    let mut raw = Vec::new();
    for (_, name) in parts {
        raw.clear();
        zip.by_name(&name).map_err(fmt_err)?.read_to_end(&mut raw)?;
        if unitsize == 1 {
            data.extend_from_slice(&raw);
        } else {
            data.extend(raw.chunks_exact(unitsize).map(|u| u[0]));
        }
        if data.len() as u64 > crate::engine::MAX_SAMPLES {
            return Err(fmt_err("the capture is larger than the app can hold"));
        }
    }
    Ok((Capture::new(rate, channels, data), warnings))
}

/// Writes a version 2 `.sr` file that PulseView and sigrok-cli open.
pub fn write_sr(w: impl Write + Seek, cap: &Capture) -> Result<()> {
    let mut zip = ZipWriter::new(w);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("version", opts).map_err(fmt_err)?;
    zip.write_all(b"2")?;
    let mut meta = String::from("[global]\nsigrok version=0.5.2\n\n[device 1]\ncapturefile=logic-1\n");
    meta.push_str(&format!("total probes={}\nsamplerate={}\ntotal analog=0\n", cap.channels.len(), fmt_rate(cap.sample_rate)));
    for (i, c) in cap.channels.iter().enumerate() {
        meta.push_str(&format!("probe{}={}\n", i + 1, c.name.replace(['\n', '='], " ")));
    }
    meta.push_str("unitsize=1\n");
    zip.start_file("metadata", opts).map_err(fmt_err)?;
    zip.write_all(meta.as_bytes())?;
    for (i, chunk) in cap.data().chunks(CHUNK).enumerate() {
        zip.start_file(format!("logic-1-{}", i + 1), opts).map_err(fmt_err)?;
        zip.write_all(chunk)?;
    }
    zip.finish().map_err(fmt_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn session(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut buf);
            for (n, d) in files {
                z.start_file(*n, SimpleFileOptions::default()).unwrap();
                z.write_all(d).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn reads_pulseview_v2_with_chunks_out_of_order() {
        let meta = b"[global]\nsigrok version=0.5.2\n\n[device 1]\ndriver=fx2lafw\ncapturefile=logic-1\ntotal probes=8\nsamplerate=2 MHz\ntotal analog=0\nprobe1=TX\nprobe2=RX\nprobe4=CS\nunitsize=1\n";
        let f = session(&[("version", b"2"), ("metadata", meta), ("logic-1-2", &[3, 4]), ("logic-1-10", &[5]), ("logic-1-1", &[1, 2])]);
        let (c, w) = read_sr(Cursor::new(f)).unwrap();
        assert!(w.is_empty());
        assert_eq!(c.sample_rate, 2_000_000);
        assert_eq!(c.data(), &[1, 2, 3, 4, 5]);
        assert_eq!(c.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["TX", "RX", "D2", "CS", "D4", "D5", "D6", "D7"]);
    }

    #[test]
    fn reads_v1_and_16_channel_files() {
        let meta = b"[device 1]\ncapturefile=logic-1\ntotal probes=16\nsamplerate=24 MHz\nunitsize=2\n";
        let f = session(&[("version", b"1"), ("metadata", meta), ("logic-1", &[0x0F, 0xAA, 0xF0, 0xBB])]);
        let (c, w) = read_sr(Cursor::new(f)).unwrap();
        assert_eq!(c.data(), &[0x0F, 0xF0]);
        assert_eq!(c.channels.len(), 8);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn round_trip() {
        let mut ch = crate::capture::default_channels(8);
        ch[0].name = "CAN_RX".into();
        let cap = Capture::new(8_000_000, ch, (0..10_000u32).map(|i| (i % 251) as u8).collect());
        let mut buf = Cursor::new(Vec::new());
        write_sr(&mut buf, &cap).unwrap();
        let (back, _) = read_sr(Cursor::new(buf.into_inner())).unwrap();
        assert_eq!(back.data(), cap.data());
        assert_eq!(back.sample_rate, 8_000_000);
        assert_eq!(back.channels[0].name, "CAN_RX");
    }

    #[test]
    fn rejects_non_logic_files() {
        assert!(read_sr(Cursor::new(session(&[("metadata", b"[device 1]\nsamplerate=1 MHz\n")]))).is_err());
        assert!(read_sr(Cursor::new(b"not a zip".to_vec())).is_err());
    }
}
