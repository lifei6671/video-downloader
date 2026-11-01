//! Last-attempt deletion timing is stored with the existing durable operation.
use crate::desktop_core::{Core, DeleteActivity};
use std::sync::Arc;
use anyhow::Result;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    future::Future,
    time::{Duration, Instant},
};

#[derive(Default)]
struct TimingDetails {
    owner: String,
    setup: bool,
    full_scan_reason: Option<&'static str>,
    difference_reason: Option<&'static str>,
    reused_index_bodies: usize,
    lock_holders: BTreeMap<&'static str, Duration>,
    steps: BTreeMap<&'static str, (Duration, u32)>,
}

tokio::task_local! {
    // One refresh can span multiple batches, documents and file-reference retries.
    static DETAILS: RefCell<TimingDetails>;
    static ACTIVE_DELETE: (Arc<Core>, String);
    static COMPANION: Option<String>;
}

pub(super) async fn track_delete<F: Future>(core: &Arc<Core>, operation: &str, work: F) -> F::Output {
    ACTIVE_DELETE.scope((core.clone(),operation.to_owned()), work).await
}

pub(super) async fn with_companion<F: Future>(operation: Option<&str>, work: F) -> F::Output {
    COMPANION.scope(operation.map(str::to_owned),work).await
}

// Nested work restores its parent when it returns, fails, or is canceled.
pub(super) struct ActivitySpan {
    core: Arc<Core>,
    previous: Option<DeleteActivity>,
}

pub(super) fn activity(step: &str) -> Option<ActivitySpan> {
    ACTIVE_DELETE.try_with(|(core, operation)| {
        let previous = core.replace_delete_activity(Some(DeleteActivity {
            operation_id: operation.clone(),
            companion: COMPANION.try_with(Clone::clone).ok().flatten(),
            step: step.to_owned(),
        }));
        ActivitySpan {core: core.clone(), previous}
    }).ok()
}

impl Drop for ActivitySpan {
    fn drop(&mut self) {
        self.core.replace_delete_activity(self.previous.take());
    }
}

pub(super) fn selected_full_scan(reason: &'static str, difference_reason: Option<&'static str>) {
    let _ = DETAILS.try_with(|details| {
        let mut details = details.borrow_mut();
        details.full_scan_reason = Some(reason);
        details.difference_reason = difference_reason;
    });
}

pub(super) fn record_index_reuse(count: usize) {
    let _ = DETAILS.try_with(|details| details.borrow_mut().reused_index_bodies = count);
}

pub(super) struct StepSpan {
    _activity: Option<ActivitySpan>,
    step: &'static str,
    start: Instant,
}

pub(super) fn refresh_span(step: &'static str) -> Option<StepSpan> {
    step_span(step, false)
}

pub(super) fn record_lock_holder(kind: &'static str, elapsed: Duration) {
    let _ = DETAILS.try_with(|details| {
        *details.borrow_mut().lock_holders.entry(kind).or_default() += elapsed;
    });
}

pub(super) fn setup_span(step: &'static str) -> Option<StepSpan> {
    step_span(step, true)
}

fn step_span(step: &'static str, setup: bool) -> Option<StepSpan> {
    DETAILS
        .try_with(|details| {
            (details.borrow().setup == setup).then(|| StepSpan {
                _activity: activity(step),
                step,
                start: Instant::now(),
            })
        })
        .ok()
        .flatten()
}

impl Drop for StepSpan {
    fn drop(&mut self) {
        let _ = DETAILS.try_with(|details| {
            let mut details = details.borrow_mut();
            let (elapsed, calls) = details.steps.entry(self.step).or_default();
            *elapsed += self.start.elapsed();
            *calls += 1;
        });
    }
}

fn stage_details(phase: &str) -> Option<serde_json::Value> {
    DETAILS
        .try_with(|details| {
            let details = details.borrow();
            if details.owner != phase { return None; }
            let steps: BTreeMap<_, _> = details
                .steps
                .iter()
                .map(|(step, (duration, calls))| {
                    (
                        *step,
                        serde_json::json!({"elapsed_ms":duration.as_millis() as u64,"calls":calls}),
                    )
                })
                .collect();
            if details.setup {
                let holders: BTreeMap<_, _> = details.lock_holders.iter()
                    .filter(|(_, duration)| duration.as_millis() > 0)
                    .map(|(kind, duration)| (kind, duration.as_millis() as u64))
                    .collect();
                return (!steps.is_empty()).then(|| serde_json::json!({"steps":steps,"session_lock_holders_ms":holders}));
            }
            Some(serde_json::json!({"full_scan":details.full_scan_reason.is_some(),
            "full_scan_reason":details.full_scan_reason,"difference_reason":details.difference_reason,
            "reused_index_bodies":details.reused_index_bodies,"steps":steps}))
        })
        .ok().flatten()
}

