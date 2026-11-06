//! Per-session occupancy totals. A waiting deletion takes two snapshots so that handoffs
//! between queued requests and cancellation are attributed without retaining request history.
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

type Totals = BTreeMap<&'static str, Duration>;

#[derive(Default)]
struct Occupancy {
    current: Option<(&'static str, Instant)>,
    completed: Totals,
}

#[derive(Default)]
pub(super) struct LockUsage(Mutex<Occupancy>);

impl LockUsage {
    // Called with the actual session mutex held. The returned guard drops before that mutex.
    pub(super) fn hold(&self, kind: &'static str) -> Holder<'_> {
        self.0.lock().unwrap().current = Some((kind, Instant::now()));
        Holder { usage: self }
    }

    fn snapshot(&self) -> Totals {
        let occupancy = self.0.lock().unwrap();
        let mut totals = occupancy.completed.clone();
        if let Some((kind, since)) = occupancy.current {
            *totals.entry(kind).or_default() += since.elapsed();
        }
        totals
    }

    pub(super) fn observe_wait(&self) -> Wait<'_> {
        Wait {
            usage: self,
            before: self.snapshot(),
        }
    }
}

pub(super) struct Holder<'a> {
    usage: &'a LockUsage,
}
impl Drop for Holder<'_> {
    fn drop(&mut self) {
        let mut occupancy = self.usage.0.lock().unwrap();
        if let Some((kind, since)) = occupancy.current.take() {
            *occupancy.completed.entry(kind).or_default() += since.elapsed();
        }
    }
}

pub(super) struct Wait<'a> {
    usage: &'a LockUsage,
    before: Totals,
}
impl Drop for Wait<'_> {
    fn drop(&mut self) {
        for (kind, total) in self.usage.snapshot() {
            let overlap = total - self.before.get(kind).copied().unwrap_or_default();
            if !overlap.is_zero() {
                crate::backup::delete_timing::record_lock_holder(kind, overlap);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waiter_counts_only_overlapping_parts_of_each_holder() {
        let usage = LockUsage::default();
        let holder = usage.hold("pull_changes");
        let baseline = usage.snapshot();
        // Advance occupied time deterministically while keeping real drop/handoff behavior.
        usage.0.lock().unwrap().current.as_mut().unwrap().1 -= Duration::from_secs(2);
        drop(holder);
        let next = usage.hold("read_asset");
        usage.0.lock().unwrap().current.as_mut().unwrap().1 -= Duration::from_secs(3);
        let after = usage.snapshot();
        assert!((after["pull_changes"] - baseline["pull_changes"]) >= Duration::from_secs(2));
        assert!(after["read_asset"] >= Duration::from_secs(3));
        drop(next);
        assert!(usage.0.lock().unwrap().current.is_none());
    }
}
