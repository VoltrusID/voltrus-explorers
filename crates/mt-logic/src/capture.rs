//! A capture: up to 8 logic channels packed one byte per sample (bit `n` is channel `n`), the
//! sample rate, and where the trigger fired.
//!
//! A summary pyramid (per block, the AND and the OR of every sample in it) makes the two questions
//! the app and the decoders keep asking cheap on hundreds of millions of samples: "what does this
//! pixel column look like" ([`Capture::summary`]) and "where is the next edge on this channel"
//! ([`Capture::next_edge`]).

use serde::{Deserialize, Serialize};

/// Channels per sample byte.
pub const MAX_CHANNELS: usize = 8;

/// Block sizes of the summary levels above the raw samples (each 16 times the one below).
const LEVEL0: u64 = 64;
const FANOUT: u64 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub name: String,
    /// CSS colour chosen in the app; `None` uses the default palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl ChannelInfo {
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into(), color: None }
    }
}

pub fn default_channels(n: usize) -> Vec<ChannelInfo> {
    (0..n).map(|i| ChannelInfo::named(format!("D{i}"))).collect()
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub sample_rate: u64,
    pub channels: Vec<ChannelInfo>,
    /// Sample index where the trigger condition matched, if the capture was triggered.
    pub trigger: Option<u64>,
    data: Vec<u8>,
    /// `levels[k]` holds `(and, or)` per block of `LEVEL0 * FANOUT^k` samples.
    levels: Vec<Vec<(u8, u8)>>,
}

/// One summary bucket per channel: every sample low, every sample high, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Low = 0,
    High = 1,
    Mixed = 2,
}

impl Capture {
    pub fn new(sample_rate: u64, channels: Vec<ChannelInfo>, data: Vec<u8>) -> Self {
        let mut c = Self { sample_rate: sample_rate.max(1), channels, trigger: None, data, levels: Vec::new() };
        c.build_levels();
        c
    }

    pub fn with_trigger(mut self, trigger: Option<u64>) -> Self {
        self.trigger = trigger;
        self
    }

    fn build_levels(&mut self) {
        self.levels.clear();
        let mut prev: Vec<(u8, u8)> = self
            .data
            .chunks(LEVEL0 as usize)
            .map(|c| c.iter().fold((0xFF, 0x00), |(a, o), &s| (a & s, o | s)))
            .collect();
        loop {
            let done = prev.len() <= 1;
            let next: Vec<(u8, u8)> = prev
                .chunks(FANOUT as usize)
                .map(|c| c.iter().fold((0xFF, 0x00), |(a, o), &(ba, bo)| (a & ba, o | bo)))
                .collect();
            self.levels.push(prev);
            if done {
                break;
            }
            prev = next;
        }
    }