struct Stage<'a> {
    core: &'a Core,
    operation: &'a str,
    phase: &'a str,
    bucket: Option<&'static str>,
    start: Instant,
    outcome: &'static str,
}
impl Drop for Stage<'_> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_millis().min(i64::MAX as u128) as i64;
        eprintln!(
            "backup_delete operation={} phase={} elapsed_ms={} outcome={}",
            self.operation, self.phase, elapsed, self.outcome
        );
        let Some(bucket) = self.bucket else {
            eprintln!("backup_delete timing persistence unavailable");
            return;
        };
        let extra = stage_details(self.phase)
            .map(|details| serde_json::json!({"details": details}).to_string());
        let result = self.core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET payload_json=json_set(payload_json,?2,
                 json_patch(json_object('elapsed_ms',?3,'outcome',?4,'at',unixepoch(),'attempt',attempt),COALESCE(?5,'{}')))
                 WHERE operation_id=?1",
                rusqlite::params![self.operation,format!("$.timings.{}.{}",bucket,self.phase),elapsed,self.outcome,extra],
            )?;
            Ok(())
        });
        if result.is_err() {
            eprintln!("backup_delete timing persistence failed");
        }
    }
}
pub(super) async fn timed<T>(
    core: &Core,
    operation: &str,
    phase: &str,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    let mut stage = Stage {
        core,
        operation,
        phase,
        bucket: core.backup_db(|db|Ok(db.query_row("SELECT COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0) FROM backup_operations WHERE operation_id=?",[operation],|row|row.get::<_,bool>(0))?)).ok().map(|done|if done {"maintenance"}else{"foreground"}),
        start: Instant::now(),
        outcome: "canceled",
    };
    let _activity = activity(phase);
    let result = work.await;
    stage.outcome = if result.is_ok() { "ok" } else { "error" };
    result
}

pub(super) async fn timed_setup<T>(
    core: &Core,
    operation: &str,
    phase: &str,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    let details = TimingDetails {
        owner: phase.into(),
        setup: true,
        ..Default::default()
    };
    DETAILS
        .scope(RefCell::new(details), timed(core, operation, phase, work))
        .await
}

/// Attribute index transfer work to the parent deletion, even when the archive has its own send ID.
#[derive(Clone, Copy, Default)]
pub(super) struct IndexTiming<'a>(Option<(&'a Core, &'a str, &'static str)>);

impl<'a> IndexTiming<'a> {
    pub(super) fn new(core: &'a Core, deletion: &'a str, stage: &'static str) -> Self {
        Self(Some((core, deletion, stage)))
    }

    pub(super) async fn refresh<T>(self, work: impl Future<Output = Result<T>>) -> Result<T> {
        if let Some((core, operation, stage)) = self.0 {
            let phase = format!("{stage}_refresh");
            let details = TimingDetails {
                owner: phase.clone(),
                ..Default::default()
            };
            DETAILS
                .scope(RefCell::new(details), timed(core, operation, &phase, work))
                .await
        } else {
            work.await
        }
    }

    pub(super) async fn run<T>(
        self,
        step: &'static str,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        match self.0 {
            Some((core, deletion, stage)) => {
                timed(core, deletion, &format!("{stage}_{step}"), work).await
            }
            None => work.await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn live_activity_tracks_batch_members_and_clears_on_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(&root.path().join("qa.sqlite"), root.path().join("downloads"), "no-python".into(), vec![], |_| {}).unwrap();
        let task = track_delete(&core, "second", async {
            let _publish = activity("remote_cleanup");
            with_companion(Some("first"), async {
                let _batch = activity("delete_message");
                let current = core.current_delete_activity().unwrap();
                assert_eq!(current.operation_id, "second");
                assert_eq!(current.companion.as_deref(), Some("first"));
                assert_eq!(current.step, "delete_message");
            }).await;
            let parent = core.current_delete_activity().unwrap();
            assert_eq!(parent.step, "remote_cleanup");
            assert!(parent.companion.is_none());
            std::future::pending::<()>().await
        });
        {
            tokio::pin!(task);
            tokio::select! {biased; _=&mut task=>unreachable!(), _=std::future::ready(())=>{}}
        }
        assert!(core.current_delete_activity().is_none());
        core.shutdown().await;
    }

