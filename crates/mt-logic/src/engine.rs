//! Capture from any sample stream (the FX2 driver or the simulator): wait for the trigger while
//! keeping only the pre-trigger samples in a ring, then record up to the requested length. Memory
//! is bounded by the capture length, which is itself capped at [`MAX_SAMPLES`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::capture::{Capture, ChannelInfo};
use crate::error::{LogicError, Result};
use crate::trigger::{Trigger, TriggerState};

/// 512 Mi samples (512 MB of RAM at one byte each), about 21 s at 24 MHz.
pub const MAX_SAMPLES: u64 = 512 * 1024 * 1024;

/// A source of packed 8-channel samples.
pub trait SampleStream: Send {
    /// Appends the next chunk to `buf`. `Ok(false)` once the source has no more samples.
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool>;
    fn sample_rate(&self) -> u64;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSettings {
    pub sample_rate: u64,
    /// Total length, pre-trigger included.
    pub samples: u64,
    #[serde(default)]
    pub trigger: Trigger,
    /// Share of the capture before the trigger, 0 to 0.9.
    #[serde(default)]
    pub pre_trigger: f64,
}

impl CaptureSettings {
    pub fn validate(&self) -> Result<()> {
        if self.samples == 0 {
            return Err(LogicError::Config("the capture length must be above zero".into()));
        }
        if self.samples > MAX_SAMPLES {
            return Err(LogicError::Config(format!("the capture length is limited to {} Mi samples", MAX_SAMPLES >> 20)));
        }
        if !(0.0..=0.9).contains(&self.pre_trigger) {
            return Err(LogicError::Config("the pre-trigger share must be 0 to 90 %".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Waiting,
    Recording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    /// Samples recorded so far (pre-trigger included once triggered).
    pub samples: u64,
    pub total: u64,
}

/// Runs a capture to completion, or until `stop` is set: then the samples so far are returned
/// (before a trigger, the pre-trigger ring with no trigger mark).
pub fn run(stream: &mut dyn SampleStream, settings: &CaptureSettings, channels: Vec<ChannelInfo>, stop: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<Capture> {
    settings.validate()?;
    let total = settings.samples as usize;
    let mut trig = TriggerState::new(settings.trigger.clone());
    let pre = if trig.is_immediate() { 0 } else { (settings.samples as f64 * settings.pre_trigger) as usize };
    let mut ring: VecDeque<u8> = VecDeque::with_capacity(pre.max(1));
    let mut data: Vec<u8> = Vec::new();
    // Where the trigger fired in `data` (`Some(None)` once recording without a trigger mark).
    let mut triggered: Option<Option<u64>> = None;
    let mut chunk = Vec::with_capacity(1 << 16);
    let rate = settings.sample_rate;
    let finish = |data: Vec<u8>, t: Option<u64>| Capture::new(rate, channels.clone(), data).with_trigger(t);

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(match triggered {
                Some(t) => finish(data, t),
                None => finish(ring.into_iter().collect(), None),
            });
        }
        chunk.clear();
        let more = stream.read(&mut chunk)?;
        let mut rest: &[u8] = &chunk;
        if triggered.is_none() {
            match trig.scan(rest) {
                Some(i) => {
                    data.reserve_exact(total);
                    if pre > 0 {
                        let head = &rest[..i];
                        let take = head.len().min(pre);
                        let over = (ring.len() + take).saturating_sub(pre);
                        ring.drain(..over);
                        ring.extend(&head[head.len() - take..]);
                    }
                    data.extend(ring.drain(..));
                    triggered = Some((!trig.is_immediate()).then_some(data.len() as u64));
                    rest = &rest[i..];
                }
                None => {
                    // Keep the newest `pre` samples.
                    if pre > 0 {
                        let take = rest.len().min(pre);
                        let over = (ring.len() + take).saturating_sub(pre);
                        ring.drain(..over);
                        ring.extend(&rest[rest.len() - take..]);
                    }
                    progress(Progress { phase: Phase::Waiting, samples: ring.len() as u64, total: settings.samples });
                    if !more {
                        return Err(LogicError::Config("the source ended before the trigger fired".into()));
                    }
                    continue;
                }
            }
        }
        let room = total - data.len();
        data.extend_from_slice(&rest[..rest.len().min(room)]);
        progress(Progress { phase: Phase::Recording, samples: data.len() as u64, total: settings.samples });
        if data.len() >= total || !more {
            return Ok(finish(data, triggered.flatten()));
        }
    }
}

/// A stream over a fixed buffer, in chunks (tests and file playback).
pub struct BufferStream {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
    rate: u64,
    looped: bool,
}

impl BufferStream {
    pub fn new(data: Vec<u8>, rate: u64, chunk: usize, looped: bool) -> Self {
        Self { data, pos: 0, chunk: chunk.max(1), rate, looped }
    }
}

impl SampleStream for BufferStream {
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        if self.data.is_empty() {
            return Ok(false);
        }
        let mut need = self.chunk;
        while need > 0 {
            if self.pos >= self.data.len() {
                if !self.looped {
                    return Ok(false);
                }
                self.pos = 0;
            }
            let n = need.min(self.data.len() - self.pos);
            buf.extend_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            need -= n;
        }
        Ok(self.looped || self.pos < self.data.len())
    }

    fn sample_rate(&self) -> u64 {
        self.rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::default_channels;
    use crate::trigger::Edge;

    fn settings(samples: u64, trigger: Trigger, pre: f64) -> CaptureSettings {
        CaptureSettings { sample_rate: 1000, samples, trigger, pre_trigger: pre }
    }

    #[test]
    fn immediate_capture_has_exact_length() {
        let mut s = BufferStream::new((0..=255).collect(), 1000, 100, true);
        let stop = AtomicBool::new(false);
        let c = run(&mut s, &settings(1000, Trigger::None, 0.0), default_channels(8), &stop, &mut |_| {}).unwrap();
        assert_eq!(c.len(), 1000);
        assert_eq!(c.trigger, None);
        assert_eq!(c.sample(256), 0);
    }

    #[test]
    fn pre_trigger_ring_and_trigger_position() {
        // D0 rises at sample 5000.
        let data: Vec<u8> = (0..20_000).map(|i| u8::from(i >= 5000)).collect();
        let mut s = BufferStream::new(data, 1000, 333, false);
        let stop = AtomicBool::new(false);
        let mut phases = vec![];
        let c = run(&mut s, &settings(4000, Trigger::Edge { channel: 0, edge: Edge::Rising }, 0.25), default_channels(8), &stop, &mut |p| phases.push(p.phase)).unwrap();
        assert_eq!(c.len(), 4000);
        assert_eq!(c.trigger, Some(1000));
        assert!(!c.level(0, 999));
        assert!(c.level(0, 1000));
        assert!(phases.contains(&Phase::Waiting) && phases.contains(&Phase::Recording));
    }

    #[test]
    fn trigger_early_keeps_short_pre_trigger() {
        let data: Vec<u8> = (0..1000).map(|i| u8::from(i >= 10)).collect();
        let mut s = BufferStream::new(data, 1000, 64, false);
        let c = run(&mut s, &settings(500, Trigger::Edge { channel: 0, edge: Edge::Rising }, 0.5), default_channels(8), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(c.trigger, Some(10));
        assert_eq!(c.len(), 500);
    }

    #[test]
    fn stop_before_trigger_returns_ring() {
        let mut s = BufferStream::new(vec![0; 100], 1000, 50, true);
        let stop = AtomicBool::new(false);
        let mut n = 0;
        let c = run(&mut s, &settings(1000, Trigger::Edge { channel: 0, edge: Edge::Rising }, 0.2), default_channels(8), &stop, &mut |_| {
            n += 1;
            if n == 10 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(c.trigger, None);
        assert_eq!(c.len(), 200);
    }

    #[test]
    fn stop_while_recording_without_trigger_keeps_samples() {
        let mut s = BufferStream::new(vec![1; 100], 1000, 50, true);
        let stop = AtomicBool::new(false);
        let mut n = 0;
        let c = run(&mut s, &settings(1_000_000, Trigger::None, 0.0), default_channels(8), &stop, &mut |_| {
            n += 1;
            if n == 3 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(c.len(), 150);
        assert_eq!(c.trigger, None);
    }

    #[test]
    fn rejects_bad_settings() {
        assert!(settings(0, Trigger::None, 0.0).validate().is_err());
        assert!(settings(MAX_SAMPLES + 1, Trigger::None, 0.0).validate().is_err());
        assert!(settings(10, Trigger::None, 0.95).validate().is_err());
    }
}
