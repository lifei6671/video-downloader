//! Shared merge policy and durable synchronization over the provider business contract.
use super::provider::*;
use anyhow::{bail, Context, Result};
use serde_json::Value;

/// Keep committed values on a true field conflict and publish the alternative exactly once.
pub fn merge(change: &AssetChange, remote: Option<&AssetSnapshot>) -> Result<AssetChange> {
    if remote.is_some_and(|v| v.deleted) && !change.delete && !change.recreate {
        bail!("BACKUP_ASSET_DELETED");
    }
    if remote.is_some_and(|v| v.deleted) && change.recreate && !change.delete {
        if change.media.is_none() {
            bail!("BACKUP_INVALID_RECORD");
        }
        let mut recreated = change.clone();
        recreated
            .conflicts
            .retain(|conflict| !change.resolved_conflicts.contains(conflict));
        recreated.base = remote.cloned();
        return Ok(recreated);
    }
    if let Some(remote) = remote {
        if remote.asset_id != change.asset_id
            || change
                .base
                .as_ref()
                .is_some_and(|b| b.library_id != remote.library_id || b.asset_id != remote.asset_id)
        {
            bail!("BACKUP_SCOPE_MISMATCH");
        }
    }
    let mut merged = change.clone();
    merged.base = remote.cloned();
    merged.conflicts = remote.map(|r| r.conflicts.clone()).unwrap_or_default();
    for conflict in &change.conflicts {
        if !merged.conflicts.contains(conflict) {
            merged.conflicts.push(conflict.clone());
        }
    }
    merged
        .conflicts
        .retain(|c| !change.resolved_conflicts.contains(c));
    for (field, local) in &change.fields {
        let baseline = change
            .field_bases
            .get(field)
            .map(FieldBaseline::value)
            .unwrap_or_else(|| change.base.as_ref().and_then(|b| b.fields.get(field)));
        let current = remote.and_then(|r| r.fields.get(field));
        if current == Some(local) {
            merged.fields.remove(field);
            continue;
        }
        if baseline == Some(local) {
            merged.fields.remove(field);
            continue;
        }
        if remote.is_some() && current != baseline {
            add_conflict(&mut merged.conflicts, field, local.clone(), change);
            merged.fields.remove(field);
        }
    }
    for field in ["cover", "transcript"] {
        let update = if field == "cover" {
            &change.cover
        } else {
            &change.transcript
        };
        let local = match update {
            ReferenceChange::Unchanged => continue,
            ReferenceChange::Set(r) => Some(r),
            ReferenceChange::Clear => None,
        };
        let reference = |r: &AssetSnapshot| {
            if field == "cover" {
                r.cover.clone()
            } else {
                r.transcript.clone()
            }
        };
        let baseline = change.base.as_ref().and_then(reference);
        let current = remote.and_then(reference);
        let baseline = change
            .field_bases
            .get(field)
            .map(|v| v.value().cloned())
            .unwrap_or_else(|| baseline.as_ref().map(content_identity));
        let current = current.as_ref().map(content_identity);
        let identity = local.map(content_identity);
        let unchanged = current == identity || baseline == identity;
        let conflict = remote.is_some() && current != baseline && !unchanged;
        if conflict {
            add_conflict(
                &mut merged.conflicts,
                field,
                serde_json::to_value(local)?,
                change,
            );
        }
        if unchanged || conflict {
            if field == "cover" {
                merged.cover = ReferenceChange::Unchanged;
            } else {
                merged.transcript = ReferenceChange::Unchanged;
            }
        }
    }
    if let (Some(local), Some(existing)) = (&change.media, remote.and_then(|r| r.media.as_ref())) {
        if local.sha256 != existing.sha256 || local.bytes != existing.bytes {
            bail!("BACKUP_IMMUTABLE_MEDIA_CHANGED");
        }
        merged.media = None;
    }
    Ok(merged)
}

fn add_conflict(
    conflicts: &mut Vec<ConflictVersion>,
    field: &str,
    value: Value,
    change: &AssetChange,
) {
    if !conflicts.iter().any(|c| {
        c.field == field
            && (c.value == value
                || matches!(field, "cover" | "transcript")
                    && conflict_content(&c.value) == conflict_content(&value))
    }) {
        conflicts.push(ConflictVersion {
            field: field.into(),
            value,
            device_id: change.device_id.clone(),
            operation_id: change.operation_id.clone(),
        });
    }
}

fn content_identity(reference: &MediaRef) -> Value {
    json!({"sha256":reference.sha256,"bytes":reference.bytes,"container":reference.container})
}
fn conflict_content(value: &Value) -> Option<Value> {
    if value.is_null() {
        Some(Value::Null)
    } else {
        serde_json::from_value::<MediaRef>(value.clone())
            .ok()
            .as_ref()
            .map(content_identity)
    }
}

/// A provider's accepted receipt never advances the local acknowledged revision.
pub async fn publish(provider: &dyn BackupProvider, change: AssetChange) -> Result<PublishReceipt> {
    change.validate_scope(provider.scope())?;
    provider.capabilities().require_multi_writer()?;
    if provider.capabilities().change_coordination == ChangeCoordination::EventReplay {
        return provider.publish_change(change, None).await;
    }
    for _ in 0..5 {
        let current = provider.read_asset(&change.asset_id).await?;
        if change.delete && current.as_ref().is_some_and(|r| r.snapshot.deleted) {
            return Ok(PublishReceipt::Converged {
                asset: Box::new(
                    current.ok_or_else(|| anyhow::anyhow!("BACKUP_RECORD_UNAVAILABLE"))?,
                ),
            });
        }
        let merged = merge(&change, current.as_ref().map(|r| &r.snapshot))?;
        let result = provider
            .publish_change(merged, current.map(|r| r.version))
            .await?;
        match result {
            PublishReceipt::Retry { .. } => continue,
            receipt => return Ok(receipt),
        }
    }
    Ok(PublishReceipt::Retry {
        reason: "BACKUP_CONCURRENT_UPDATE".into(),
    })
}

use super::{
    model::{Asset, AssetTranscript},
    repository, scheduler,
};
use crate::desktop_core::Core;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::Duration;
use tokio::time::Instant;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldEdit {
    pub value: Value,
    pub revision: u64,
    pub base: FieldBaseline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictChoice {
    pub conflict: ConflictVersion,
    pub adopt: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PendingChanges {
    pub recreate: bool,
    pub recreate_revision: Option<u64>,
    pub retained_conflicts: Vec<ConflictVersion>,
    pub choices: Vec<ConflictChoice>,
    pub fields: std::collections::BTreeMap<String, FieldEdit>,
    pub media: Option<u64>,
    pub cover: Option<u64>,
    pub transcript: Option<u64>,
    pub reference_bases: std::collections::BTreeMap<String, FieldBaseline>,
}
impl PendingChanges {
    pub fn transcript_dirty(&self) -> bool {
        self.transcript.is_some()
            || self
                .choices
                .iter()
                .any(|c| c.adopt && c.conflict.field == "transcript")
    }
    pub fn is_empty(&self) -> bool {
        !self.recreate
            && self.retained_conflicts.is_empty()
            && self.fields.is_empty()
            && self.choices.is_empty()
            && self.media.is_none()
            && self.cover.is_none()
            && self.transcript.is_none()
    }
    fn acknowledge(&mut self, revision: u64) {
        self.recreate = false;
        self.recreate_revision = None;
        self.choices.retain(|choice| choice.revision > revision);
        self.fields.retain(|_, edit| edit.revision > revision);
        for (name, field) in [
            ("media", &mut self.media),
            ("cover", &mut self.cover),
            ("transcript", &mut self.transcript),
        ] {
            if field.is_some_and(|r| r <= revision) {
                *field = None;
                self.reference_bases.remove(name);
            }
        }
    }
}

pub fn fields(asset: &Asset) -> FieldPatch {
    let m = &asset.metadata;
    let mut fields: FieldPatch = ["title", "author", "duration_ms", "width", "height"]
        .into_iter()
        .map(|k| (k.into(), m[k].clone()))
        .collect();
    let mut source = json!({"platform":m["platform"],"video_id":m["video_id"],"canonical_url":m["canonical_url"]});
    // Avatar cache flags are local state; preserve the portable author identity only.
    if let Some(profile) = m["author_profile"].as_object() {
        let profile: serde_json::Map<String, Value> = ["id", "nickname", "handle", "kind"]
            .into_iter()
            .filter_map(|key| profile.get(key).map(|v| (key.into(), v.clone())))
            .collect();
        source["author_profile"] = Value::Object(profile);
    }
    let completed = m["completed_at"]
        .as_i64()
        .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        .map(|v| v.to_rfc3339())
        .or_else(|| m["completed_at"].as_str().map(str::to_owned));
    fields.insert("completed_at".into(), json!(completed));
    fields.insert(
        "original_filename".into(),
        asset.original_filename.clone().into(),
    );
    fields.insert("source".into(), source);
    fields
}

/// Only a newly completed local download joins the selected space. Imports never call this.
pub fn register_download(db: &Connection, local: &str) -> Result<()> {
    let current:Option<String>=db.query_row("SELECT s.space_id FROM backup_current_space c JOIN backup_spaces s USING(space_id) JOIN backup_provider_config p USING(space_id) WHERE c.id=1 AND COALESCE((SELECT enabled FROM backup_sync_preference WHERE id=1),s.enabled)=1 AND p.provider_type IN ('s3','telegram')",[],|r|r.get(0)).optional()?;
    let Some(space) = current else {
        return Ok(());
    };
    let existing: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_space_assets WHERE local_asset_id=?)",
        [local],
        |r| r.get(0),
    )?;
    if existing {
        return Ok(());
    }
    let asset = repository::list_assets(db)?
        .into_iter()
        .find(|a| a.asset_id == local)
        .ok_or_else(|| anyhow::anyhow!("BACKUP_ASSET_NOT_FOUND"))?;
    join_asset(db, &space, &asset)
}

