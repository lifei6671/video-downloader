//! Last-attempt deletion timing is stored with the existing durable operation.
use crate::desktop_core::{Core, DeleteActivity};
use anyhow::Result;
use std::sync::Arc;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
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
    difference_pages: usize,
    difference_changed_ids: usize,
    history_messages: usize,
    unknown_media_bytes: u64,
    bucket: Option<&'static str>,
    last_checkpoint: Option<Instant>,
    session_wait_ended_ms: Option<i64>,
    prior: serde_json::Value,
    lock_holders: BTreeMap<&'static str, Duration>,
    steps: BTreeMap<&'static str, (Duration, u32)>,
}

tokio::task_local! {
    // One refresh can span multiple batches, documents and file-reference retries.
    static DETAILS: RefCell<TimingDetails>;
    static ACTIVE_DELETE: (Arc<Core>, String);
    static SEEN_PHASES: RefCell<BTreeSet<String>>;
    static COMPANION: Option<String>;
}

pub(super) async fn track_delete<F: Future>(
    core: &Arc<Core>,
    operation: &str,
    work: F,
) -> F::Output {
    ACTIVE_DELETE
        .scope(
            (core.clone(), operation.to_owned()),
            SEEN_PHASES.scope(RefCell::default(), work),
        )
        .await
}

pub(super) async fn with_companion<F: Future>(operation: Option<&str>, work: F) -> F::Output {
    COMPANION.scope(operation.map(str::to_owned), work).await
}

// Nested work restores its parent when it returns, fails, or is canceled.
pub(super) struct ActivitySpan {
    core: Arc<Core>,
    previous: Option<DeleteActivity>,
}

pub(super) fn activity(step: &str) -> Option<ActivitySpan> {
    ACTIVE_DELETE
        .try_with(|(core, operation)| {
            let previous = core.replace_delete_activity(Some(DeleteActivity {
                operation_id: operation.clone(),
                companion: COMPANION.try_with(Clone::clone).ok().flatten(),
                step: step.to_owned(),
            }));
            ActivitySpan {
                core: core.clone(),
                previous,
            }
        })
        .ok()
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

pub(super) fn record_difference_page(new_ids: usize) {
    let _ = DETAILS.try_with(|details| {
        let mut details = details.borrow_mut();
        details.difference_pages += 1;
        details.difference_changed_ids += new_ids;
    });
}

pub(super) fn record_history_message() {
    let _ = DETAILS.try_with(|details| details.borrow_mut().history_messages += 1);
}

pub(super) fn record_index_reuse(count: usize) {
    let _ = DETAILS.try_with(|details| details.borrow_mut().reused_index_bodies = count);
}

pub(super) struct StepSpan {
    _activity: Option<ActivitySpan>,
    step: &'static str,
    start: Instant,
    bytes: u64,
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
                bytes: 0,
            })
        })
        .ok()
        .flatten()
}

impl StepSpan {
    pub fn add_bytes(&mut self, count: usize) {
        self.bytes += count as u64;
    }
}

impl Drop for StepSpan {
    fn drop(&mut self) {
        let _ = DETAILS.try_with(|details| {
            let mut details = details.borrow_mut();
            if self.step == "unknown_media_verify" {
                details.unknown_media_bytes += self.bytes;
            }
            let (elapsed, calls) = details.steps.entry(self.step).or_default();
            *elapsed += self.start.elapsed();
            *calls += 1;
            if self.step == "session_lock" {
                details.session_wait_ended_ms = Some(chrono::Utc::now().timestamp_millis());
            }
        });
        // Preserve partial evidence across a crash without a write for every RPC/chunk.
        let _ = ACTIVE_DELETE.try_with(|(core, operation)| {
            let _ = DETAILS.try_with(|details| {
                let mut details = details.borrow_mut();
                if let Some(bucket) = details.bucket {
                    if details.last_checkpoint.is_some_and(|last| last.elapsed() < Duration::from_secs(3)) {
                        return;
                    }
                    details.last_checkpoint = Some(Instant::now());
                    let path = format!("$.timings.{bucket}.{}", details.owner);
                    let values = details.running_record();
                    if core.backup_db(|db| {
                        db.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,?2,json_set(json(?3),'$.attempt',attempt)) WHERE operation_id=?1", rusqlite::params![operation,path,values.to_string()])?;
                        Ok(())
                    }).is_err() {
                        eprintln!("backup_delete timing persistence failed");
                    }
                }
            });
        });
    }
}

impl TimingDetails {
    fn running_record(&self) -> serde_json::Value {
        serde_json::json!({"outcome":"running", "details":self.json(),
            "calls": self.prior["calls"].as_u64().unwrap_or(0) + 1,
            "total_elapsed_ms": self.prior["total_elapsed_ms"].as_u64().unwrap_or(0)})
    }

    fn json(&self) -> serde_json::Value {
        let mut current = self.sample();
        add_detail_totals(&mut current, &self.prior["details"]);
        if let Some(at) = self.session_wait_ended_ms {
            current["session_wait_ended_ms"] = at.into();
        }
        current
    }

