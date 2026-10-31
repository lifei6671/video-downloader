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
    if current.protocol != 2 {
        bail!("BACKUP_UPGRADE_REQUIRED");
    }
    Ok(Scope {
        space_id: space.into(),
        library_id: current.library_id,
        provider_type: current.provider_type,
        protocol: 2,
    })
}
pub fn intent(core: &Core, space: &str, local: &str) -> Result<String> {
    let scope = scope(core, space)?;
    let remote = sync_read::remote_id(core, space, local)?;
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let previous:Option<(String,String,String)>=tx.query_row("SELECT operation_id,state,payload_json FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state<>'canceled' ORDER BY CASE WHEN state='completed' THEN 1 ELSE 0 END,rowid DESC LIMIT 1",params![space,local],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id,state,raw))=previous {
            let mut payload:Value=serde_json::from_str(&raw)?;
            if state=="completed" {payload["cleanup_pass"]=0.into();}
            if let Some(saved)=payload.get("scope") {scope.check(&serde_json::from_value(saved.clone())?)?;}
            payload["scope"]=serde_json::to_value(&scope)?;payload["remote_asset_id"]=remote.clone().into();
            tx.execute("UPDATE backup_operations SET state=CASE WHEN state='running' THEN state ELSE 'pending' END,phase=CASE WHEN ?2 THEN 'local' ELSE phase END,next_retry_at=NULL,last_error=NULL,attempt=0,payload_json=?3 WHERE operation_id=?1",params![id,state=="completed",payload.to_string()])?;
        } else {
            tx.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'delete','pending','local',?4)",params![uuid::Uuid::new_v4().to_string(),space,local,json!({"scope":scope,"remote_asset_id":remote,"cleanup_pass":0}).to_string()])?;
        }
        let operation:String=tx.query_row("SELECT operation_id FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state NOT IN ('completed','canceled')",params![space,local],|r|r.get(0))?;
        tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_DELETE_PENDING' WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state NOT IN ('completed','canceled')",params![space,local])?;
        tx.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_key,remote_state) VALUES(?1,?2,'provider','pending_delete') ON CONFLICT(space_id,asset_id) DO UPDATE SET remote_state='pending_delete'",params![space,local])?;
        tx.commit()?;Ok(operation)
    })
}
pub async fn delete(core: &Arc<Core>, space: &str, local: &str) -> Result<()> {
    let operation = intent(core, space, local)?;
    core.wake_backup();
    let op = scheduler::operations(core)?
        .into_iter()
        .find(|o| o.operation_id == operation)
        .context("BACKUP_INVALID_OPERATION")?;
    prepare_local(core, &op).await
}
async fn prepare_local(core: &Arc<Core>, op: &scheduler::Operation) -> Result<()> {
    let local = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    core.cancel_asset_restore(local)?;
    core.cancel_asset_transcription(local).await?;
    let _writer = core.backup_write_lock.lock().await;
    let _restore = core.backup_restore_lock.lock().await;
    let phase: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT phase FROM backup_operations WHERE operation_id=?",
            [&op.operation_id],
            |r| r.get(0),
        )?)
    })?;
    if phase != "local" {
        return Ok(());
    }
    if let Err(error) = lifecycle::remove_local(core, local).await {
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1",
                params![op.operation_id, error.to_string()],
            )?;
            Ok(())
        })?;
        return Err(error);
    }
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET phase='tombstone',last_error=NULL WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })?;
    core.wake_backup();
    Ok(())
}
pub async fn execute(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    operation: &str,
) -> Result<()> {
    let op = scheduler::operations(core)?
        .into_iter()
        .find(|o| o.operation_id == operation && o.kind == "delete")
        .context("BACKUP_INVALID_OPERATION")?;
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
    let published = if matches!(op.phase.as_str(), "objects" | "cleanup_verify") {
        let current = provider
            .read_asset(&remote)
            .await?
            .context("BACKUP_RECORD_UNAVAILABLE")?;
        if !current.snapshot.deleted {
            complete_superseded(core, operation)?;
            return Ok(());
        }
        Some(current)
    } else {
        None
    };
    prepare_local(core, &op).await?;
    let _writer = core.backup_write_lock.lock().await;
    let change = AssetChange {
        recreate: false,
        resolved_conflicts: vec![],
        asset_id: remote.clone(),
        base: None,
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
        sync_engine::publish(provider, change).await?
    } {
        PublishReceipt::Converged { asset } => asset,
        PublishReceipt::Accepted { .. } => bail!("BACKUP_PUBLICATION_UNCONFIRMED"),
        PublishReceipt::Retry { reason } => bail!(reason),
    };
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
    if let Err(error) = provider.reconcile_deleted_asset(&remote, operation).await {
        if provider
            .read_asset(&remote)
            .await?
            .is_some_and(|r| !r.snapshot.deleted)
        {
            complete_superseded(core, operation)?;
            return Ok(());
        }
        return Err(error);
    }
    core.backup_db(|db|{
        let raw:String=db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?",[operation],|r|r.get(0))?;
        let mut payload:Value=serde_json::from_str(&raw)?;
        let pass=payload["cleanup_pass"].as_u64().unwrap_or(0)+1;payload["cleanup_pass"]=pass.into();
        let done=pass>=3;
        db.execute("UPDATE backup_operations SET state=?2,phase=?3,payload_json=?4,next_retry_at=CASE WHEN ?5 THEN NULL ELSE unixepoch()+?6 END,last_error=NULL,updated_at=unixepoch() WHERE operation_id=?1 AND state='running'",params![operation,if done {"completed"} else {"retry_wait"},if done {"completed"} else {"cleanup_verify"},payload.to_string(),done,if pass==1 {30} else {120}])?;
        db.execute("UPDATE asset_remote_objects SET remote_state='deleted' WHERE space_id=?1 AND asset_id=?2",params![owned.space_id,local])?;Ok(())
    })
}