    pub fn len(&self) -> u64 {
        self.data.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Duration of one sample in seconds.
    pub fn period(&self) -> f64 {
        1.0 / self.sample_rate as f64
    }

    pub fn time_of(&self, sample: u64) -> f64 {
        sample as f64 / self.sample_rate as f64
    }

    pub fn sample(&self, i: u64) -> u8 {
        self.data.get(i as usize).copied().unwrap_or_else(|| self.data.last().copied().unwrap_or(0))
    }

    pub fn level(&self, ch: u8, i: u64) -> bool {
        self.sample(i) >> ch & 1 == 1
    }

    fn block_size(level: usize) -> u64 {
        LEVEL0 * FANOUT.pow(level as u32)
    }

    /// AND and OR of every sample in `[a, b)`.
    pub fn and_or(&self, mut a: u64, b: u64) -> (u8, u8) {
        let b = b.min(self.len());
        let (mut and, mut or) = (0xFFu8, 0u8);
        while a < b {
            // The largest summary block that starts at `a` and fits in the range.
            let mut step = None;
            for (lv, blocks) in self.levels.iter().enumerate().rev() {
                let size = Self::block_size(lv);
                if a.is_multiple_of(size) && a + size <= b {
                    let (ba, bo) = blocks[(a / size) as usize];
                    and &= ba;
                    or |= bo;
                    step = Some(size);
                    break;
                }
            }
            match step {
                Some(s) => a += s,
                None => {
                    let s = self.data[a as usize];
                    and &= s;
                    or |= s;
                    a += 1;
                }
            }
        }
        (and, or)
    }

    /// `buckets` equal slices of `[start, end)`, per channel, flattened channel-major
    /// (`out[ch * buckets + i]`). When a bucket is narrower than a sample it shows that sample.
    pub fn summary(&self, start: u64, end: u64, buckets: usize, channels: usize) -> Vec<u8> {
        let end = end.min(self.len());
        let mut out = vec![0u8; buckets * channels];
        if buckets == 0 || start >= end {
            return out;
        }
        let span = (end - start) as f64;
        for i in 0..buckets {
            let a = start + (span * i as f64 / buckets as f64).floor() as u64;
            let mut b = start + (span * (i + 1) as f64 / buckets as f64).floor() as u64;
            if b <= a {
                b = a + 1;
            }
            let (and, or) = self.and_or(a, b.min(end).max(a + 1));
            for ch in 0..channels {
                let bit = 1u8 << ch;
                out[ch * buckets + i] = if and & bit != 0 {
                    Bucket::High as u8
                } else if or & bit == 0 {
                    Bucket::Low as u8
                } else {
                    Bucket::Mixed as u8
                };
            }
        }
        out
    }

    /// The first sample index `> from` whose level on `ch` differs from the level at `from`.
    pub fn next_edge(&self, ch: u8, from: u64) -> Option<u64> {
        let n = self.len();
        if from + 1 >= n {
            return None;
        }
        let bit = 1u8 << ch;
        let lvl = self.data[from as usize] & bit != 0;
        let same = |(a, o): (u8, u8)| if lvl { a & bit != 0 } else { o & bit == 0 };
        let mut i = from + 1;
        while i < n {
            // Skip whole blocks where the channel stays at `lvl`.
            let mut skipped = false;
            for (lv, blocks) in self.levels.iter().enumerate().rev() {
                let size = Self::block_size(lv);
                if i.is_multiple_of(size) {
                    let idx = (i / size) as usize;
                    if idx < blocks.len() && i + size <= n && same(blocks[idx]) {
                        i += size;
                        skipped = true;
                        break;
                    }
                }
            }
            if skipped {
                continue;
            }
            if (self.data[i as usize] & bit != 0) != lvl {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// The last sample index `< from` where `ch` changed level (the index of the new level).
    pub fn prev_edge(&self, ch: u8, from: u64) -> Option<u64> {
        let bit = 1u8 << ch;
        let from = from.min(self.len().saturating_sub(1));
        let lvl = self.data.get(from as usize)? & bit != 0;
        let mut i = from;
        while i > 0 {
            if (self.data[i as usize - 1] & bit != 0) != lvl {
                return Some(i);
            }
            i -= 1;
        }
        None
    }

    /// Every edge of `ch` in `[start, end)` as `(sample, new level)`.
    pub fn edges(&self, ch: u8, start: u64, end: u64) -> Edges<'_> {
        Edges { cap: self, ch, pos: start, end: end.min(self.len()) }
    }

    /// Frequency and duty cycle of `ch` over `[start, end)` from its full periods (rising edge to rising edge).
    pub fn measure(&self, ch: u8, start: u64, end: u64) -> Option<Measurement> {
        let rising: Vec<u64> = self.edges(ch, start, end).filter(|e| e.1).map(|e| e.0).collect();
        if rising.len() < 2 {
            return None;
        }
        let periods = rising.len() - 1;
        let span = rising[periods] - rising[0];
        let mut high = 0u64;
        for w in rising.windows(2) {
            let fall = self.next_edge(ch, w[0]).unwrap_or(w[1]).min(w[1]);
            high += fall - w[0];
        }
        let period_s = span as f64 / periods as f64 / self.sample_rate as f64;
        Some(Measurement { frequency: 1.0 / period_s, period: period_s, duty: high as f64 / span as f64, periods: periods as u64 })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub frequency: f64,
    pub period: f64,
    pub duty: f64,
    pub periods: u64,
}

pub struct Edges<'a> {
    cap: &'a Capture,
    ch: u8,
    pos: u64,
    end: u64,
}

impl Iterator for Edges<'_> {
    type Item = (u64, bool);

    fn next(&mut self) -> Option<(u64, bool)> {
        if self.pos >= self.end {
            return None;
        }
        let e = self.cap.next_edge(self.ch, self.pos)?;
        if e >= self.end {
            self.pos = self.end;
            return None;
        }
        self.pos = e;
        Some((e, self.cap.level(self.ch, e)))
    }
}

/// Formats a duration in seconds with an SI prefix: `1.25 ms`, `417 ns`.
pub fn fmt_time(s: f64) -> String {
    let a = s.abs();
    let (v, unit) = if a == 0.0 {
        (0.0, "s")
    } else if a < 1e-6 {
        (s * 1e9, "ns")
    } else if a < 1e-3 {
        (s * 1e6, "µs")
    } else if a < 1.0 {
        (s * 1e3, "ms")
    } else {
        (s, "s")
    };
    let txt = if v.abs() >= 100.0 { format!("{v:.0}") } else if v.abs() >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    format!("{txt} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(data: Vec<u8>) -> Capture {
        Capture::new(1_000_000, default_channels(8), data)
    }

    #[test]
    fn edges_across_summary_blocks() {
        let mut d = vec![0u8; 100_000];
        for s in d.iter_mut().skip(70_000) {
            *s = 0b0000_0010;
        }
        d[5] = 1;
        let c = cap(d);
        assert_eq!(c.next_edge(1, 0), Some(70_000));
        assert_eq!(c.next_edge(0, 0), Some(5));
        assert_eq!(c.next_edge(0, 5), Some(6));
        assert_eq!(c.next_edge(0, 6), None);
        assert_eq!(c.prev_edge(1, 99_999), Some(70_000));
        let e: Vec<_> = c.edges(0, 0, 100_000).collect();
        assert_eq!(e, vec![(5, true), (6, false)]);
    }

    #[test]
    fn and_or_matches_brute_force() {
        let d: Vec<u8> = (0..20_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8 & if i % 977 < 400 { 0xFF } else { 0x0F }).collect();
        let c = cap(d.clone());
        for (a, b) in [(0, 20_000), (63, 1025), (1000, 1001), (4096, 8193), (17, 19_999)] {
            let want = d[a..b].iter().fold((0xFF, 0), |(x, y), &s| (x & s, y | s));
            assert_eq!(c.and_or(a as u64, b as u64), want, "{a}..{b}");
        }
    }

    #[test]
    fn summary_marks_mixed_buckets() {
        let mut d = vec![0u8; 1000];
        for s in d.iter_mut().skip(500) {
            *s = 1;
        }
        let c = cap(d);
        let s = c.summary(0, 1000, 4, 1);
        assert_eq!(s, vec![0, 0, 1, 1]);
        let s = c.summary(0, 1000, 3, 1);
        assert_eq!(s, vec![0, 2, 1]);
        // Narrower than a sample: shows the sample.
        let s = c.summary(499, 501, 8, 1);
        assert_eq!(s, vec![0, 0, 0, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn measures_frequency_and_duty() {
        // 10 kHz, 25 % duty at 1 MS/s: 25 samples high, 75 low.
        let d: Vec<u8> = (0..10_000).map(|i| u8::from(i % 100 < 25)).collect();
        let m = cap(d).measure(0, 0, 10_000).unwrap();
        assert!((m.frequency - 10_000.0).abs() < 1e-6);
        assert!((m.duty - 0.25).abs() < 1e-9);
    }

    #[test]
    fn formats_times() {
        assert_eq!(fmt_time(0.00125), "1.25 ms");
        assert_eq!(fmt_time(417e-9), "417 ns");
        assert_eq!(fmt_time(2.0), "2.00 s");
    }
}