    fn sample(&self) -> serde_json::Value {
        let steps: BTreeMap<_, _> = self
            .steps
            .iter()
            .map(|(step, (elapsed, calls))| {
                (
                    *step,
                    serde_json::json!({"elapsed_ms":elapsed.as_millis() as u64,"calls":calls}),
                )
            })
            .collect();
        if self.setup {
            let holders: BTreeMap<_, _> = self
                .lock_holders
                .iter()
                .filter(|(_, d)| d.as_millis() > 0)
                .map(|(kind, d)| (kind, d.as_millis() as u64))
                .collect();
            return serde_json::json!({"steps":steps,"session_lock_holders_ms":holders,
                "unknown_media_verify_bytes":self.unknown_media_bytes,
                "full_scan_calls":usize::from(self.full_scan_reason.is_some()),
                "full_scan_reason":self.full_scan_reason,"history_messages":self.history_messages});
        }
        serde_json::json!({"steps":steps,"full_scan":self.full_scan_reason.is_some(),
            "full_scan_calls":usize::from(self.full_scan_reason.is_some()),
            "full_scan_reason":self.full_scan_reason,"difference_reason":self.difference_reason,
            "difference_fallback_reason":self.difference_reason,"difference_pages":self.difference_pages,
            "difference_changed_ids":self.difference_changed_ids,"history_messages":self.history_messages,
            "reused_index_bodies":self.reused_index_bodies})
    }
}

// Repeated stages in one attempt (e.g. two confirmed segments) accumulate. Reasons
// describe the latest non-empty observation; counts are summed, flags are ORed.
fn add_detail_totals(current: &mut serde_json::Value, prior: &serde_json::Value) {
    use serde_json::Value;
    match (current, prior) {
        (Value::Object(now), Value::Object(old)) => {
            for (key, value) in old {
                if let Some(slot) = now.get_mut(key) {
                    add_detail_totals(slot, value);
                } else {
                    now.insert(key.clone(), value.clone());
                }
            }
        }
        (slot @ Value::Number(_), Value::Number(old)) => {
            *slot = slot
                .as_u64()
                .unwrap_or(0)
                .saturating_add(old.as_u64().unwrap_or(0))
                .into();
        }
        (Value::Bool(now), Value::Bool(old)) => *now |= old,
        (slot @ Value::Null, old) => *slot = old.clone(),
        _ => {}
    }
}

fn stage_details(phase: &str) -> Option<serde_json::Value> {
    DETAILS
        .try_with(|details| {
            let details = details.borrow();
            (details.owner == phase).then(|| details.json())
        })
        .ok()
        .flatten()
}

