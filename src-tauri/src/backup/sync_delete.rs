//! Durable current-space deletion; receiving devices never delete local files or remote objects.
use super::{
    lifecycle,
    provider::*,
    repository, scheduler, spaces,
    sync_engine::{self, PendingChanges},
    sync_read,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::sync::Arc;

fn scope(core: &Core, space: &str) -> Result<Scope> {
    let current = spaces::get(core)?
        .filter(|s| s.space_id == space && s.enabled)
        .context("BACKUP_SPACE_INACTIVE")?;
    current.provider_type.require_implemented()?;
    Ok(Scope {
        space_id: space.into(),
        library_id: current.library_id,
        provider_type: current.provider_type,
        protocol: 2,
    })
}
pub fn intent(core: &Core, space: &str, local: &str) -> Result<String> {
    let received_ms = chrono::Utc::now().timestamp_millis();
    let scope = scope(core, space)?;
    if scope.provider_type == ProviderType::Telegram
        && !spaces::config(core, space)?
            .capabilities
            .is_some_and(|c| c.can_delete_owned_media)
    {
        bail!("BACKUP_DELETE_UNSUPPORTED");
    }
    let base: Option<AssetSnapshot> = if scope.provider_type == ProviderType::Telegram {
        core.backup_db(|db| {
        let raw:Option<String>=db.query_row("SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",params![space,local],|r|r.get(0)).optional()?;
        raw.map(|raw|serde_json::from_str(&raw).map_err(anyhow::Error::from)).transpose()
    })?
    } else {
        None
    };
    let remote = sync_read::remote_id(core, space, local)?;
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let previous:Option<(String,String,String)>=tx.query_row("SELECT operation_id,state,payload_json FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state<>'canceled' ORDER BY CASE WHEN state='completed' THEN 1 ELSE 0 END,rowid DESC LIMIT 1",params![space,local],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id,state,raw))=previous.filter(|(_,state,_)|!(scope.provider_type==ProviderType::Telegram && state=="completed")) {
            let mut payload:Value=serde_json::from_str(&raw)?;
            if let Some(saved)=payload.get("scope") {scope.check(&serde_json::from_value(saved.clone())?)?;}
            // A duplicate click observes the original request, including its backoff or parked successor.
            // Explicit recovery remains available through the queue retry command.
            if scope.provider_type == ProviderType::Telegram && matches!(state.as_str(), "pending" | "running" | "retry_wait") {
                if payload["remote_asset_id"] != remote { bail!("BACKUP_SCOPE_MISMATCH"); }
                return Ok(id);
            }
            if state=="completed" {payload["cleanup_pass"]=0.into();payload.as_object_mut().context("BACKUP_INVALID_OPERATION")?.remove("local_cleanup_done");}
            payload["scope"]=serde_json::to_value(&scope)?;payload["remote_asset_id"]=remote.clone().into();
            tx.execute("UPDATE backup_operations SET state=CASE WHEN state='running' THEN state ELSE 'pending' END,phase=CASE WHEN ?2 THEN 'tombstone' ELSE phase END,next_retry_at=NULL,last_error=NULL,attempt=CASE WHEN state='running' THEN attempt ELSE 0 END,payload_json=?3 WHERE operation_id=?1",params![id,state=="completed",payload.to_string()])?;
        } else {
            let mut payload = json!({"scope":scope,"remote_asset_id":remote,"cleanup_pass":0,"base":base});
            if scope.provider_type == ProviderType::Telegram {
                payload["first_archive_publish"] = true.into();
                payload["command_received_ms"] = received_ms.into();
                let queued: i64 = tx.query_row("SELECT CAST(unixepoch('subsec')*1000 AS INTEGER)", [], |r| r.get(0))?;
                payload["queued_at_ms"] = queued.into();
            }
            tx.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'delete','pending','tombstone',?4)",params![uuid::Uuid::new_v4().to_string(),space,local,payload.to_string()])?;
        }
        let operation:String=tx.query_row("SELECT operation_id FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state NOT IN ('completed','canceled')",params![space,local],|r|r.get(0))?;
        tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_DELETE_PENDING' WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state NOT IN ('completed','canceled')",params![space,local])?;
        tx.execute("INSERT INTO asset_remote_objects(space_id,asset_id,remote_state) VALUES(?1,?2,'pending_delete') ON CONFLICT(space_id,asset_id) DO UPDATE SET remote_state='pending_delete'",params![space,local])?;
        tx.commit()?;Ok(operation)
    })
}
pub async fn delete(core: &Arc<Core>, space: &str, local: &str) -> Result<String> {
    let operation = intent(core, space, local)?;
    eprintln!(
        "backup_delete operation={operation} phase=intent_committed at_ms={}",
        chrono::Utc::now().timestamp_millis()
    );
    core.delete_queue_wake.notify_one();
    core.cancel_asset_restore(local)?;
    core.cancel_asset_transcription(local).await?;
    Ok(operation)
}

