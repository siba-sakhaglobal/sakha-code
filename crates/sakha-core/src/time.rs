//! Time helpers shared across crates: UTC timestamps and monotonic wall-clock
//! elapsed measurement for budgets/loops.

use chrono::{DateTime, Utc};
use std::time::Instant;

/// Returns the current UTC timestamp. Centralized so tests/mocks can wrap it
/// later without touching call sites.
pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// A monotonic stopwatch used for wall-clock budget accounting. Wraps
/// `std::time::Instant` since `DateTime<Utc>` is not guaranteed monotonic.
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch {
    start: Instant,
}

impl Stopwatch {
    pub fn start() -> Self {
        Self { start: Instant::now() }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn now_utc_is_monotonically_non_decreasing_across_calls() {
        let a = now_utc();
        let b = now_utc();
        assert!(b >= a);
    }

    #[test]
    fn stopwatch_measures_elapsed_time() {
        let sw = Stopwatch::start();
        sleep(Duration::from_millis(5));
        assert!(sw.elapsed() >= Duration::from_millis(5));
    }
}
