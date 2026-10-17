//! Per-peer read-only probe pacing. Never used to retry service operations.
use std::time::Duration;

#[derive(Default)]
pub struct Retry {
    failures: u32,
}
impl Retry {
    pub fn reset(&mut self) {
        self.failures = 0;
    }
    pub fn completed(&mut self, success: bool) -> Duration {
        if success {
            self.reset();
            return Duration::from_secs(1);
        }
        self.failures = self.failures.saturating_add(1);
        Duration::from_secs(1u64 << self.failures.saturating_sub(1).min(5))
            .min(Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_are_capped_independent_and_reset_by_recovery_or_notification() {
        let mut offline = Retry::default();
        let mut healthy = Retry::default();
        for seconds in [1, 2, 4, 8, 16, 30, 30, 30] {
            assert_eq!(offline.completed(false), Duration::from_secs(seconds));
            assert_eq!(healthy.completed(true), Duration::from_secs(1));
        }
        assert_eq!(offline.completed(true), Duration::from_secs(1));
        assert_eq!(offline.completed(false), Duration::from_secs(1));
        assert_eq!(offline.completed(false), Duration::from_secs(2));
        offline.reset();
        assert_eq!(offline.completed(false), Duration::from_secs(1));
        offline.failures = u32::MAX;
        assert_eq!(offline.completed(false), Duration::from_secs(30));
    }
}