// The caller owns the asset file lease; remote cleanup was confirmed before this phase.
async fn finish_local(core: &Arc<Core>, op: &scheduler::Operation) -> Result<()> {
    let local = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    lifecycle::remove_local(core, local).await?;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET phase='finalizing',last_error=NULL WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })?;
    let _stage = super::delete_timing::activity("finalizing");
    core.finish_asset_deletion(local, &op.operation_id)
}

// Consume the first-run marker before network I/O. Requeueing (even after a crash before send)
// cannot re-enable it; pre-upgrade pending operations conservatively use a fresh download.
fn claim_archive_read(db: &rusqlite::Connection, operation: &str) -> Result<bool> {
    Ok(db.execute(
        "UPDATE backup_operations SET payload_json=json_remove(payload_json,'$.first_archive_publish')
         WHERE operation_id=? AND phase='tombstone' AND attempt=1 AND json_extract(payload_json,'$.first_archive_publish')=1",
        [operation],
    )? == 1)
}

// Only pair with an already queued fresh delete. A parked operation remains ordinary due work
// if its successor fails, is delayed, or the process restarts.
pub(super) fn park_confirmed_archive(
    db: &rusqlite::Connection,
    space: &str,
    operation: &str,
    revision: u64,
) -> Result<bool> {
    let tx = db.unchecked_transaction()?;
    let next: Option<String> = tx.query_row(
        "SELECT operation_id FROM backup_operations WHERE space_id=? AND kind='delete' AND state='pending'
         AND phase='tombstone' AND attempt=0 AND operation_id<>? ORDER BY created_at,rowid LIMIT 1",
        params![space,operation], |row| row.get(0),
    ).optional()?;
    let Some(next) = next else { return Ok(false) };
    let parked = tx.execute(
        "UPDATE backup_operations SET state='pending',phase='archive_ready',next_retry_at=NULL,
         payload_json=json_set(payload_json,'$.archive_next',?3,'$.archive_revision',?4,
           '$.archive_confirmed_ms',CAST(unixepoch('subsec')*1000 AS INTEGER))
         WHERE space_id=?1 AND operation_id=?2 AND phase='objects' AND state='running' AND attempt=1
         AND json_extract(payload_json,'$.archive_revision') IS NULL AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)=0
         AND NOT EXISTS (SELECT 1 FROM backup_operations WHERE space_id=?1 AND phase='archive_ready' AND state='pending')",
        params![space,operation,next,i64::try_from(revision)?],
    )? != 0;
    tx.commit()?;
    Ok(parked)
}

pub(super) struct ParkedArchive {
    pub operation: String,
    pub asset: String,
    pub revision: i64,
}

pub(super) fn parked_archive(
    db: &rusqlite::Connection,
    space: &str,
    successor: &str,
) -> Result<Option<ParkedArchive>> {
    Ok(db.query_row("SELECT operation_id,json_extract(payload_json,'$.remote_asset_id'),json_extract(payload_json,'$.archive_revision')
        FROM backup_operations WHERE space_id=?1 AND kind='delete' AND phase='archive_ready' AND state='pending'
        AND json_extract(payload_json,'$.archive_next')=?2", params![space, successor],
        |r| Ok(ParkedArchive {operation:r.get(0)?,asset:r.get(1)?,revision:r.get(2)?})).optional()?)
}