struct Stage<'a> {
    core: &'a Core,
    operation: &'a str,
    phase: &'a str,
    bucket: Option<&'static str>,
    start: Instant,
    outcome: &'static str,
    prior_calls: i64,
    prior_elapsed_ms: i64,
}
impl Drop for Stage<'_> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_millis().min(i64::MAX as u128) as i64;
        eprintln!(
            "backup_delete operation={} phase={} elapsed_ms={} outcome={} at_ms={} bucket={}",
            self.operation,
            self.phase,
            elapsed,
            self.outcome,
            chrono::Utc::now().timestamp_millis(),
            self.bucket.unwrap_or("unknown")
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
                 json_patch(json_object('elapsed_ms',?3,'outcome',?4,'at',unixepoch(),'ended_ms',CAST(unixepoch('subsec')*1000 AS INTEGER),'attempt',attempt,'calls',?6,'total_elapsed_ms',?7),COALESCE(?5,'{}')))
                 WHERE operation_id=?1",
                rusqlite::params![self.operation,format!("$.timings.{}.{}",bucket,self.phase),elapsed,self.outcome,extra,self.prior_calls+1,self.prior_elapsed_ms.saturating_add(elapsed)],
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
    let state = core.backup_db(|db| {
        let (done, attempt, old): (bool, i64, Option<String>) = db.query_row(
            "SELECT COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0), attempt,
             json_extract(payload_json,'$.timings.' || CASE WHEN json_extract(payload_json,'$.local_cleanup_done') THEN 'maintenance' ELSE 'foreground' END || '.' || ?2)
             FROM backup_operations WHERE operation_id=?1",
            rusqlite::params![operation,phase], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let old: serde_json::Value = old.as_deref().and_then(|v| serde_json::from_str(v).ok())
            .filter(|v: &serde_json::Value| v["attempt"] == attempt && v["outcome"] != "running").unwrap_or_default();
        Ok((if done {"maintenance"} else {"foreground"},old))
    }).ok();
    let (bucket, mut prior) = state
        .map(|(bucket, prior)| (Some(bucket), prior))
        .unwrap_or_default();
    // Manual retry can reuse attempt number 1. Only accumulate within this execution.
    if SEEN_PHASES
        .try_with(|seen| {
            seen.borrow_mut()
                .insert(format!("{}.{}", bucket.unwrap_or_default(), phase))
        })
        .unwrap_or(false)
    {
        prior = serde_json::Value::Null;
    }
    let mut stage = Stage {
        core,
        operation,
        phase,
        bucket,
        start: Instant::now(),
        outcome: "canceled",
        prior_calls: prior["calls"].as_i64().unwrap_or(0),
        prior_elapsed_ms: prior["total_elapsed_ms"].as_i64().unwrap_or(0),
    };
    let _ = DETAILS.try_with(|details| {
        let mut details = details.borrow_mut();
        if details.owner == phase {
            details.bucket = bucket;
            details.prior = prior;
        }
    });
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

pub(super) async fn timed_refresh<T>(
    core: &Core,
    operation: &str,
    phase: &str,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    let details = TimingDetails {
        owner: phase.into(),
        ..Default::default()
    };
    DETAILS
        .scope(RefCell::new(details), timed(core, operation, phase, work))
        .await
}

pub(super) async fn read_asset_snapshot<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    match ACTIVE_DELETE.try_with(Clone::clone) {
        Ok((core, operation)) => timed_refresh(&core, &operation, "read_asset_refresh", work).await,
        Err(_) => work.await,
    }
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
            timed_refresh(core, operation, &phase, work).await
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
                let phase = format!("{stage}_{step}");
                match step {
                    "locate" => timed_refresh(core, deletion, &phase, work).await,
                    "receipt" | "delete_old" => timed_setup(core, deletion, &phase, work).await,
                    _ => timed(core, deletion, &phase, work).await,
                }
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
        let core = Core::open(
            &root.path().join("qa.sqlite"),
            root.path().join("downloads"),
            "no-python".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        let task = track_delete(&core, "second", async {
            let _publish = activity("remote_cleanup");
            with_companion(Some("first"), async {
                let _batch = activity("delete_message");
                let current = core.current_delete_activity().unwrap();
                assert_eq!(current.operation_id, "second");
                assert_eq!(current.companion.as_deref(), Some("first"));
                assert_eq!(current.step, "delete_message");
            })
            .await;
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
        core.backup_db(|db| {
            db.execute_batch("CREATE TEMP TABLE timing_writes(n INTEGER);
                INSERT INTO timing_writes VALUES(0);
                CREATE TEMP TRIGGER count_timing_writes AFTER UPDATE OF payload_json ON backup_operations
                BEGIN UPDATE timing_writes SET n=n+1; END;")?;
            Ok(())
        }).unwrap();
        track_delete(
            &core,
            "timing",
            timed_refresh(&core, "timing", "bulk", async {
                for _ in 0..100 {
                    drop(refresh_span("by_id"));
                }
                Ok(())
            }),
        )
        .await
        .unwrap();
        let (writes, calls): (u32, u32) = core.backup_db(|db| Ok(db.query_row(
            "SELECT n,json_extract(payload_json,'$.timings.foreground.bulk.details.steps.by_id.calls')
             FROM timing_writes,backup_operations WHERE operation_id='timing'", [], |r| Ok((r.get(0)?, r.get(1)?))
        )?)).unwrap();
        assert_eq!((writes, calls), (2, 100));
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
        track_delete(&core, "timing", async {
          for _ in 0..2 {
            timed_refresh(&core, "timing", "pages", async {
                record_difference_page(3);
                record_history_message();
                drop(refresh_span("channel_changes"));
                // A separate connection sees completed work before the enclosing future returns.
                let db=rusqlite::Connection::open(&path)?;
                let count: i64=db.query_row("SELECT json_extract(payload_json,'$.timings.foreground.pages.details.steps.channel_changes.calls') FROM backup_operations WHERE operation_id='timing'",[],|r|r.get(0))?;
                assert!(count>=1);
                Ok(())
            }).await.unwrap();
          }
        }).await;
        let totals:String=core.backup_db(|db| Ok(db.query_row("SELECT json_extract(payload_json,'$.timings.foreground.pages') FROM backup_operations WHERE operation_id='timing'",[],|r|r.get(0))?)).unwrap();
        let totals: serde_json::Value = serde_json::from_str(&totals).unwrap();
        assert_eq!(totals["calls"], 2);
        assert_eq!(totals["details"]["steps"]["channel_changes"]["calls"], 2);
        assert_eq!(totals["details"]["difference_changed_ids"], 6);
        assert_eq!(totals["details"]["difference_pages"], 2);
        assert_eq!(totals["details"]["history_messages"], 2);
        // A new execution may have the same numeric attempt after manual retry.
        track_delete(
            &core,
            "timing",
            timed_refresh(&core, "timing", "pages", async {
                drop(refresh_span("channel_changes"));
                Ok(())
            }),
        )
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
        assert_eq!(foreground["pages"]["calls"], 1);
        assert_eq!(
            foreground["pages"]["details"]["steps"]["channel_changes"]["calls"],
            1
        );
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