fn join_asset(db: &Connection, space: &str, asset: &Asset) -> Result<()> {
    let local = &asset.asset_id;
    let pending = PendingChanges {
        recreate: false,
        recreate_revision: None,
        retained_conflicts: vec![],
        choices: vec![],
        fields: fields(asset)
            .into_iter()
            .map(|(k, value)| {
                (
                    k,
                    FieldEdit {
                        value,
                        revision: 1,
                        base: FieldBaseline::Missing,
                    },
                )
            })
            .collect(),
        media: Some(1),
        cover: Some(1),
        transcript: Some(1),
        reference_bases: Default::default(),
    };
    db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id,local_revision,pending_json) VALUES(?1,?2,?2,1,?3)",params![space,local,serde_json::to_string(&pending)?])?;
    Ok(())
}

/// Preserve local edits even when this space has a remote tombstone or is currently paused.
pub fn mark_local(db: &Connection, local: &str, patch: FieldPatch, transcript: bool) -> Result<()> {
    let mut q=db.prepare("SELECT space_id,remote_asset_id,local_revision,pending_json FROM backup_space_assets WHERE local_asset_id=?")?;
    let rows = q
        .query_map([local], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (space, remote, revision, raw) in rows {
        let revision = revision + 1;
        let mut pending: PendingChanges = serde_json::from_str(&raw)?;
        let remote_raw: Option<String> = db
            .query_row(
                "SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",
                params![space, local],
                |r| r.get(0),
            )
            .optional()?;
        let remote_snapshot =
            remote_raw.and_then(|v| serde_json::from_str::<AssetSnapshot>(&v).ok());
        let operation:Option<String>=db.query_row("SELECT payload_json FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state NOT IN ('completed','canceled')",params![space,local],|r|r.get(0)).optional()?;
        let captured = operation
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .and_then(|v| serde_json::from_value::<Capture>(v["capture"].clone()).ok());
        for (field, value) in &patch {
            let existing = pending.fields.get(field);
            let in_flight = captured.as_ref().and_then(|c| c.pending.fields.get(field));
            let base = match (existing, in_flight) {
                (Some(existing), Some(sent)) if existing.revision == sent.revision => {
                    FieldBaseline::Value(sent.value.clone())
                }
                (Some(existing), _) => existing.base.clone(),
                _ => FieldBaseline::from_value(
                    remote_snapshot
                        .as_ref()
                        .and_then(|r| r.fields.get(field).cloned()),
                ),
            };
            pending.fields.insert(
                field.clone(),
                FieldEdit {
                    value: value.clone(),
                    revision,
                    base,
                },
            );
        }
        if transcript {
            let captured_revision = captured.as_ref().and_then(|c| c.pending.transcript);
            if pending.transcript.is_none() {
                pending.reference_bases.insert(
                    "transcript".into(),
                    FieldBaseline::from_value(
                        remote_snapshot
                            .as_ref()
                            .and_then(|r| r.transcript.as_ref())
                            .map(content_identity),
                    ),
                );
            } else if pending.transcript == captured_revision {
                pending.reference_bases.insert("transcript".into(),FieldBaseline::from_value(captured.as_ref().and_then(|c|{
                    c.transcript.as_ref().and_then(|t|{let mut t=t.clone();t.asset_id=c.remote.clone();serde_json::to_vec(&t).ok()})
                        .map(|body|json!({"sha256":format!("{:x}",Sha256::digest(&body)),"bytes":body.len(),"container":"json"}))
                })));
            }
            pending.transcript = Some(revision);
        }
        db.execute("UPDATE backup_space_assets SET local_revision=?3,pending_json=?4 WHERE space_id=?1 AND remote_asset_id=?2",params![space,remote,i64::try_from(revision)?,serde_json::to_string(&pending)?])?;
    }
    Ok(())
}

type PullReply = tokio::sync::oneshot::Sender<Result<usize>>;

/// One pending catch-up per space. Manual refreshes during that batch share its actual result.
#[derive(Default)]
pub(crate) struct DeferredPulls {
    pending: std::sync::Mutex<std::collections::HashMap<String, Vec<PullReply>>>,
    finished: tokio::sync::Notify,
}

impl DeferredPulls {
    fn mark(&self, space: &str) {
        self.pending.lock().unwrap().entry(space.to_owned()).or_default();
    }

    fn has(&self, space: &str) -> bool {
        self.pending.lock().unwrap().contains_key(space)
    }

    fn fail_waiters(&self, space: &str, error: &anyhow::Error) {
        // Keep the space marked: after the queue recovers it still owes a catch-up.
        if let Some(replies) = self.pending.lock().unwrap().get_mut(space) {
            for reply in replies.drain(..) {
                let _ = reply.send(Err(anyhow::anyhow!(error.to_string())));
            }
        }
    }
}

pub struct SyncEngine {
    core: Arc<Core>,
    provider: Arc<dyn BackupProvider>,
}
impl SyncEngine {
    pub fn new(core: Arc<Core>, provider: Arc<dyn BackupProvider>) -> Self {
        Self { core, provider }
    }
    pub fn provider(&self) -> &dyn BackupProvider {
        self.provider.as_ref()
    }
    pub async fn refresh(&self) -> Result<usize> {
        self.check_space()?;
        let space = &self.provider.scope().space_id;
        let deferred = self.core.backup_db(|db| {
            let mut pending = self.core.deferred_pulls.pending.lock().unwrap();
            if !pending.contains_key(space) && !scheduler::foreground_delete_busy(db, space)? {
                return Ok(None);
            }
            let (reply, result) = tokio::sync::oneshot::channel();
            pending.entry(space.clone()).or_default().push(reply);
            Ok(Some(result))
        })?;
        if let Some(result) = deferred {
            let mut cancel = self.core.backup_cancellation();
            return scoped(self, &mut cancel, async { result.await.context("CANCELED")? }).await;
        }
        self.pull().await
    }

    pub async fn pull(&self) -> Result<usize> {
        self.pull_from("manual_refresh").await
    }

    async fn pull_from(&self, trigger: &'static str) -> Result<usize> {
        self.check_space()?;
        let space = &self.provider.scope().space_id;
        let raw: Option<String> = self.core.backup_db(|db| {
            Ok(db
                .query_row(
                    "SELECT checkpoint_json FROM backup_sync_checkpoints WHERE space_id=?",
                    [space],
                    |r| r.get(0),
                )
                .optional()?
                .flatten())
        })?;
        let checkpoint = raw.map(|v| serde_json::from_str(&v)).transpose()?;
        let mut shutdown = self.core.backup_cancellation();
        if *shutdown.borrow() {
            bail!("CANCELED");
        }
        let batch = scoped(self, &mut shutdown, async {
            let mut timing = super::pull_timing::Run::start(&self.core, space, trigger);
            let result = self.provider.pull_changes(checkpoint).await;
            timing.finish(if result.is_ok() { "ok" } else { "error" });
            result
        }).await?;
        self.check_space()?;
        if !batch.complete {
            bail!("BACKUP_INCOMPLETE_DISCOVERY");
        }
        let count = batch.assets.len();
        self.core.backup_db(|db|{
            let tx=db.unchecked_transaction()?;
            for asset in &batch.assets {import_asset(&tx,self.provider.scope(),asset)?;}
            tx.execute("INSERT INTO backup_sync_checkpoints(space_id,checkpoint_json,last_success_at,last_error) VALUES(?1,?2,unixepoch(),NULL) ON CONFLICT(space_id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json,last_success_at=excluded.last_success_at,last_error=NULL",params![space,serde_json::to_string(&batch.checkpoint)?])?;
            tx.execute("INSERT INTO backup_window_sync(space_id,last_success_ms) VALUES(?1,CAST(unixepoch('subsec')*1000 AS INTEGER))
                ON CONFLICT(space_id) DO UPDATE SET last_success_ms=excluded.last_success_ms", [space])?;
            tx.commit()?;Ok(())
        })?;
        Ok(count)
    }
    pub fn enqueue(&self) -> Result<()> {
        let scope = self.provider.scope();
        self.core.backup_db(|db|{
            let mut q=db.prepare("SELECT m.local_asset_id,m.pending_json FROM backup_space_assets m JOIN backup_spaces s USING(space_id) WHERE m.space_id=? AND (m.remote_deleted=0 OR COALESCE(json_extract(m.pending_json,'$.recreate'),0)=1) AND s.enabled=1 AND NOT EXISTS(SELECT 1 FROM backup_operations o WHERE o.space_id=m.space_id AND o.asset_id=m.local_asset_id AND o.kind='delete' AND o.state NOT IN ('completed','canceled'))")?;
            let rows=q.query_map([&scope.space_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for (local,pending) in rows {
                if serde_json::from_str::<PendingChanges>(&pending)?.is_empty() {continue;}
                let canceled:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state='canceled' AND phase<>'superseded')",params![scope.space_id,local],|r|r.get(0))?;
                if canceled {continue;}
                let payload=json!({"scope":scope});
                db.execute("INSERT OR IGNORE INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'upload','pending','queued',?4)",params![uuid::Uuid::new_v4().to_string(),scope.space_id,local,payload.to_string()])?;
            }
            Ok(())
        })
    }
    fn check(&self, operation: &str) -> Result<()> {
        check_operation(&self.core, self.provider.scope(), operation)
    }
    fn check_space(&self) -> Result<()> {
        let scope = self.provider.scope();
        self.core.backup_db(|db|{let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_spaces s JOIN backup_current_space c USING(space_id) JOIN backup_provider_config p USING(space_id) WHERE s.space_id=?1 AND s.library_id=?2 AND s.enabled=1 AND p.provider_type IN ('s3','telegram') AND p.protocol=2 AND p.provider_type=?3)",params![scope.space_id,scope.library_id,serde_json::to_value(scope.provider_type)?.as_str()],|r|r.get(0))?;if !valid {bail!("BACKUP_SPACE_INACTIVE");}Ok(())})
    }
    fn save_payload(&self, operation: &str, phase: &str, payload: &Value) -> Result<()> {
        self.check(operation)?;
        self.core.backup_db(|db|{
            let raw:String=db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?",[operation],|r|r.get(0))?;
            let previous:Value=serde_json::from_str(&raw)?;
            let mut next=payload.clone();
            for (key,value) in previous.as_object().into_iter().flatten() {
                if key.starts_with("resume_") || key=="completed_bytes" || key=="transfer_started" { next[key]=value.clone(); }
            }
            db.execute("UPDATE backup_operations SET phase=?2,payload_json=?3,updated_at=unixepoch() WHERE operation_id=?1",params![operation,phase,next.to_string()])?;Ok(())})
    }
    pub async fn execute_upload(&self, operation: &str) -> Result<()> {
        use anyhow::Context;
        let _writer = self.core.backup_write_lock.read().await;
        self.check(operation)?;
        let op = scheduler::operation(&self.core, operation)?;
        let local = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
        // Different assets may transfer together; a retry of the same asset cannot.
        // Keep global -> asset lock order consistent with destructive file work.
        let _file = self.core.restore_file_guard(local).await;
        self.check(operation)?;
        let scope = self.provider.scope();
        let mut payload = op.payload.clone();
        let saved_scope: Scope =
            serde_json::from_value(payload["scope"].clone()).context("BACKUP_INVALID_OPERATION")?;
        scope.check(&saved_scope)?;
        let capture: Capture = if payload.get("capture").is_some() {
            serde_json::from_value(payload["capture"].clone())?
        } else {
            let media_dirty:bool=self.core.backup_db(|db|{let raw:String=db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local],|r|r.get(0))?;Ok(serde_json::from_str::<PendingChanges>(&raw)?.media.is_some())})?;
            if media_dirty {
                self.core.verify_backup_asset(local).await?;
            }
            let capture=self.core.backup_db(|db|{
                let (remote,revision,pending,baseline):(String,u64,String,Option<String>)=db.query_row("SELECT remote_asset_id,local_revision,pending_json,baseline_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local],|r|Ok((r.get(0)?,r.get::<_,i64>(1)? as u64,r.get(2)?,r.get(3)?)))?;
                let asset=repository::list_assets(db)?.into_iter().find(|a|a.asset_id==local).context("BACKUP_ASSET_NOT_FOUND")?;
                let pending:PendingChanges=serde_json::from_str(&pending)?;
                let transcript=if pending.transcript.is_some() {repository::transcript(db,local)?} else {None};
                let cover=if pending.cover.is_some() {db.query_row("SELECT body FROM asset_covers WHERE asset_id=?",[local],|r|r.get::<_,Vec<u8>>(0)).optional()?} else {None};
                let mut conflicts=pending.retained_conflicts.clone();
                if pending.recreate {
                    let mut q=db.prepare("SELECT value_json FROM backup_conflicts WHERE space_id=?1 AND remote_asset_id=?2")?;
                    let values=q.query_map(params![scope.space_id,remote],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    for raw in values {
                        let conflict:ConflictVersion=serde_json::from_str(&raw)?;
                        if !conflicts.contains(&conflict) {conflicts.push(conflict);}
                    }
                }
                let baseline=if pending.recreate && scope.provider_type==ProviderType::Telegram {
                    Some(db.query_row("SELECT record_json FROM backup_tombstones WHERE space_id=?1 AND remote_asset_id=?2",params![scope.space_id,remote],|r|r.get::<_,String>(0))?)
                } else {baseline};
                Ok(Capture{remote,revision,pending,baseline:baseline.map(|v|serde_json::from_str(&v)).transpose()?,asset,transcript,cover,conflicts})
            })?;
            payload["capture"] = serde_json::to_value(&capture)?;
            self.save_payload(operation, "captured", &payload)?;
            capture
        };
        let mut change = AssetChange {
            recreate: capture.pending.recreate,
            resolved_conflicts: capture
                .pending
                .choices
                .iter()
                .map(|c| c.conflict.clone())
                .collect(),
            asset_id: capture.remote.clone(),
            base: capture.baseline.clone(),
            fields: capture
                .pending
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), v.value.clone()))
                .collect(),
            media: None,
            cover: ReferenceChange::Unchanged,
            transcript: ReferenceChange::Unchanged,
            delete: false,
            device_id: self.core.backup_db(repository::device_id)?,
            operation_id: operation.into(),
            conflicts: capture.conflicts.clone(),
            field_bases: capture
                .pending
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), v.base.clone()))
                .chain(capture.pending.reference_bases.clone())
                .collect(),
        };
        for choice in capture.pending.choices.iter().filter(|c| c.adopt) {
            let c = &choice.conflict;
            if matches!(c.field.as_str(), "cover" | "transcript") {
                if c.field == "cover" && capture.pending.cover.is_some()
                    || c.field == "transcript" && capture.pending.transcript.is_some()
                {
                    continue;
                }
                let update = if c.value.is_null() {
                    ReferenceChange::Clear
                } else {
                    ReferenceChange::Set(serde_json::from_value(c.value.clone())?)
                };
                if c.field == "cover" {
                    change.cover = update;
                } else {
                    change.transcript = update;
                }
            } else if !change.fields.contains_key(&c.field) {
                change.fields.insert(c.field.clone(), c.value.clone());
            }
        }
        if capture.pending.media.is_some() {
            let hash = capture
                .asset
                .content_sha256
                .clone()
                .context("BACKUP_HASH_PENDING")?;
            let path = capture.asset.path.clone().context("FILE_MISSING")?;
            let container = std::path::Path::new(&capture.asset.original_filename)
                .extension()
                .and_then(|v| v.to_str())
                .context("BACKUP_INVALID_FILENAME")?
                .to_ascii_lowercase();
            let receipt = self
                .upload_source(
                    operation,
                    "video",
                    VideoAttributes::from_metadata(&capture.asset.metadata),
                    path,
                    capture.asset.content_bytes,
                    hash,
                    container,
                    &change.device_id,
                    &capture.remote,
                    &payload,
                )
                .await?;
            payload["video_receipt"] = serde_json::to_value(&receipt)?;
            change.media = Some(receipt.reference);
            self.save_payload(operation, "media_uploaded", &payload)?;
            self.core.verify_backup_asset(local).await?;
            let actual = self
                .core
                .backup_assets()?
                .into_iter()
                .find(|a| a.asset_id == local)
                .context("BACKUP_ASSET_NOT_FOUND")?;
            if actual.content_sha256 != capture.asset.content_sha256
                || actual.content_bytes != capture.asset.content_bytes
            {
                bail!("BACKUP_SOURCE_CHANGED");
            }
        }
        for kind in ["cover", "transcript"] {
            let body = if kind == "cover" {
                capture.cover.clone()
            } else {
                capture
                    .transcript
                    .as_ref()
                    .filter(|t| {
                        t.content_sha256 == capture.asset.content_sha256
                            && t.content_sha256.is_some()
                    })
                    .map(|t| {
                        let mut t = t.clone();
                        t.asset_id = capture.remote.clone();
                        serde_json::to_vec(&t)
                    })
                    .transpose()?
            };
            let Some(body) = body else {
                continue;
            };
            let container = if kind == "cover" {
                crate::files::image_extension(&body).context("INVALID_MEDIA_CONTENT")?
            } else {
                "json"
            };
            let mut temporary = tempfile::NamedTempFile::new()?;
            use std::io::Write;
            temporary.write_all(&body)?;
            temporary.flush()?;
            let receipt = self
                .upload_source(
                    operation,
                    kind,
                    None,
                    temporary.path().to_owned(),
                    body.len() as u64,
                    format!("{:x}", Sha256::digest(&body)),
                    container.into(),
                    &change.device_id,
                    &capture.remote,
                    &payload,
                )
                .await?;
            if kind == "cover" {
                change.cover = ReferenceChange::Set(receipt.reference);
            } else {
                change.transcript = ReferenceChange::Set(receipt.reference);
            }
        }
        payload["change"] = serde_json::to_value(&change)?;
        self.save_payload(operation, "publishing", &payload)?;
        match publish(self.provider.as_ref(), change).await? {
            PublishReceipt::Converged { asset } => {
                self.check(operation)?;
                self.core.backup_db(|db|{
                    let tx=db.unchecked_transaction()?;
                    let raw:String=tx.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local],|r|r.get(0))?;
                    let mut pending:PendingChanges=serde_json::from_str(&raw)?;pending.acknowledge(capture.revision);
                    pending.retained_conflicts.retain(|conflict|!capture.conflicts.contains(conflict));
                    tx.execute("UPDATE backup_space_assets SET pending_json=?3,ack_revision=MAX(ack_revision,?4),baseline_json=?5 WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local,serde_json::to_string(&pending)?,i64::try_from(capture.revision)?,serde_json::to_string(&asset.snapshot)?])?;
                    import_asset(&tx,scope,&asset)?;
                    if let Some(value)=payload.get("video_receipt") {
                        let receipt:UploadReceipt=serde_json::from_value(value.clone())?;
                        receipt.reference.validate_scope(scope)?;
                        if receipt.operation_id==operation&&receipt.device_id==self.core_device(&tx)?&&asset.snapshot.media.as_ref().is_some_and(|m|m==&receipt.reference) {
                            tx.execute("INSERT INTO backup_upload_proofs VALUES(?1,?2,?3,?4,?5) ON CONFLICT(space_id,remote_asset_id,device_id) DO UPDATE SET operation_id=excluded.operation_id,media_sha256=excluded.media_sha256",params![scope.space_id,capture.remote,receipt.device_id,receipt.operation_id,receipt.reference.sha256])?;
                        }
                    }
                    tx.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL,payload_json=?2 WHERE operation_id=?1",params![operation,payload.to_string()])?;
                    tx.commit()?;Ok(())
                })
            }
            PublishReceipt::Accepted { .. } => bail!("BACKUP_PUBLICATION_UNCONFIRMED"),
            PublishReceipt::Retry { reason } => bail!(reason),
        }
    }
    fn core_device(&self, db: &Connection) -> Result<String> {
        repository::device_id(db)
    }
    #[allow(clippy::too_many_arguments)]
    async fn upload_source(
        &self,
        operation: &str,
        kind: &str,
        video: Option<VideoAttributes>,
        path: std::path::PathBuf,
        bytes: u64,
        sha256: String,
        container: String,
        device: &str,
        remote: &str,
        payload: &Value,
    ) -> Result<UploadReceipt> {
        let core = self.core.clone();
        let scope = self.provider.scope().clone();
        let id = operation.to_owned();
        let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let check_core = core.clone();
        let check_scope = scope.clone();
        let check_id = id.clone();
        let field = format!("resume_{kind}");
        let persisted = payload
            .get(&field)
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        let control = TransferControl {
            check: Box::new(move || check_operation(&check_core, &check_scope, &check_id)),
            checkpoint: Box::new(move |state| {
                check_operation(&core, &scope, &id)?;
                core.backup_db(|db| {
                    let raw: String = db.query_row(
                        "SELECT payload_json FROM backup_operations WHERE operation_id=?",
                        [&id],
                        |r| r.get(0),
                    )?;
                    let mut payload: Value = serde_json::from_str(&raw)?;
                    payload[&field] = serde_json::to_value(state)?;
                    db.execute(
                        "UPDATE backup_operations SET payload_json=?2 WHERE operation_id=?1",
                        params![id, payload.to_string()],
                    )?;
                    Ok(())
                })
            }),
            progress: progress.clone(),
        };
        let upload = self.provider.upload_media(
            MediaSource {
                video,
                base_revision: payload["capture"]["baseline"]["revision"].as_u64(),
                recreate_revision: payload["capture"]["pending"]["recreate_revision"].as_u64(),
                asset_id: remote.into(),
                kind: kind.into(),
                path,
                bytes,
                sha256,
                container,
                operation_id: operation.into(),
                device_id: device.into(),
                control: Some(Arc::new(control)),
            },
            persisted,
        );
        tokio::pin!(upload);
        let mut timer = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            tokio::select! {
                result=&mut upload=>return result,
                _=timer.tick()=>{
                    self.check(operation)?;
                    self.core.backup_db(|db|{
                        let raw:String=db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?",[operation],|r|r.get(0))?;
                        let mut value:Value=serde_json::from_str(&raw)?;
                        let transferred = progress.load(std::sync::atomic::Ordering::Relaxed);
                        if transferred > 0 { value["transfer_started"] = json!(true); }
                        if kind=="video" {
                            let confirmed=value["resume_video"]["value"].as_str().and_then(|raw|serde_json::from_str::<Value>(raw).ok()).and_then(|v|v["completed_bytes"].as_u64()).unwrap_or(0);
                            value["completed_bytes"]=json!((confirmed+transferred).min(bytes));
                        }
                        db.execute("UPDATE backup_operations SET payload_json=?2 WHERE operation_id=?1",params![operation,value.to_string()])?;Ok(())
                    })?;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Capture {
    remote: String,
    revision: u64,
    pending: PendingChanges,
    baseline: Option<AssetSnapshot>,
    asset: Asset,
    transcript: Option<AssetTranscript>,
    cover: Option<Vec<u8>>,
    conflicts: Vec<ConflictVersion>,
}

fn check_operation(core: &Core, scope: &Scope, operation: &str) -> Result<()> {
    if *core.backup_cancellation().borrow() {
        bail!("CANCELED");
    }
    core.backup_db(|db|{
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations o JOIN backup_spaces s USING(space_id) JOIN backup_provider_config p USING(space_id) JOIN backup_current_space c USING(space_id) LEFT JOIN backup_space_assets m ON m.space_id=o.space_id AND m.local_asset_id=o.asset_id WHERE o.operation_id=?1 AND o.space_id=?2 AND o.state='running' AND s.enabled=1 AND s.library_id=?3 AND p.protocol=2 AND p.provider_type IN ('s3','telegram') AND p.provider_type=?4 AND c.id=1 AND (m.remote_deleted=0 OR COALESCE(json_extract(m.pending_json,'$.recreate'),0)=1 OR o.kind='delete') AND (o.kind='delete' OR NOT EXISTS(SELECT 1 FROM backup_operations d WHERE d.space_id=o.space_id AND d.asset_id=o.asset_id AND d.kind='delete' AND d.state NOT IN ('completed','canceled'))))",params![operation,scope.space_id,scope.library_id,serde_json::to_value(scope.provider_type)?.as_str()],|r|r.get(0))?;
        if !valid {bail!("CANCELED");}Ok(())
    })
}

pub fn import_asset(db: &Connection, scope: &Scope, remote: &RemoteAsset) -> Result<String> {
    remote.version.scope.check(scope)?;
    let snapshot = &remote.snapshot;
    if snapshot.library_id != scope.library_id {
        bail!("BACKUP_LIBRARY_MISMATCH");
    }
    let local = repository::bind_remote_asset(db, &scope.space_id, &snapshot.asset_id)?;
    let (previous,deleted,pending):(Option<String>,bool,String)=db.query_row("SELECT baseline_json,remote_deleted,pending_json FROM backup_space_assets WHERE space_id=?1 AND remote_asset_id=?2",params![scope.space_id,snapshot.asset_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let previous = previous
        .map(|v| serde_json::from_str::<AssetSnapshot>(&v))
        .transpose()?;
    if previous
        .as_ref()
        .is_some_and(|p| p.revision > snapshot.revision)
    {
        bail!("BACKUP_RECORD_ROLLBACK");
    }
    let mut pending: PendingChanges = serde_json::from_str(&pending)?;
    // A tombstone must not turn unresolved local versions into a disposable remote projection.
    // These versions stay pending across discovery, restarts and explicit retry captures.
    if snapshot.deleted || pending.recreate {
        let mut q = db.prepare(
            "SELECT value_json FROM backup_conflicts WHERE space_id=?1 AND remote_asset_id=?2",
        )?;
        let values = q
            .query_map(params![scope.space_id, snapshot.asset_id], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for raw in values {
            let conflict: ConflictVersion = serde_json::from_str(&raw)?;
            if !pending.retained_conflicts.contains(&conflict) {
                pending.retained_conflicts.push(conflict);
            }
        }
    }
    let known: Option<i64> = db
        .query_row(
            "SELECT record_revision FROM asset_remote_objects WHERE space_id=?1 AND asset_id=?2",
            params![scope.space_id, local],
            |r| r.get(0),
        )
        .optional()?;
    if known.is_some_and(|r| {
        r as u64 > snapshot.revision
            || deleted && !snapshot.deleted && r as u64 >= snapshot.revision
    }) {
        bail!("BACKUP_RECORD_ROLLBACK");
    }
    if !snapshot.deleted {
        let mut fields = snapshot.fields.clone();
        for (field, edit) in &pending.fields {
            fields.insert(field.clone(), edit.value.clone());
        }
        let source = fields.get("source").cloned().unwrap_or_default();
        let mut metadata = serde_json::Map::from_iter(
            fields
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "source" | "original_filename"))
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        if let Some(source) = source.as_object() {
            metadata.extend(source.clone());
        }
        let media = snapshot
            .media
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("BACKUP_INVALID_RECORD"))?;
        media.validate_scope(scope)?;
        db.execute("UPDATE media_assets SET metadata_json=?2,original_filename=?3,content_sha256=?4,content_bytes=?5 WHERE asset_id=?1",params![local,Value::Object(metadata).to_string(),fields.get("original_filename").and_then(Value::as_str).unwrap_or("video.mp4"),media.sha256,i64::try_from(media.bytes)?])?;
    }
    let pending_raw = serde_json::to_string(&pending)?;
    db.execute("UPDATE backup_space_assets SET pending_json=?7,remote_deleted=?3,cloud_visible=NOT ?3,baseline_json=CASE WHEN ?4 THEN ?5 ELSE baseline_json END,version_json=?6 WHERE space_id=?1 AND remote_asset_id=?2",params![scope.space_id,snapshot.asset_id,snapshot.deleted,pending.is_empty()&&!snapshot.deleted,serde_json::to_string(snapshot)?,serde_json::to_string(&remote.version)?,pending_raw])?;
    if snapshot.deleted {
        db.execute("INSERT INTO backup_tombstones VALUES(?1,?2,?3,?4) ON CONFLICT(space_id,remote_asset_id) DO UPDATE SET record_json=excluded.record_json,version_json=excluded.version_json",params![scope.space_id,snapshot.asset_id,serde_json::to_string(snapshot)?,serde_json::to_string(&remote.version)?])?;
    } else {
        db.execute("INSERT INTO asset_remote_records VALUES(?1,?2,?3) ON CONFLICT(space_id,asset_id) DO UPDATE SET record_json=excluded.record_json",params![scope.space_id,local,serde_json::to_string(snapshot)?])?;
    }
    db.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_revision,remote_state) VALUES(?1,?2,?3,?4) ON CONFLICT(space_id,asset_id) DO UPDATE SET record_revision=excluded.record_revision,remote_state=CASE WHEN remote_state='pending_delete' THEN remote_state ELSE excluded.remote_state END",params![scope.space_id,local,i64::try_from(snapshot.revision)?,if snapshot.deleted {"deleted"} else if pending_raw!="{}"&&!pending.is_empty() {"pending"} else {"backed_up"}])?;
    if snapshot.deleted && !pending.recreate {
        db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_ASSET_DELETED' WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state NOT IN ('completed','canceled')",params![scope.space_id,local])?;
    }
    if !snapshot.deleted {
        db.execute(
            "DELETE FROM backup_tombstones WHERE space_id=?1 AND remote_asset_id=?2",
            params![scope.space_id, snapshot.asset_id],
        )?;
        db.execute(
            "DELETE FROM backup_conflicts WHERE space_id=?1 AND remote_asset_id=?2",
            params![scope.space_id, snapshot.asset_id],
        )?;
    }
    for conflict in snapshot.conflicts.iter().chain(&pending.retained_conflicts) {
        let raw = serde_json::to_string(conflict)?;
        let id = format!("{:x}", Sha256::digest(raw.as_bytes()));
        db.execute(
            "INSERT OR IGNORE INTO backup_conflicts VALUES(?1,?2,?3,?4)",
            params![scope.space_id, snapshot.asset_id, id, raw],
        )?;
    }
    Ok(local)
}

pub async fn run(core: Arc<Core>, cancel: tokio::sync::watch::Receiver<bool>) {
    run_with(core, cancel, Arc::new(super::spaces::provider)).await;
}
pub type ProviderResolver =
    Arc<dyn Fn(&Arc<Core>, &str) -> Result<Arc<dyn BackupProvider>> + Send + Sync>;
pub async fn run_with(
    core: Arc<Core>,
    cancel: tokio::sync::watch::Receiver<bool>,
    resolve: ProviderResolver,
) {
    tokio::join!(
        discovery_loop(core.clone(), cancel.clone(), resolve.clone()),
        queue_loop(core.clone(), cancel.clone(), resolve.clone(), QueueLane::Primary),
        queue_loop(core.clone(), cancel.clone(), resolve.clone(), QueueLane::Upload),
        maintenance_loop(core, cancel, resolve)
    );
}
const FOREGROUND_SYNC_SECONDS: u32 = 30;

async fn pause_until(core: &Core, cancel: &mut tokio::sync::watch::Receiver<bool>, deadline: Instant) -> Option<&'static str> {
    if *cancel.borrow() {
        return None;
    }
    tokio::select! {
        _ = cancel.changed() => None,
        _ = core.backup_window_wake.notified() => Some("window_wake"),
        _ = core.backup_wake.notified() => Some("backup_wake"),
        _ = tokio::time::sleep_until(deadline) => Some("scheduled"),
    }
}
// Only window notifications consult this receipt. A delete batch always goes through discovery.
fn skip_recent_window_wake(core: &Core, space: &str) -> Result<Option<Duration>> {
    core.backup_db(|db| {
        if core.deferred_pulls.has(space) || scheduler::foreground_delete_busy(db, space)? {
            return Ok(None);
        }
        let remaining_ms: Option<u32> = db.query_row(
            "UPDATE backup_window_sync SET last_skipped_ms=CAST(unixepoch('subsec')*1000 AS INTEGER)
             WHERE space_id=?1 AND space_id=(SELECT space_id FROM backup_current_space WHERE id=1)
               AND CAST(unixepoch('subsec')*1000 AS INTEGER)-last_success_ms BETWEEN 0 AND ?2-1
             RETURNING last_success_ms+?2-last_skipped_ms",
            params![space, FOREGROUND_SYNC_SECONDS * 1000], |r| r.get(0),
        ).optional()?;
        Ok(remaining_ms.map(|ms| Duration::from_millis(u64::from(ms))))
    })
}

async fn discovery_loop(
    core: Arc<Core>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    resolve: ProviderResolver,
) {
    let mut failures = 0u32;
    let mut trigger = "startup";
    while !*cancel.borrow() {
        let attempted_space = super::spaces::current_id(&core).ok().flatten();
        let result = async {
            let Some(settings) =
                super::spaces::get(&core)?.filter(|s| s.enabled && !s.connection_pending)
            else {
                return Ok::<_, anyhow::Error>(false);
            };
            if settings.provider_type == ProviderType::Telegram {
                let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
                if binding.session.reconnect.waiting() {
                    wait_backup_network(&core, &settings.space_id, cancel.clone()).await?;
                }
            }
            let replies = core.backup_db(|db| {
                let mut pending = core.deferred_pulls.pending.lock().unwrap();
                pending.retain(|space, _| space == &settings.space_id);
                if scheduler::foreground_delete_busy(db, &settings.space_id)? {
                    pending.entry(settings.space_id.clone()).or_default();
                    return Ok(None);
                }
                let (source, replies) = match pending.remove(&settings.space_id) {
                    Some(replies) => ("post_delete", replies),
                    None => (trigger, Vec::new()),
                };
                Ok(Some((source, replies)))
            })?;
            if let Some((source, replies)) = replies {
                let result = async {
                    let provider = resolve(&core, &settings.space_id)?;
                    let engine = SyncEngine::new(core.clone(), provider);
                    tokio::select! {_=cancel.changed()=>bail!("CANCELED"),result=engine.pull_from(source)=>result}
                }.await;
                for reply in replies {
                    let _ = reply.send(result.as_ref().copied().map_err(|e| anyhow::anyhow!(e.to_string())));
                }
                result?;
            }
            Ok(true)
        }
        .await;
        let delay = match result {
            Ok(active) => {
                failures = 0;
                if !active {
                    2
                } else if core
                    .backup_foreground
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    u64::from(FOREGROUND_SYNC_SECONDS)
                } else {
                    180
                }
            }
            // Stopping the worker is control flow, not a persistent space failure.
            Err(error) if error.to_string() == "CANCELED" => return,
            Err(mut error) => {
                if crate::telegram_reconnect::network_error(&error.to_string()) {
                    if let Some(space) = attempted_space.as_deref().filter(|space|
                        super::spaces::get(&core).ok().flatten().is_some_and(|settings| settings.space_id == *space && settings.provider_type == ProviderType::Telegram)
                    ) {
                        let _ = core.backup_db(|db| {
                            db.execute("INSERT INTO backup_sync_checkpoints(space_id,last_error) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET last_error=excluded.last_error", params![space,error.to_string()])?;
                            Ok(())
                        });
                        match wait_backup_network(&core, space, cancel.clone()).await {
                            Ok(()) => { failures = 0; trigger = "reconnected"; continue; }
                            Err(_) if *cancel.borrow() => return,
                            Err(error) if error.to_string() == "BACKUP_SPACE_INACTIVE" => { failures = 0; continue; }
                            Err(recovery_error) => error = recovery_error,
                        }
                    }
                }
                failures = (failures + 1).min(8);
                if let Some(space) = &attempted_space {
                    let _=core.backup_db(|db|{db.execute("INSERT INTO backup_sync_checkpoints(space_id,last_error) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET last_error=excluded.last_error",params![space,error.to_string()])?;Ok(())});
                }
                if error.to_string() == "BACKUP_SPACE_INACTIVE" {
                    0
                } else {
                    (2u64.pow(failures)).min(300)
                }
            }
        };
        let mut deadline = Instant::now() + Duration::from_secs(delay);
        loop {
            tokio::select! {
                wake = pause_until(&core, &mut cancel, deadline) => {
                    let Some(source) = wake else { return };
                    if let ("window_wake", Some(space)) = (source, attempted_space.as_deref()) {
                        match skip_recent_window_wake(&core, space) {
                            Ok(Some(remaining)) => {
                                // A return from background can shorten the timer. Repeated wakes cannot extend it.
                                deadline = deadline.min(Instant::now() + remaining);
                                continue;
                            }
                            Err(error) => eprintln!("backup window wake freshness: {error}"),
                            Ok(None) => {}
                        }
                    }
                    trigger = source;
                    break;
                }
                _ = core.deferred_pulls.finished.notified() => {
                    // A normal wake may already have consumed this batch's catch-up.
                    if attempted_space.as_deref().is_some_and(|space| core.deferred_pulls.has(space)) {
                        trigger = "backup_wake";
                        break;
                    }
                }
            }
        }
    }
}
const CLAIM_OPERATION_SQL: &str = "UPDATE backup_operations SET state='running',last_error=NULL,next_retry_at=NULL,
    payload_json=CASE WHEN kind='delete' AND attempt=0 THEN json_set(
        json_insert(payload_json,'$.first_claimed_ms',CAST(unixepoch('subsec')*1000 AS INTEGER)),
        CASE WHEN COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)=1 THEN '$.timings.maintenance.queue' ELSE '$.timings.foreground.queue' END,
        json_object('elapsed_ms',max(0,CAST(unixepoch('subsec')*1000 AS INTEGER)-COALESCE(json_extract(payload_json,'$.queued_at_ms'),created_at*1000)),
                    'outcome','ok','at',unixepoch(),'attempt',1))
        ELSE payload_json END,attempt=attempt+1 WHERE operation_id=? AND state IN ('pending','retry_wait')
        AND (next_retry_at IS NULL OR next_retry_at<=unixepoch())";