fn complete_superseded(core: &Core, operation: &str) -> Result<()> {
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='superseded',last_error=NULL WHERE operation_id=?",[operation])?;Ok(())})
}

/// Existing background maintenance handles the current space only; no page or second task queue.
pub async fn compact_current(
    core: &Arc<Core>,
    resolve: sync_engine::ProviderResolver,
) -> Result<()> {
    let Some(current) = spaces::get(core)?
        .filter(|s| s.enabled && !s.connection_pending && s.cleanup_deleted_records)
    else {
        return Ok(());
    };
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let due=core.backup_db(|db|Ok(db.execute("UPDATE backup_record_retention SET next_check_at=?2 WHERE space_id=?1 AND enabled=1 AND next_check_at<=?3",params![current.space_id,i64::try_from(now+300)?,i64::try_from(now)?])?>0))?;
    if !due {
        return Ok(());
    }
    let work=async {
        let provider=resolve(core,&current.space_id)?;
        let candidates:Vec<RemoteAsset>=core.backup_db(|db|{
            let mut q=db.prepare("SELECT t.record_json,t.version_json FROM backup_tombstones t JOIN backup_space_assets m USING(space_id,remote_asset_id) WHERE t.space_id=? AND m.remote_deleted=1")?;
            let rows=q.query_map([&current.space_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter().map(|(snapshot,version)|Ok(RemoteAsset{snapshot:serde_json::from_str(&snapshot)?,version:serde_json::from_str(&version)?})).collect()
        })?;
        let engine=sync_engine::SyncEngine::new(core.clone(),provider.clone());
        let mut cancel=core.backup_cancellation();
        let verified:bool=core.backup_db(|db|Ok(db.query_row("SELECT delete_verified FROM backup_record_retention WHERE space_id=?",[&current.space_id],|r|r.get(0))?))?;
        let cleanup=async {
            if !candidates.is_empty() && !verified {
                spaces::verify_record_cleanup(core,&current.space_id).await?;
                core.backup_db(|db|{db.execute("UPDATE backup_record_retention SET delete_verified=1 WHERE space_id=?",[&current.space_id])?;Ok(())})?;
            }
            provider.compact_deleted_records(candidates,current.deleted_record_retention_days,now).await
        };
        let compacted=sync_engine::scoped(&engine,&mut cancel,async {tokio::time::timeout(std::time::Duration::from_secs(120),cleanup).await.context("BACKUP_TIMEOUT")?}).await?;
        core.backup_db(|db|{let tx=db.unchecked_transaction()?;for remote in compacted {sync_engine::import_asset(&tx,provider.scope(),&remote)?;}tx.execute("UPDATE backup_record_retention SET next_check_at=?2,last_error=NULL WHERE space_id=?1",params![current.space_id,i64::try_from(now+3600)?])?;tx.commit()?;Ok(())})?;
        Ok::<_,anyhow::Error>(())
    }.await;
    if let Err(error) = &work {
        if error.to_string() != "CANCELED" {
            core.backup_db(|db| {
                db.execute(
                    "UPDATE backup_record_retention SET last_error=?2 WHERE space_id=?1",
                    params![current.space_id, error.to_string()],
                )?;
                Ok(())
            })?;
        }
    }
    work
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
                if body.is_some_and(|b|format!("{:x}",Sha256::digest(b))==reference.sha256) {tx.execute("DELETE FROM asset_covers WHERE asset_id=?",[local])?;}
            }
        }
        tx.execute("DELETE FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",params![scope.space_id,local])?;
        tx.execute("UPDATE backup_space_assets SET baseline_json=NULL WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local])?;
        tx.execute("UPDATE asset_remote_objects SET media_key=NULL,cover_key=NULL,transcript_key=NULL WHERE space_id=?1 AND asset_id=?2",params![scope.space_id,local])?;
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
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == local)
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
    let _writer = core.backup_write_lock.lock().await;
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
