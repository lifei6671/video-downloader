//! One cancellable connection probe loop per Telegram account.
use anyhow::Result;
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::{watch, Mutex};

pub(crate) fn network_error(reason: &str) -> bool {
    matches!(
        reason,
        "TELEGRAM_CONNECTION_ERROR"
            | "TELEGRAM_TIMEOUT"
            | "TELEGRAM_NOT_CONNECTED"
            | "TELEGRAM_TRANSFER_TIMEOUT"
            | "BACKUP_NETWORK_ERROR"
    )
}

#[derive(Default)]
pub(crate) struct Reconnect {
    owner: Mutex<()>,
    // Low bit is the disconnected latch; upper bits identify a successful probe.
    state: AtomicU64,
}
impl Reconnect {
    pub(crate) fn disconnected(&self) {
        self.state.fetch_or(1, Ordering::SeqCst);
    }
    pub(crate) fn waiting(&self) -> bool {
        self.state.load(Ordering::SeqCst) & 1 != 0
    }
    pub(crate) fn active(&self) -> bool {
        self.waiting() && self.owner.try_lock().is_err()
    }
    pub(crate) async fn run<F, Fut>(
        &self,
        mut cancel: watch::Receiver<bool>,
        mut probe: F,
    ) -> Result<()>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut generation = self.state.fetch_or(1, Ordering::SeqCst) & !1;
        let work = async {
            let _owner = self.owner.lock().await;
            let current = self.state.load(Ordering::SeqCst);
            if current & !1 != generation {
                if current & 1 == 0 {
                    return Ok(());
                }
                // Another outage started after the result this waiter could reuse.
                // Serve that generation before admitting any business request.
                generation = current & !1;
            }
            let mut failures = 0u32;
            loop {
                // Delay before the first probe too: a healthy control socket must not
                // turn a broken media connection into a tight retry loop.
                tokio::time::sleep(Duration::from_secs((1u64 << failures.min(4)).min(15))).await;
                match Box::pin(probe()).await {
                    Ok(()) => {
                        self.state
                            .store(generation.wrapping_add(2), Ordering::SeqCst);
                        return Ok(());
                    }
                    Err(error)
                        if network_error(&error.to_string())
                            || matches!(
                                error.to_string().as_str(),
                                "TELEGRAM_NETWORK_CHANGED" | "TELEGRAM_BUSY"
                            ) =>
                    {
                        failures = failures.saturating_add(1);
                    }
                    Err(error) => {
                        self.state.fetch_and(!1, Ordering::SeqCst);
                        return Err(error);
                    }
                }
            }
        };
        tokio::select! { biased;
            // Cancellation drops this owner only. Retain the disconnected latch
            // so another waiting/new task probes before issuing business requests.
            _ = cancel.wait_for(|v| *v) => anyhow::bail!("CANCELED"),
            result = Box::pin(work) => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc};

    #[tokio::test(start_paused = true)]
    async fn reconnect_survives_long_outage_and_shares_probe() {
        let reconnect = Arc::new(Reconnect::default());
        let attempts = Arc::new(AtomicUsize::new(0));
        let (stop, cancel) = watch::channel(false);
        let mut workers = Vec::new();
        for _ in 0..8 {
            let (reconnect, attempts, cancel) =
                (reconnect.clone(), attempts.clone(), cancel.clone());
            workers.push(tokio::spawn(async move {
                reconnect
                    .run(cancel, || {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        async move {
                            if attempt < 12 {
                                anyhow::bail!("TELEGRAM_CONNECTION_ERROR");
                            }
                            Ok(())
                        }
                    })
                    .await
            }));
        }
        for worker in workers {
            worker.await.unwrap().unwrap();
        }
        assert_eq!(attempts.load(Ordering::SeqCst), 13);
        assert!(!reconnect.waiting());
        drop(stop);
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_cancel_releases_owner_and_other_waiter_continues() {
        let reconnect = Arc::new(Reconnect::default());
        let (stop, cancel) = watch::channel(false);
        let first_reconnect = reconnect.clone();
        let first = tokio::spawn(async move {
            first_reconnect
                .run(cancel, || async {
                    std::future::pending::<Result<()>>().await
                })
                .await
        });
        tokio::time::advance(Duration::from_secs(2)).await;
        stop.send_replace(true);
        assert_eq!(first.await.unwrap().unwrap_err().to_string(), "CANCELED");
        let (_stop, cancel) = watch::channel(false);
        reconnect.run(cancel, || async { Ok(()) }).await.unwrap();
        assert!(!reconnect.waiting());
    }

    #[tokio::test(start_paused = true)]
    async fn old_waiter_cannot_clear_or_bypass_new_outage() {
        let reconnect = Arc::new(Reconnect::default());
        let available = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owner = reconnect.owner.lock().await;
        let (_old_stop, old_cancel) = watch::channel(false);
        let old_reconnect = reconnect.clone();
        let old_available = available.clone();
        let old = tokio::spawn(async move {
            old_reconnect
                .run(old_cancel, || async {
                    if !old_available.load(Ordering::SeqCst) {
                        anyhow::bail!("TELEGRAM_CONNECTION_ERROR");
                    }
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;
        // The preceding owner has just established generation 1.
        reconnect.state.store(2, Ordering::SeqCst);
        let (_stop, cancel) = watch::channel(false);
        let new_reconnect = reconnect.clone();
        let new_available = available.clone();
        let new = tokio::spawn(async move {
            new_reconnect
                .run(cancel, || async {
                    if !new_available.load(Ordering::SeqCst) {
                        anyhow::bail!("TELEGRAM_CONNECTION_ERROR");
                    }
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;
        drop(owner);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(reconnect.waiting());
        assert!(
            !old.is_finished(),
            "old waiter must not admit work during the new outage"
        );
        assert!(!new.is_finished());
        available.store(true, Ordering::SeqCst);
        old.await.unwrap().unwrap();
        new.await.unwrap().unwrap();
        assert!(!reconnect.waiting());
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_stops_on_auth_and_business_errors() {
        for reason in [
            "TELEGRAM_AUTH_REQUIRED",
            "TELEGRAM_ACCOUNT_MISMATCH",
            "TELEGRAM_CHANNEL_FORBIDDEN",
        ] {
            let reconnect = Reconnect::default();
            let (_stop, cancel) = watch::channel(false);
            let attempts = AtomicUsize::new(0);
            let error = reconnect
                .run(cancel, || {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    async { Err(anyhow::anyhow!(reason)) }
                })
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), reason);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert!(!reconnect.waiting());
        }
    }
}