// A failed ownership check on the parked asset cannot block its healthy successor.
pub(super) fn recover_cleanup(
    core: &Core,
    scope: &Scope,
    operation: &str,
    error: &anyhow::Error,
) -> Result<bool> {
    let rebuilt = super::telegram_provider::discard_mismatched_map(core, scope, error)?;
    if !rebuilt
        && !matches!(
            error.to_string().as_str(),
            "BACKUP_CHECKSUM_MISMATCH"
                | "BACKUP_MEDIA_IN_USE"
                | "BACKUP_REUPLOAD_STALE"
                | "TELEGRAM_INDEX_INVALID"
                | "BACKUP_CLEANUP_PENDING"
                | "TELEGRAM_MEDIA_MISSING"
                | "TELEGRAM_MEDIA_CHANGED"
                | "BACKUP_PUBLICATION_UNCONFIRMED"
        )
    {
        return Ok(false);
    }
    let separated = core.backup_db(|db| {
        Ok(db.execute(
            "UPDATE backup_operations SET payload_json=json_remove(payload_json,'$.archive_next')
         WHERE space_id=?1 AND kind='delete' AND state='pending' AND phase='archive_ready'
         AND json_extract(payload_json,'$.archive_next')=?2",
            params![scope.space_id, operation],
        )? != 0)
    })?;
    Ok(rebuilt || separated)
}

pub async fn execute(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    operation: &str,
) -> Result<()> {
    let work = super::delete_timing::timed(core, operation, "total", async {
        let first_publish = provider.scope().provider_type == ProviderType::Telegram && core.backup_db(|db| {
            db.execute("WITH clock(started) AS (SELECT CAST(unixepoch('subsec')*1000 AS INTEGER)),
                queue(elapsed) AS (SELECT max(0,COALESCE(json_extract(payload_json,'$.first_claimed_ms'),started)-json_extract(payload_json,'$.queued_at_ms'))
                    FROM clock,backup_operations WHERE operation_id=?1)
                UPDATE backup_operations SET payload_json=json_set(payload_json,
                    '$.foreground_started_ms',(SELECT started FROM clock),
                    '$.initial_queue_ms',(SELECT elapsed FROM queue),
                    '$.timings.foreground.queue',json_object('elapsed_ms',(SELECT elapsed FROM queue),
                        'total_elapsed_ms',(SELECT elapsed FROM queue),'calls',1,'attempt',attempt,'outcome','ok','at',unixepoch()))
                WHERE operation_id=?1 AND json_extract(payload_json,'$.queued_at_ms') IS NOT NULL
                AND json_extract(payload_json,'$.foreground_started_ms') IS NULL",[operation])?;
            claim_archive_read(db, operation)
        })?;
        let attempt = || {
            Box::pin(super::telegram_session::with_delete_config(execute_inner(
                core, provider, operation,
            )))
        };
        let result =
            super::telegram_transport::with_delete_archive_read(first_publish, attempt()).await;
        if let Err(error) = &result {
            if provider.scope().provider_type == ProviderType::Telegram
                && recover_cleanup(core, provider.scope(), operation, error)?
            {
                // Resume at the durable phase after rebuilding; retain all upload and delete receipts.
                return attempt().await;
            }
        }
        result
    });
    super::delete_timing::track_delete(core, operation, Box::pin(work)).await
}
async fn execute_inner(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    operation: &str,
) -> Result<()> {
    let op = scheduler::operation(core, operation)?;
    anyhow::ensure!(op.kind == "delete", "BACKUP_INVALID_OPERATION");
    let owned: Scope = serde_json::from_value(op.payload["scope"].clone())?;
    provider.scope().check(&owned)?;
    scope(core, &op.space_id)?.check(&owned)?;
    let local = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    let remote = sync_read::remote_id(core, &op.space_id, local)?;
    if op.payload["remote_asset_id"] != remote {
        bail!("BACKUP_SCOPE_MISMATCH");
    }
    if op.phase == "superseded" {
        complete_superseded(core, operation)?;
        return Ok(());
    }
    let published = if matches!(
        op.phase.as_str(),
        "archive_ready" | "objects" | "cleanup_verify" | "local_cleanup" | "finalizing"
    ) {
        let current = super::delete_timing::timed_setup(
            core,
            operation,
            "read_asset",
            provider.read_asset(&remote),
        )
        .await?
        .context("BACKUP_RECORD_UNAVAILABLE")?;
        if !current.snapshot.deleted
            || op.payload["archive_revision"]
                .as_u64()
                .is_some_and(|r| r != current.snapshot.revision)
        {
            complete_superseded(core, operation)?;
            return Ok(());
        }
        Some(current)
    } else {
        None
    };
    core.cancel_asset_restore(local)?;
    core.cancel_asset_transcription(local).await?;
    let _writer = super::delete_timing::timed(core, operation, "wait_writer", async {
        Ok(core.backup_write_lock.read().await)
    })
    .await?;
    let _file = core.restore_file_guard(local).await;
    scope(core, &op.space_id)?.check(&owned)?;
    if matches!(op.phase.as_str(), "local_cleanup" | "finalizing") {
        super::delete_timing::timed(core, operation, "local_cleanup", finish_local(core, &op))
            .await?;
        return finish_pass(core, &op);
    }
    let change = AssetChange {
        recreate: false,
        resolved_conflicts: vec![],
        asset_id: remote.clone(),
        base: op
            .payload
            .get("base")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?,
        fields: Default::default(),
        field_bases: Default::default(),
        media: None,
        cover: ReferenceChange::Unchanged,
        transcript: ReferenceChange::Unchanged,
        delete: true,
        device_id: core.backup_db(repository::device_id)?,
        operation_id: operation.into(),
        conflicts: vec![],
    };
    let asset = match if let Some(asset) = published {
        PublishReceipt::Converged {
            asset: Box::new(asset),
        }
    } else {
        super::delete_timing::timed_setup(
            core,
            operation,
            "publish",
            sync_engine::publish(provider, change),
        )
        .await?
    } {
        PublishReceipt::Converged { asset } => asset,
        PublishReceipt::Accepted { .. } => bail!("BACKUP_PUBLICATION_UNCONFIRMED"),
        PublishReceipt::Retry { reason } => bail!(reason),
    };
    if !asset.snapshot.deleted {
        complete_superseded(core, operation)?;
        return Ok(());
    }
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        sync_engine::import_asset(&tx, &owned, &asset)?;
        tx.execute(
            "UPDATE backup_operations SET phase='objects',last_error=NULL WHERE operation_id=?",
            [operation],
        )?;
        tx.commit()?;
        Ok(())
    })?;
    match super::delete_timing::timed_setup(
        core,
        operation,
        "remote_cleanup",
        provider.reconcile_deleted_asset(&remote, operation),
    )
    .await
    {
        Ok(DeleteCleanup::ArchiveReady) => return Ok(()),
        Ok(DeleteCleanup::Complete) => {}
        Err(error) => {
            if super::delete_timing::timed_setup(
                core,
                operation,
                "read_asset",
                provider.read_asset(&remote),
            )
            .await?
            .is_some_and(|r| !r.snapshot.deleted)
            {
                complete_superseded(core, operation)?;
                return Ok(());
            }
            return Err(error);
        }
    }
    if op.phase != "cleanup_verify" || op.payload["local_cleanup_done"] != true {
        core.backup_db(|db| {
            db.execute("UPDATE backup_operations SET phase='local_cleanup',last_error=NULL WHERE operation_id=?",[operation])?;
            Ok(())
        })?;
        super::delete_timing::timed(core, operation, "local_cleanup", finish_local(core, &op))
            .await?;
    }
    finish_pass(core, &op)
}

