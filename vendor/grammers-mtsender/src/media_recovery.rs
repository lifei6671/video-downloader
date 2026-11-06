//! Demand-driven recovery of a file-only DC route. Never retries a file RPC itself.
use std::time::Duration;
use tokio::time::Instant;

pub(super) struct Recovery {
    pub attempts: u8,
    pub next_probe: Instant,
    pub probing: Option<u64>,
}
impl Recovery {
    pub fn new() -> Self {
        Self { attempts: 0, next_probe: Instant::now(), probing: None }
    }
    pub fn ready(&self) -> bool {
        self.probing.is_none() && Instant::now() >= self.next_probe
    }
    pub fn begin(&mut self, connection: u64) {
        self.attempts += 1;
        self.probing = Some(connection);
    }
    pub fn defer(&mut self) -> Duration {
        self.probing = None;
        let delay = Duration::from_secs(match self.attempts { 0 => 10, 1 => 30, 2 => 60, _ => 300 });
        if self.attempts >= 3 { self.attempts = 0; }
        self.next_probe = Instant::now() + delay;
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn retries_are_bounded_per_cycle_and_a_later_cycle_can_recover() {
        let mut recovery = Recovery::new();
        assert_eq!(recovery.defer().as_secs(), 10);
        assert!(!recovery.ready());
        for (attempt, delay) in [(1, 30), (2, 60), (3, 300)] {
            tokio::time::advance(recovery.next_probe.saturating_duration_since(Instant::now())).await;
            assert!(recovery.ready());
            recovery.begin(attempt);
            assert!(!recovery.ready(), "only one probe may run");
            assert_eq!(recovery.defer().as_secs(), delay);
        }
        assert_eq!(recovery.attempts, 0);
        tokio::time::advance(Duration::from_secs(300)).await;
        assert!(recovery.ready(), "a failed cycle cannot disable a DC forever");
        recovery.begin(4);
        assert_eq!(recovery.attempts, 1);
    }
}