// Proxy reconfiguration is not a failed attempt. Retain all publication evidence
// and let the same operation reconcile an uncertain send before continuing.
// TASK_BUSY here is only local finalization after a confirmed remote delete.
// Keep the phase and all ownership evidence; no publish/upload is replayed.
const TASK_BUSY_RETRY_SQL: &str = "UPDATE backup_operations SET state='retry_wait',last_error=NULL,
    next_retry_at=unixepoch()+1,attempt=MAX(0,attempt-1)
    WHERE operation_id=? AND state='running' AND kind='delete' AND phase IN ('local_cleanup','finalizing')";

const NETWORK_RETRY_SQL: &str = "UPDATE backup_operations SET state='retry_wait',last_error=NULL,
    attempt=max(0,attempt-1),next_retry_at=unixepoch()+2
    WHERE operation_id=? AND state='running'";
// Cleanup is maintenance, never a prerequisite for unrelated foreground work.
async fn cleanup_one_upload(
    engine: &SyncEngine,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool> {
    if *cancel.borrow() {
        bail!("CANCELED");
    }
    engine.check_space()?;
    let space = &engine.provider.scope().space_id;
    if engine
        .core
        .backup_db(|db| Ok(scheduler::foreground_delete_busy(db, space)? || scheduler::upload_running(db, space)?))?
    {
        return Ok(false);
    }
    // Cleanup can physically delete owned temporary media. Keep it exclusive
    // with upload/file work, without waiting behind an active foreground job.
    // A second lane may claim an upload while we clean: it then waits here,
    // remaining observable by the preemption check until cleanup is dropped.
    let Ok(_writer) = engine.core.backup_write_lock.try_write() else {
        return Ok(false);
    };
    let Some(op) = scheduler::uploads_to_clean(&engine.core, space)?
        .into_iter()
        .next()
    else {
        return Ok(false);
    };
    let work = async {
        for kind in ["video", "cover", "transcript"] {
            if let Some(state) = op.payload.get(format!("resume_{kind}")).cloned() {
                engine
                    .provider
                    .cancel_upload(serde_json::from_value(state)?, &op.operation_id)
                    .await?;
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            _ = cancel.changed() => return Err(anyhow::anyhow!("CANCELED")),
            result = &mut work => break result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                engine.check_space()?;
                let (runnable, deleting, uploading) = engine.core.backup_db(|db| Ok((
                    scheduler::next_sync_operation(db, space)?.is_some(),
                    scheduler::foreground_delete_busy(db, space)?,
                    scheduler::upload_running(db, space)?,
                )))?;
                if runnable || deleting || uploading { return Ok(runnable); }
            }
        }
    };
    match result {
        Ok(()) => scheduler::finish_upload_cleanup(&engine.core, &op.operation_id)?,
        Err(error) if *cancel.borrow() || engine.check_space().is_err() => return Err(error),
        Err(error) if engine.provider.scope().provider_type == ProviderType::Telegram && crate::telegram_reconnect::network_error(&error.to_string()) => {
            return Err(error);
        }
        Err(error) => {
            scheduler::defer_upload_cleanup(&engine.core, &op, &error.to_string())?;
            eprintln!("backup cleanup operation={}: {error}", op.operation_id);
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum QueueLane {
    Primary,
    Upload,
}

// One primary lane handles uploads, deletes and cleanup. The second lane only
// uploads, so cleanup is never executed twice and no per-operation task escapes.
async fn queue_loop(
    core: Arc<Core>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    resolve: ProviderResolver,
    lane: QueueLane,
) {
    while !*cancel.borrow() {
        let mut queue_space = None;
        let result=async {
            settings_cleanup(&core)?;
            if !super::spaces::enabled(&core)? {return Ok::<_,anyhow::Error>(false);}
            let Some(mut settings)=super::spaces::get(&core)? else {return Ok(false);};
            queue_space = Some(settings.space_id.clone());
            if settings.provider_type == ProviderType::Telegram {
                let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
                if binding.session.reconnect.waiting() {
                    wait_backup_network(&core, &settings.space_id, cancel.clone()).await?;
                    settings = super::spaces::get(&core)?.filter(|s| s.enabled).context("BACKUP_SPACE_INACTIVE")?;
                    queue_space = Some(settings.space_id.clone());
                }
                recover_network_operations(&core, &settings.space_id)?;
            }
            if settings.connection_pending || !settings.enabled {
                super::spaces::prepare_current(&core).await?;
                settings=super::spaces::get(&core)?.context("BACKUP_SPACE_NOT_FOUND")?;
                queue_space = Some(settings.space_id.clone());
            }
            if !settings.enabled {return Ok(false);}
            let provider=resolve(&core,&settings.space_id)?;
            let engine=SyncEngine::new(core.clone(),provider);
            // Canceled uploads retain their exact private resume state until own-session cleanup succeeds.
            let foreground_busy = || core.backup_db(|db| scheduler::foreground_delete_busy(db, &settings.space_id));
            if !foreground_busy()? {
                core.register_backup_history(&settings.space_id)?;
                engine.enqueue()?;
                scheduler::park_upload_cleanup_dependencies(&core, &settings.space_id)?;
            }
            // Select and claim under one DB lock. A delete can arrive while upload discovery
            // or canceled-upload cleanup is running.
            let claim=core.backup_db(|db|{
                let next = match lane {
                    QueueLane::Primary => scheduler::next_sync_operation(db, &settings.space_id)?,
                    QueueLane::Upload => scheduler::next_upload_operation(db, &settings.space_id)?,
                };
                let Some((operation,foreground_delete))=next else {return Ok(None);};
                if db.execute(CLAIM_OPERATION_SQL,[&operation])? == 0 {return Ok(None);}
                if foreground_delete { core.deferred_pulls.mark(&settings.space_id); }
                Ok(Some((operation,foreground_delete)))
            })?;
            let Some((operation,foreground_delete))=claim else {
                return match lane {
                    QueueLane::Primary => cleanup_one_upload(&engine, &mut cancel).await,
                    QueueLane::Upload => Ok(false),
                };
            };
            let kind:String=core.backup_db(|db|Ok(db.query_row("SELECT kind FROM backup_operations WHERE operation_id=?",[&operation],|r|r.get(0))?))?;
            if kind == "delete" {
                eprintln!("backup_delete operation={operation} phase=queue_claim at_ms={} foreground={foreground_delete}", chrono::Utc::now().timestamp_millis());
            }
            let work=async {if kind=="delete" {super::sync_delete::execute(&core,engine.provider.as_ref(),&operation).await} else {engine.execute_upload(&operation).await}};tokio::pin!(work);
            let result=loop {
                tokio::select! {
                    _=cancel.changed()=>break Err(anyhow::anyhow!("CANCELED")),
                    result=&mut work=>break result,
                    _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{if let Err(error)=engine.check(&operation) {break Err(error);}}
                }
            };
            if let Err(error)=result {
                let op=scheduler::operation(&core,&operation)?;
                let reason=error.to_string();
                core.backup_db(|db|{
                    if reason == "TASK_BUSY" && db.execute(TASK_BUSY_RETRY_SQL, [&operation])? != 0 {
                        return Ok(());
                    }
                    if engine.provider.scope().provider_type==ProviderType::Telegram && (reason=="TELEGRAM_NETWORK_CHANGED" || crate::telegram_reconnect::network_error(&reason)) {
                        db.execute(NETWORK_RETRY_SQL, [&operation])?;
                        return Ok(());
                    }
                    let state=if op.attempt>=5 || (engine.provider.scope().provider_type==ProviderType::Telegram && reason=="BACKUP_CHECKSUM_MISMATCH") || matches!(reason.as_str(),"BACKUP_ACCESS_DENIED"|"BACKUP_ASSET_DELETED"|"BACKUP_SOURCE_CHANGED"|"BACKUP_INVALID_OPERATION"|"BACKUP_REUPLOAD_STALE"|"BACKUP_DELETE_UNSUPPORTED"|"BACKUP_MEDIA_IN_USE"|"BACKUP_TOMBSTONE_REQUIRED"|"TELEGRAM_MEDIA_CHANGED"|"TELEGRAM_MEDIA_MISSING"|"TELEGRAM_AUTH_REQUIRED"|"TELEGRAM_ACCOUNT_MISMATCH"|"TELEGRAM_CHANNEL_FORBIDDEN"|"TELEGRAM_INDEX_INVALID"|"TELEGRAM_INDEX_CHANGED"|"TELEGRAM_OPERATION_MISMATCH"|"TELEGRAM_FILE_TOO_LARGE") {"blocked"} else {"retry_wait"};
                    db.execute("UPDATE backup_operations SET state=?2,last_error=?3,next_retry_at=unixepoch()+?4 WHERE operation_id=?1 AND state='running'",params![operation,state,reason,reason.strip_prefix("TELEGRAM_FLOOD_WAIT:").and_then(|v|v.parse::<u32>().ok()).map(i64::from).unwrap_or_else(||(2i64.pow(op.attempt.min(8) as u32)).min(300))])?;
                    Ok(())
                })?;
                if engine.provider.scope().provider_type == ProviderType::Telegram && crate::telegram_reconnect::network_error(&reason) {
                    core.backup_db(|db| { db.execute("UPDATE backup_operations SET last_error='TELEGRAM_RECONNECTING' WHERE operation_id=? AND state='retry_wait'", [&operation])?; Ok(()) })?;
                    wait_backup_network(&core, &settings.space_id, cancel.clone()).await?;
                }
            }
            if foreground_delete && !core.backup_db(|db| scheduler::foreground_delete_busy(db, &settings.space_id))? {
                core.deferred_pulls.finished.notify_one();
            }
            Ok(true)
        }.await;
        // A handled failure has its own retry deadline. Drain other runnable work immediately.
        if matches!(result, Ok(true)) {
            continue;
        }
        if result.as_ref().is_err_and(|e| crate::telegram_reconnect::network_error(&e.to_string()))
            && super::spaces::get(&core).ok().flatten().is_some_and(|s| s.provider_type == ProviderType::Telegram) {
            if let Some(space) = queue_space.as_deref() {
                let _ = wait_backup_network(&core, space, cancel.clone()).await;
            }
        }
        let connection_error = result.is_err()
            && super::spaces::get(&core)
                .ok()
                .flatten()
                .is_some_and(|s| s.connection_pending);
        if let Err(error) = result {
            if let Some(space) = queue_space.or_else(|| super::spaces::current_id(&core).ok().flatten()) {
                core.deferred_pulls.fail_waiters(&space, &error);
            }
            if error.to_string() != "CANCELED" {
                eprintln!("backup sync: {error}");
            }
        }
        tokio::select! {
            _ = core.delete_queue_wake.notified() => {},
            _ = pause_until(&core, &mut cancel, Instant::now() + Duration::from_secs(if connection_error { 60 } else { 2 })) => {},
        }
    }
}
async fn wait_backup_network(core: &Core, space: &str, cancel: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let work = crate::telegram_accounts::recover_network(core, crate::telegram_accounts::Purpose::Backup, cancel);
    tokio::pin!(work);
    loop {
        if !super::spaces::enabled(core)? || super::spaces::current_id(core)?.as_deref() != Some(space) { bail!("BACKUP_SPACE_INACTIVE"); }
        tokio::select! {
            result = &mut work => return result,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

const RECOVER_NETWORK_OPERATIONS_SQL: &str = "UPDATE backup_operations SET attempt=CASE WHEN state='blocked' THEN 0 ELSE attempt END,state='retry_wait',next_retry_at=NULL,last_error=NULL
            WHERE space_id=? AND kind IN ('upload','delete') AND state IN ('blocked','retry_wait') AND phase<>'superseded'
            AND last_error IN ('TELEGRAM_CONNECTION_ERROR','TELEGRAM_TIMEOUT','TELEGRAM_NOT_CONNECTED','TELEGRAM_TRANSFER_TIMEOUT','BACKUP_NETWORK_ERROR','TELEGRAM_RECONNECTING')";

fn recover_network_operations(core: &Core, space: &str) -> Result<()> {
    core.backup_db(|db| {
        db.execute(RECOVER_NETWORK_OPERATIONS_SQL, [space])?;
        db.execute("UPDATE backup_operations SET payload_json=json_remove(payload_json,'$.cleanup_blocked','$.cleanup_attempt','$.cleanup_error'),next_retry_at=NULL
            WHERE space_id=? AND kind='upload' AND (state='canceled' OR state='blocked' AND last_error='BACKUP_ASSET_DELETED')
            AND json_extract(payload_json,'$.cleanup_error') IN ('TELEGRAM_CONNECTION_ERROR','TELEGRAM_TIMEOUT','TELEGRAM_NOT_CONNECTED','TELEGRAM_TRANSFER_TIMEOUT','BACKUP_NETWORK_ERROR')", [space])?;
        Ok(())
    })
}

fn settings_cleanup(core: &Core) -> Result<()> {
    super::settings::cleanup_credentials(core, &super::settings::SystemVault)
}

pub(super) async fn scoped<T>(
    engine: &SyncEngine,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    if *cancel.borrow() {
        bail!("CANCELED");
    }
    tokio::pin!(work);
    loop {
        tokio::select! {
            result=&mut work=>return result,_=cancel.changed()=>bail!("CANCELED"),
            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>engine.check_space()?,
        }
    }
}
pub async fn maintenance_once(
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    work: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    if *cancel.borrow() {
        bail!("CANCELED");
    }
    tokio::select! {_=cancel.changed()=>bail!("CANCELED"),result=work=>result}
}
async fn maintenance_loop(
    core: Arc<Core>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    resolve: ProviderResolver,
) {
    while !*cancel.borrow() {
        if let Err(error) = maintenance_once(&mut cancel, async {
            super::spaces::cleanup_probes(&core).await?;
            super::sync_delete::compact_current(&core, resolve.clone()).await
        })
        .await
        {
            if error.to_string() != "CANCELED" {
                eprintln!("backup probe maintenance: {error}");
            }
        }
        pause_until(&core, &mut cancel, Instant::now() + Duration::from_secs(30)).await;
    }
}

pub fn request_upload(core: &Arc<Core>, space: &str, local: &str) -> Result<()> {
    let provider = super::spaces::provider(core, space)?;
    request_upload_with(core, provider, local)
}
pub fn request_upload_with(
    core: &Arc<Core>,
    provider: Arc<dyn BackupProvider>,
    local: &str,
) -> Result<()> {
    let engine = SyncEngine::new(core.clone(), provider);
    engine.check_space()?;
    let space = &engine.provider.scope().space_id;
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let db=&tx;
        let member:Option<bool>=db.query_row("SELECT remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|r.get(0)).optional()?;
        let deleted = if let Some(deleted) = member { deleted } else {
            let asset=repository::list_assets(db)?.into_iter().find(|a|a.asset_id==local).context("BACKUP_ASSET_NOT_FOUND")?;
            if asset.asset_state != "active" { bail!("BACKUP_ASSET_DELETED"); }
            if !asset.path.as_ref().is_some_and(|p|p.is_file()) { bail!("FILE_MISSING"); }
            // Only this explicit upload request joins an existing video to another target.
            join_asset(db, space, &asset)?;
            false
        };
        let deleting:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state NOT IN ('completed','canceled'))",params![space,local],|r|r.get(0))?;
        if deleted {
            let asset=repository::list_assets(db)?.into_iter().find(|a|a.asset_id==local).context("BACKUP_ASSET_NOT_FOUND")?;
            if asset.path.as_ref().is_none_or(|p|!p.is_file()) {bail!("FILE_MISSING");}
            let (revision,raw):(u64,String)=db.query_row("SELECT local_revision,pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|Ok((r.get::<_,i64>(0)? as u64+1,r.get(1)?)))?;
            let mut pending:PendingChanges=serde_json::from_str(&raw)?;
            for (name,value) in fields(&asset) {pending.fields.entry(name).or_insert(FieldEdit{value,revision,base:FieldBaseline::Missing});}
            let tombstone:String=db.query_row("SELECT record_json FROM backup_tombstones WHERE space_id=?1 AND remote_asset_id=(SELECT remote_asset_id FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2)",params![space,local],|r|r.get(0))?;
            pending.recreate_revision=Some(serde_json::from_str::<AssetSnapshot>(&tombstone)?.revision);
            pending.recreate=true;pending.media=Some(revision);pending.cover=Some(revision);pending.transcript=Some(revision);
            db.execute("UPDATE backup_operations SET state=CASE WHEN kind='upload' THEN 'canceled' ELSE 'completed' END,phase='superseded',last_error=NULL WHERE space_id=?1 AND asset_id=?2 AND state NOT IN ('completed','canceled')",params![space,local])?;
            // Canceled operations keep their owned cleanup state but do not suppress the new request.
            db.execute("UPDATE backup_operations SET state='completed' WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state='canceled' AND phase='cancel_cleaned'",params![space,local])?;
            db.execute("UPDATE backup_space_assets SET cloud_visible=1,local_revision=?3,pending_json=?4 WHERE space_id=?1 AND local_asset_id=?2",params![space,local,i64::try_from(revision)?,serde_json::to_string(&pending)?])?;
        } else if deleting {bail!("BACKUP_ASSET_DELETED");}
        if !deleted {
            let raw:String=db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|r.get(0))?;
            let mut pending:PendingChanges=serde_json::from_str(&raw)?;
            if pending.recreate {
                pending.recreate=false;pending.recreate_revision=None;
                db.execute("UPDATE backup_space_assets SET pending_json=?3 WHERE space_id=?1 AND local_asset_id=?2",params![space,local,serde_json::to_string(&pending)?])?;
            }
        }
        if engine.provider.scope().provider_type==ProviderType::Telegram {
            // Cleanup can have removed confirmed messages: a resumed request needs fresh send identities.
            db.execute("UPDATE backup_operations SET state=CASE WHEN phase='cancel_cleaned' THEN 'completed' ELSE state END,phase=CASE WHEN phase='cancel_cleaned' THEN phase ELSE 'superseded' END WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state='canceled'",params![space,local])?;
        }
        // An explicit retry also retries owned cleanup, retaining every send identity and receipt.
        db.execute("UPDATE backup_operations SET next_retry_at=NULL,
            payload_json=json_remove(payload_json,'$.cleanup_blocked','$.cleanup_attempt','$.cleanup_error')
            WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state='canceled' AND phase<>'cancel_cleaned'",
            params![space,local])?;
        // A changed incarnation cannot resume the previous immutable object path.
        // Keep its private upload state for owned-session cleanup, then capture a fresh operation.
        db.execute("UPDATE backup_operations SET state='canceled',phase='superseded',last_error=NULL WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state IN ('blocked','retry_wait') AND last_error IN ('BACKUP_REUPLOAD_STALE','BACKUP_SOURCE_CHANGED') AND json_extract(payload_json,'$.scope.protocol')=2",params![space,local])?;
        db.execute("UPDATE backup_operations SET state='pending',last_error=NULL,next_retry_at=NULL,attempt=0 WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state IN ('canceled','blocked','retry_wait') AND phase<>'superseded' AND json_extract(payload_json,'$.scope.protocol')=2",params![space,local])?;
        db.execute("DELETE FROM asset_deletion_hidden_history WHERE space_id=?1 AND asset_id=?2",params![space,local])?;
        tx.commit()?;Ok(())
    })?;
    engine.enqueue()?;
    core.wake_backup();
    Ok(())
}

#[cfg(test)]
mod claim_timing_tests {
    use super::*;
    #[test]
    fn reconnect_recovers_only_network_operations_and_preserves_send_evidence() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE backup_operations(operation_id TEXT PRIMARY KEY,space_id TEXT,kind TEXT,state TEXT,phase TEXT,attempt INTEGER,payload_json TEXT,next_retry_at INTEGER,last_error TEXT)").unwrap();
        let evidence = json!({"video_receipt":{"message_id":123},"resume_video":{"phase":"Sending","random_id":42}}).to_string();
        for (id, space, kind, state, phase, error) in [
            ("retry", "active", "upload", "blocked", "publishing", "TELEGRAM_CONNECTION_ERROR"),
            ("delete", "active", "delete", "retry_wait", "objects", "TELEGRAM_RECONNECTING"),
            ("canceled", "active", "upload", "canceled", "captured", "TELEGRAM_CONNECTION_ERROR"),
            ("uncertain", "active", "upload", "blocked", "publishing", "TELEGRAM_SEND_UNCONFIRMED"),
            ("other", "other", "upload", "blocked", "publishing", "TELEGRAM_CONNECTION_ERROR"),
            ("stale", "active", "upload", "blocked", "superseded", "TELEGRAM_CONNECTION_ERROR"),
            ("done", "active", "upload", "completed", "completed", "TELEGRAM_CONNECTION_ERROR"),
            ("restore", "active", "restore", "blocked", "downloading", "TELEGRAM_CONNECTION_ERROR"),
        ] {
            db.execute("INSERT INTO backup_operations VALUES(?1,?2,?3,?4,?5,9,?6,9999999999,?7)", params![id,space,kind,state,phase,evidence,error]).unwrap();
        }
        assert_eq!(db.execute(RECOVER_NETWORK_OPERATIONS_SQL, ["active"]).unwrap(), 2);
        for id in ["retry", "delete"] {
            let (state, payload, attempt, error): (String,String,i64,Option<String>) = db.query_row("SELECT state,payload_json,attempt,last_error FROM backup_operations WHERE operation_id=?", [id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!((state,payload,attempt,error), ("retry_wait".into(),evidence.clone(),if id == "retry" {0} else {9},None));
        }
        assert_eq!(db.query_row("SELECT count(*) FROM backup_operations WHERE attempt=9", [], |r|r.get::<_,i64>(0)).unwrap(), 7);
    }

    #[test]
    fn maintenance_retry_preserves_foreground_queue_timing_and_upload_payload() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE backup_operations(operation_id TEXT PRIMARY KEY,kind TEXT,state TEXT,attempt INTEGER,payload_json TEXT,created_at INTEGER,next_retry_at INTEGER,last_error TEXT)").unwrap();
        let original = serde_json::json!({"local_cleanup_done":true,"timings":{"foreground":{"queue":{"elapsed_ms":1234}}}});
        db.execute("INSERT INTO backup_operations VALUES('maintenance','delete','retry_wait',0,?1,unixepoch()-10,NULL,NULL)",[original.to_string()]).unwrap();
        db.execute(CLAIM_OPERATION_SQL, ["maintenance"]).unwrap();
        let (state,attempt,payload):(String,i64,String)=db.query_row("SELECT state,attempt,payload_json FROM backup_operations WHERE operation_id='maintenance'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(state, "running");
        assert_eq!(attempt, 1);
        assert_eq!(
            payload["timings"]["foreground"]["queue"],
            original["timings"]["foreground"]["queue"]
        );
        assert!(
            payload["timings"]["maintenance"]["queue"]["elapsed_ms"]
                .as_u64()
                .unwrap()
                >= 10000
        );
        db.execute(
            "INSERT INTO backup_operations VALUES('upload','upload','pending',0,'{}',unixepoch(),NULL,NULL)",
            [],
        )
        .unwrap();
        db.execute(CLAIM_OPERATION_SQL, ["upload"]).unwrap();
        let raw: String = db
            .query_row(
                "SELECT payload_json FROM backup_operations WHERE operation_id='upload'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, "{}");
    }

    #[test]
    fn task_busy_retry_keeps_delete_phase_evidence_and_cancellation() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE backup_operations(operation_id TEXT PRIMARY KEY,kind TEXT,state TEXT,phase TEXT,attempt INTEGER,payload_json TEXT,created_at INTEGER,next_retry_at INTEGER,last_error TEXT)").unwrap();
        let evidence = json!({"scope":{"account_id":"test"},"remote_asset_id":"original-generation","archive_revision":7,"local_cleanup_done":true,"video_receipt":{"message_id":123}}).to_string();
        db.execute("INSERT INTO backup_operations VALUES('original','delete','running','finalizing',5,?1,unixepoch(),NULL,'TASK_BUSY')", [&evidence]).unwrap();
        for _ in 0..8 {
            assert_eq!(db.execute(TASK_BUSY_RETRY_SQL, ["original"]).unwrap(), 1);
            let (attempt, error, payload, phase): (i64, Option<String>, String, String) = db
                .query_row(
                    "SELECT attempt,last_error,payload_json,phase FROM backup_operations",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!(
                (attempt, error, payload, phase),
                (4, None, evidence.clone(), "finalizing".into())
            );
            assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["original"]).unwrap(), 0);
            db.execute(
                "UPDATE backup_operations SET next_retry_at=unixepoch()-1",
                [],
            )
            .unwrap();
            assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["original"]).unwrap(), 1);
        }
        for state in ["canceled", "blocked", "completed"] {
            db.execute("UPDATE backup_operations SET state=?", [state])
                .unwrap();
            assert_eq!(db.execute(TASK_BUSY_RETRY_SQL, ["original"]).unwrap(), 0);
            assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["original"]).unwrap(), 0);
        }
        for (kind, phase) in [
            ("upload", "finalizing"),
            ("delete", "tombstone"),
            ("delete", "objects"),
        ] {
            db.execute(
                "UPDATE backup_operations SET state='running',kind=?1,phase=?2",
                params![kind, phase],
            )
            .unwrap();
            assert_eq!(db.execute(TASK_BUSY_RETRY_SQL, ["original"]).unwrap(), 0);
        }
    }

    #[test]
    fn proxy_retry_preserves_evidence_budget_and_user_cancellation() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE backup_operations(operation_id TEXT PRIMARY KEY,kind TEXT,state TEXT,attempt INTEGER,payload_json TEXT,created_at INTEGER,next_retry_at INTEGER,last_error TEXT)").unwrap();
        let evidence = json!({"video_receipt":{"message_id":123},"capture":{"operation_id":"same-send"}}).to_string();
        db.execute("INSERT INTO backup_operations VALUES('same-send','upload','running',5,?1,unixepoch(),NULL,'TELEGRAM_TRANSFER_CANCELED')", [&evidence]).unwrap();
        for _ in 0..6 {
            assert_eq!(db.execute(NETWORK_RETRY_SQL, ["same-send"]).unwrap(), 1);
            let (attempt, error, payload): (i64, Option<String>, String) = db.query_row("SELECT attempt,last_error,payload_json FROM backup_operations", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(attempt, 4);
            assert_eq!(error, None);
            assert_eq!(payload, evidence);
            assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["same-send"]).unwrap(), 0, "respect reconnect delay");
            db.execute("UPDATE backup_operations SET next_retry_at=unixepoch()-1,last_error='TELEGRAM_TRANSFER_CANCELED'", []).unwrap();
            assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["same-send"]).unwrap(), 1);
            let (attempt, error, deadline): (i64, Option<String>, Option<i64>) = db.query_row("SELECT attempt,last_error,next_retry_at FROM backup_operations", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!((attempt,error,deadline), (5,None,None));
        }
        db.execute("UPDATE backup_operations SET state='canceled'", []).unwrap();
        assert_eq!(db.execute(NETWORK_RETRY_SQL, ["same-send"]).unwrap(), 0);
        assert_eq!(db.execute(CLAIM_OPERATION_SQL, ["same-send"]).unwrap(), 0);
    }
}