fn finish_pass(core: &Core, op: &scheduler::Operation) -> Result<()> {
    let operation = &op.operation_id;
    core.backup_db(|db|{
        let raw:String=db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?",[operation],|r|r.get(0))?;
        let mut payload:Value=serde_json::from_str(&raw)?;
        let pass=payload["cleanup_pass"].as_u64().unwrap_or(0)+1;payload["cleanup_pass"]=pass.into();
        if let Some(start) = payload["foreground_started_ms"].as_i64().filter(|_| pass == 1) {
            let end: i64 = db.query_row("SELECT CAST(unixepoch('subsec')*1000 AS INTEGER)", [], |r|r.get(0))?;
            payload["foreground_elapsed_ms"] = (end-start).into();
            if let Some(queued) = payload["queued_at_ms"].as_i64() {
                let claim = payload["first_claimed_ms"].as_i64().unwrap_or(start);
                payload["initial_queue_ms"] = (claim-queued).max(0).into();
            }
        }
        let done=pass>=3;
        db.execute("UPDATE backup_operations SET state=?2,phase=?3,payload_json=?4,next_retry_at=CASE WHEN ?5 THEN NULL ELSE unixepoch()+?6 END,last_error=NULL,updated_at=unixepoch() WHERE operation_id=?1 AND state='running'",params![operation,if done {"completed"} else {"retry_wait"},if done {"completed"} else {"cleanup_verify"},payload.to_string(),done,if pass==1 {30} else {120}])?;
        db.execute("UPDATE asset_remote_objects SET remote_state='deleted' WHERE space_id=?1 AND asset_id=?2",params![op.space_id,op.asset_id])?;Ok(())
    })
}

