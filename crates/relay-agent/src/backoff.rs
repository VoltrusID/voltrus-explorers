//! Exponential backoff with jitter for reconnect attempts.

use std::time::Duration;

/// `base * 2^attempt`, capped at `max`, plus up to 25% jitter.
#[derive(Debug)]
pub struct Backoff {
    attempt: u32,
    base: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Backoff {
            attempt: 0,
            base,
            max,
        }
    }

    /// Advance the sequence and return the next delay.
    pub fn next_delay(&mut self) -> Duration {
        let mut delay = self.base;
        for _ in 0..self.attempt {
            delay = delay.saturating_mul(2);
            if delay >= self.max {
                break;
            }
        }
        self.attempt = self.attempt.saturating_add(1);
        delay.min(self.max) + jitter(delay.min(self.max) / 4)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Cheap, dependency-free jitter: sub-millisecond system clock entropy folded
/// into `0..=span`. Not cryptographic — reconnect jitter does not need to be.
fn jitter(span: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let bound = span.as_nanos().max(1) as u64;
    Duration::from_nanos(nanos % (bound + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_grows_then_caps_with_bounded_jitter() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(2));
        let mut delays = Vec::new();
        for _ in 0..8 {
            delays.push(backoff.next_delay());
        }
        // 100ms, 200ms, 400ms, 800ms, 1.6s (each + <=25% jitter), then capped at 2s.
        let first = delays[0];
        let fifth = delays[4];
        assert!(first < Duration::from_millis(200), "{first:?}");
        assert!(fifth >= Duration::from_millis(1600), "{fifth:?}");
        for delay in &delays[5..] {
            assert!(
                (Duration::from_secs(2)..=Duration::from_millis(2500)).contains(delay),
                "{delay:?}"
            );
        }
    }

    #[test]
    fn reset_restarts_the_sequence() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
        for _ in 0..6 {
            backoff.next_delay();
        }
        backoff.reset();
        assert!(backoff.next_delay() < Duration::from_millis(200));
    }
}