    #[tokio::test]
    async fn timings_survive_restart_and_preserve_operation_payload_on_failure_and_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("qa.sqlite");
        let open = || {
            Core::open(
                &path,
                root.path().join("downloads"),
                "no-python".into(),
                vec![],
                |_| {},
            )
            .unwrap()
        };
        let core = open();
        core.backup_db(|db|{db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,enabled) VALUES('space','library','','','',0)",[])?;db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('timing','space','delete','running','objects','{\"remote_asset_id\":\"original\"}')",[])?;Ok(())}).unwrap();
        let error: Result<()> =
            timed(&core, "timing", "scan", async { anyhow::bail!("expected") }).await;
        assert!(error.is_err());
        {
            let future = IndexTiming::new(&core, "timing", "publish").refresh(async {
                for _ in 0..2 {
                    let _download = refresh_span("index_download");
                }
                let _batch = refresh_span("by_id");
                std::future::pending::<Result<()>>().await
            });
            tokio::pin!(future);
            tokio::select! {_= &mut future=>unreachable!(),_=tokio::time::sleep(std::time::Duration::from_millis(1))=>{}}
        }
        timed_setup(&core, "timing", "remote_cleanup", async {
            drop(setup_span("session_lock"));
            record_lock_holder("pull_changes", Duration::from_millis(23));
            record_lock_holder("pull_changes", Duration::from_millis(7));
            record_lock_holder("read_asset", Duration::from_millis(9));
            timed(&core, "timing", "archive", async { Ok(()) }).await?;
            IndexTiming::new(&core, "timing", "archive")
                .refresh(async {
                    drop(refresh_span("by_id"));
                    assert!(setup_span("channel_lookup").is_none());
                    Ok(())
                })
                .await?;
            assert!(refresh_span("index_download").is_none());
            drop(setup_span("channel_lookup"));
            Ok(())
        })
        .await
        .unwrap();
        // End the future's scope so its drop guard persists cancellation before shutdown.
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.local_cleanup_done',json('true')) WHERE operation_id='timing'",[])?;Ok(())}).unwrap();
        timed(&core, "timing", "scan", async { Ok(()) })
            .await
            .unwrap();
        core.shutdown().await;
        drop(core);
        let core = open();
        let raw: String = core
            .backup_db(|db| {
                Ok(db.query_row(
                    "SELECT payload_json FROM backup_operations WHERE operation_id='timing'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(payload["remote_asset_id"], "original");
        let foreground = &payload["timings"]["foreground"];
        let setup = &foreground["remote_cleanup"]["details"]["steps"];
        let holders = &foreground["remote_cleanup"]["details"]["session_lock_holders_ms"];
        assert_eq!(holders["pull_changes"], 30);
        assert_eq!(holders["read_asset"], 9);
        assert_eq!(setup["session_lock"]["calls"], 1);
        assert_eq!(setup["channel_lookup"]["calls"], 1);
        assert!(setup.get("by_id").is_none());
        assert!(foreground["archive"].get("details").is_none());
        assert_eq!(
            foreground["archive_refresh"]["details"]["steps"]["by_id"]["calls"],
            1
        );
        assert_eq!(payload["timings"]["foreground"]["scan"]["outcome"], "error");
        assert_eq!(
            payload["timings"]["foreground"]["publish_refresh"]["outcome"],
            "canceled"
        );
        let details = &payload["timings"]["foreground"]["publish_refresh"]["details"];
        assert_eq!(details["steps"]["index_download"]["calls"], 2);
        assert_eq!(details["steps"]["by_id"]["calls"], 1);
        assert!(details["steps"]["by_id"]["elapsed_ms"].is_u64());
        assert!(payload["timings"]["maintenance"]["scan"]
            .get("details")
            .is_none());
        assert_eq!(payload["timings"]["maintenance"]["scan"]["outcome"], "ok");
        core.shutdown().await;
    }
}