fn complete_superseded(core: &Core, operation: &str) -> Result<()> {
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='superseded',last_error=NULL WHERE operation_id=?",[operation])?;Ok(())})
}

/// Called only after a fresh authoritative tombstone during an actual missing-video read.
pub async fn confirm_on_read(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
    asset: &RemoteAsset,
) -> Result<()> {
    let scope = provider.scope();
    asset.version.scope.check(scope)?;
    if !asset.snapshot.deleted
        || asset.snapshot.library_id != scope.library_id
        || asset.snapshot.asset_id != sync_read::remote_id(core, &scope.space_id, local)?
    {
        bail!("BACKUP_TOMBSTONE_REQUIRED");
    }
    if super::restore::local(core, local).await?.is_some() {
        return Ok(());
    }
    core.backup_db(|db|{
        let device=repository::device_id(db)?;
        let proven:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_upload_proofs WHERE space_id=?1 AND remote_asset_id=?2 AND device_id=?3)",params![scope.space_id,asset.snapshot.asset_id,device],|r|r.get(0))?;
        if !proven {return Ok(());}
        let tx=db.unchecked_transaction()?;
        let pending:String=tx.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local],|r|r.get(0))?;
        let pending:PendingChanges=serde_json::from_str(&pending)?;
        if pending.cover.is_none() {
            // Only discard a cover proven to be the old synchronized cache.
            let raw:Option<String>=tx.query_row("SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",params![scope.space_id,local],|r|r.get(0)).optional()?;
            let old=raw.map(|r|serde_json::from_str::<AssetSnapshot>(&r)).transpose()?;
            if let Some(reference)=old.and_then(|s|s.cover) {
                let body:Option<Vec<u8>>=tx.query_row("SELECT body FROM asset_covers WHERE asset_id=?",[local],|r|r.get(0)).optional()?;
                use sha2::{Digest,Sha256};
                if body.is_some_and(|b|format!("{:x}",Sha256::digest(b))==reference.sha256) {
                    crate::cover_store::remove(&tx, crate::cover_store::Owner::Asset, local)?;
                }
            }
        }
        tx.execute("DELETE FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",params![scope.space_id,local])?;
        tx.execute("UPDATE backup_space_assets SET baseline_json=NULL WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local])?;
        tx.commit()?;Ok(())
    })
}

pub async fn release(core: &Arc<Core>, local: &str) -> Result<()> {
    let current = spaces::get(core)?
        .filter(|s| s.enabled && s.protocol == 2)
        .context("BACKUP_NOT_BACKED_UP")?;
    let pending: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",
            params![current.space_id, local],
            |r| r.get(0),
        )?)
    })?;
    if !serde_json::from_str::<PendingChanges>(&pending)?.is_empty() {
        bail!("BACKUP_UNSYNCED_LOCAL_CHANGES");
    }
    let provider = spaces::provider(core, &current.space_id)?;
    release_with(core, provider.as_ref(), local).await
}
pub async fn release_with(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
) -> Result<()> {
    let current = spaces::get(core)?
        .filter(|s| s.enabled && s.space_id == provider.scope().space_id && s.protocol == 2)
        .context("BACKUP_SPACE_INACTIVE")?;
    let remote = sync_read::latest(core, provider, local).await?;
    if remote.snapshot.deleted {
        bail!("BACKUP_ASSET_DELETED");
    }
    let asset = core
        .backup_asset(local)?
        .context("BACKUP_ASSET_NOT_FOUND")?;
    let media = remote.snapshot.media.context("BACKUP_NOT_BACKED_UP")?;
    let mut shutdown = core.backup_cancellation();
    if *shutdown.borrow() {
        bail!("CANCELED");
    }
    tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=provider.verify_media(media.clone())=>{result?;}}
    if asset.content_sha256.as_deref() != Some(&media.sha256) || asset.content_bytes != media.bytes
    {
        bail!("BACKUP_NOT_BACKED_UP");
    }
    core.cancel_asset_restore(local)?;
    core.cancel_asset_transcription(local).await?;
    let _writer = core.backup_write_lock.write().await;
    let _restore = core.backup_restore_lock.lock().await;
    let pending: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",
            params![current.space_id, local],
            |r| r.get(0),
        )?)
    })?;
    if !serde_json::from_str::<PendingChanges>(&pending)?.is_empty() {
        bail!("BACKUP_UNSYNCED_LOCAL_CHANGES");
    }
    lifecycle::remove_local_checked(core,local,|db|{
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_spaces s JOIN backup_current_space c USING(space_id) WHERE s.space_id=?1 AND s.library_id=?2 AND s.enabled=1)",params![current.space_id,provider.scope().library_id],|r|r.get(0))?;
        if !valid {bail!("BACKUP_SPACE_INACTIVE");}
        let (deleted,pending):(bool,String)=db.query_row("SELECT remote_deleted,pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![current.space_id,local],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if deleted {bail!("BACKUP_ASSET_DELETED");}
        let deleting:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state NOT IN ('completed','canceled'))",params![current.space_id,local],|r|r.get(0))?;
        if deleting {bail!("BACKUP_DELETE_PENDING");}
        if !serde_json::from_str::<PendingChanges>(&pending)?.is_empty() {bail!("BACKUP_UNSYNCED_LOCAL_CHANGES");}
        Ok(())
    }).await
}

