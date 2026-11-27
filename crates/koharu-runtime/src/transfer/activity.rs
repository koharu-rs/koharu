use std::time::{Duration, Instant};

pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const EWMA_TAU_SECS: f64 = 2.0;
const MIN_SAMPLE_SECS: f64 = 0.1;

/// Tracks smoothed download throughput (bytes/sec) using an exponentially
/// weighted moving average and throttles progress emission.
#[derive(Debug)]
pub(crate) struct SpeedTracker {
    last_sample_at: Instant,
    last_completed: u64,
    rate: f64,
    has_sample: bool,
    last_emit: Instant,
}

impl SpeedTracker {
    pub(crate) fn new(initial_completed: u64) -> Self {
        let now = Instant::now();
        Self::with_now(initial_completed, now)
    }

    pub(crate) fn with_now(initial_completed: u64, now: Instant) -> Self {
        Self {
            last_sample_at: now,
            last_completed: initial_completed,
            rate: 0.0,
            has_sample: false,
            last_emit: now,
        }
    }

    /// Updates the EWMA rate when at least `MIN_SAMPLE_SECS` has elapsed and
    /// returns `(bytes_per_sec, should_emit)`.
    pub(crate) fn record(&mut self, completed: u64, now: Instant) -> (u64, bool) {
        let dt = now
            .saturating_duration_since(self.last_sample_at)
            .as_secs_f64();
        if dt >= MIN_SAMPLE_SECS {
            let delta = completed.saturating_sub(self.last_completed) as f64;
            let instant = delta / dt;
            let alpha = 1.0 - (-dt / EWMA_TAU_SECS).exp();
            self.rate = if self.has_sample {
                alpha * instant + (1.0 - alpha) * self.rate
            } else {
                self.has_sample = true;
                instant
            };
            self.last_sample_at = now;
            self.last_completed = completed;
        }

        let should_emit = now.saturating_duration_since(self.last_emit) >= PROGRESS_INTERVAL;
        if should_emit {
            self.last_emit = now;
        }

        (self.rate.round().max(0.0) as u64, should_emit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewma_handles_zero_dt_and_smooths_rate() {
        let t0 = Instant::now();
        let mut tracker = SpeedTracker::with_now(0, t0);

        // 0ms elapsed: must not divide by zero or emit before interval.
        let (rate, emit) = tracker.record(1024, t0);
        assert_eq!(rate, 0);
        assert!(!emit);

        // +100ms with 1000 bytes -> 10_000 B/s initial sample and emits.
        let t1 = t0 + Duration::from_millis(100);
        let (rate, emit) = tracker.record(1000, t1);
        assert_eq!(rate, 10_000);
        assert!(emit);

        // +2s with 4000 additional bytes -> instant 2_000 B/s blended with 10_000 B/s.
        let t2 = t1 + Duration::from_secs(2);
        let (rate, emit) = tracker.record(5000, t2);
        assert!(emit);
        assert!(rate > 2_000 && rate < 10_000, "unexpected rate: {rate}");
    }
}