#[cfg(test)]
#[test]
fn confirmed_archive_yields_to_its_successor_and_remains_runnable_after_restart_or_failure(
) -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("queue.sqlite");
    let db = rusqlite::Connection::open(&path)?;
    repository::init(&db)?;
    db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,enabled) VALUES('s','l','','','',0)", [])?;
    db.execute(
        "INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,attempt,payload_json)
                VALUES('a','s','delete','running','objects',1,'{\"remote_asset_id\":\"first\"}')",
        [],
    )?;
    assert!(!park_confirmed_archive(&db, "s", "a", 300)?);
    db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json)
                VALUES('b','s','delete','pending','tombstone','{}'),('c','s','delete','pending','tombstone','{}')", [])?;
    assert!(park_confirmed_archive(&db, "s", "a", 300)?);
    drop(db);
    let db = rusqlite::Connection::open(path)?;
    assert_eq!(
        scheduler::next_sync_operation(&db, "s")?,
        Some(("b".into(), true))
    );
    let peer = parked_archive(&db, "s", "b")?.unwrap();
    assert_eq!(
        (peer.operation.as_str(), peer.asset.as_str(), peer.revision),
        ("a", "first", 300)
    );
    assert!(parked_archive(&db, "s", "c")?.is_none());
    db.execute("UPDATE backup_operations SET state='retry_wait',next_retry_at=unixepoch()+30 WHERE operation_id='b'", [])?;
    assert_eq!(
        scheduler::next_sync_operation(&db, "s")?,
        Some(("a".into(), true))
    );
    assert!(scheduler::foreground_delete_busy(&db, "s")?);
    Ok(())
}

#[cfg(test)]
#[test]
fn retrying_a_delete_before_send_cannot_claim_the_publish_read_again() -> Result<()> {
    let db = rusqlite::Connection::open_in_memory()?;
    repository::init(&db)?;
    db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,enabled) VALUES('s','l','','','',0)", [])?;
    db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,attempt,payload_json)
                VALUES('op','s','delete','running','tombstone',1,'{\"first_archive_publish\":true,\"cleanup_pass\":0}')", [])?;
    assert!(claim_archive_read(&db, "op")?);
    // A manual retry resets the counter, even if no remote send was reserved.
    db.execute(
        "UPDATE backup_operations SET state='pending', attempt=0 WHERE operation_id='op'",
        [],
    )?;
    db.execute(
        "UPDATE backup_operations SET state='running', attempt=attempt+1 WHERE operation_id='op'",
        [],
    )?;
    assert!(!claim_archive_read(&db, "op")?);
    let saved: String = db.query_row(
        "SELECT payload_json FROM backup_operations WHERE operation_id='op'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        serde_json::from_str::<Value>(&saved)?,
        json!({"cleanup_pass":0})
    );
    Ok(())
}
