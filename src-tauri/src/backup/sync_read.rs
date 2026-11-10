//! Provider-neutral lazy reads; listing and small-object reads never restore video bytes.
use super::{
    model::AssetTranscript,
    provider::*,
    restore, scheduler,
    sync_engine::{self, PendingChanges},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub fn remote_id(core: &Core, space: &str, local: &str) -> Result<String> {
    core.backup_db(|db|Ok(db.query_row("SELECT remote_asset_id FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|r.get(0))?))
}
pub fn cached(core: &Core, space: &str, local: &str) -> Result<AssetSnapshot> {
    let raw: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",
            params![space, local],
            |r| r.get(0),
        )?)
    })?;
    serde_json::from_str(&raw).context("BACKUP_RECORD_UNAVAILABLE")
}
pub async fn latest(
    core: &Core,
    provider: &dyn BackupProvider,
    local: &str,
) -> Result<RemoteAsset> {
    let id = remote_id(core, &provider.scope().space_id, local)?;
    let remote = provider
        .read_asset(&id)
        .await?
        .context("BACKUP_OBJECT_NOT_FOUND")?;
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        sync_engine::import_asset(&tx, provider.scope(), &remote)?;
        tx.commit()?;
        Ok(())
    })?;
    Ok(remote)
}
pub async fn small(
    core: &Core,
    provider: &dyn BackupProvider,
    reference: &MediaRef,
) -> Result<Vec<u8>> {
    for attempt in 0..2 {
        match small_once(core, provider, reference).await {
            Err(error)
                if attempt == 0
                    && provider.scope().provider_type == ProviderType::Telegram
                    && error.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" =>
            {
                continue
            }
            result => return result,
        }
    }
    unreachable!("bounded small-object refresh always returns")
}
async fn small_once(
    core: &Core,
    provider: &dyn BackupProvider,
    reference: &MediaRef,
) -> Result<Vec<u8>> {
    if reference.bytes > 16 * 1024 * 1024 {
        bail!("BACKUP_METADATA_TOO_LARGE");
    }
    let mut shutdown = core.backup_cancellation();
    if *shutdown.borrow() {
        bail!("CANCELED");
    }
    let stream = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=provider.download_media(reference.clone(),0)=>result?};
    let mut body = Vec::new();
    let mut limited = stream.reader.take(reference.bytes + 1);
    let mut buffer = vec![0u8; 65536];
    loop {
        let count = tokio::select! {
            _ = shutdown.changed() => bail!("CANCELED"),
            result = tokio::time::timeout(provider.media_read_timeout(), limited.read(&mut buffer)) => result.context("BACKUP_NETWORK_ERROR")??,
        };
        if count == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..count]);
    }
    if body.len() as u64 != reference.bytes
        || format!("{:x}", Sha256::digest(&body)) != reference.sha256
    {
        bail!("BACKUP_CHECKSUM_MISMATCH");
    }
    Ok(body)
}
pub async fn transcript(
    core: &Core,
    provider: &dyn BackupProvider,
    local: &str,
) -> Result<Option<AssetTranscript>> {
    let space = &provider.scope().space_id;
    let (previous, raw, deleted) = core.backup_db(|db| {
        let state = local_transcript(db, local)?;
        let raw: String = db.query_row("SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2", params![space,local], |r| r.get(0))?;
        let deleted: bool = db.query_row("SELECT remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2", params![space,local], |r| r.get(0))?;
        Ok((state, raw, deleted))
    })?;
    let existing = previous.body.clone();
    if deleted || previous.dirty {
        return Ok(existing);
    }
    let record: AssetSnapshot = serde_json::from_str(&raw)?;
    if record.deleted {
        return Ok(existing);
    }
    let Some(reference) = &record.transcript else {
        return Ok(existing);
    };
    if let Some(mut value) = existing.clone() {
        value.asset_id = record.asset_id.clone();
        if format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)) == reference.sha256 {
            return Ok(existing);
        }
    }
    let mut value: AssetTranscript =
        serde_json::from_slice(&small(core, provider, reference).await?)
            .context("BACKUP_INVALID_TRANSCRIPT")?;
    if value.asset_id != record.asset_id
        || value.content_sha256 != record.media.as_ref().map(|m| m.sha256.clone())
    {
        bail!("BACKUP_INVALID_TRANSCRIPT");
    }
    value.asset_id = local.into();
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let current = local_transcript(&tx, local)?;
        let current_remote: Option<(String,bool)> = tx.query_row(
            "SELECT r.record_json,m.remote_deleted FROM asset_remote_records r JOIN backup_space_assets m ON m.space_id=r.space_id AND m.local_asset_id=r.asset_id WHERE r.space_id=?1 AND r.asset_id=?2",
            params![space,local], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        if current.dirty || current.revisions != previous.revisions || current.body != previous.body
            || current_remote.as_ref().is_none_or(|(record,deleted)| *deleted || record != &raw) {
            return Ok(current.body);
        }
        tx.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,model_name,pipeline_revision,completed_at,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,NULL) ON CONFLICT(asset_id) DO UPDATE SET content_sha256=excluded.content_sha256,text=excluded.text,model_revision=excluded.model_revision,model_name=excluded.model_name,pipeline_revision=excluded.pipeline_revision,completed_at=excluded.completed_at,source_json=NULL",params![local,value.content_sha256,value.text,value.model_revision,value.model_name,value.pipeline_revision,i64::try_from(value.completed_at)?])?;
        tx.commit()?;
        Ok(Some(value))
    })
}
struct TranscriptState {
    body: Option<AssetTranscript>,
    dirty: bool,
    revisions: Vec<(String, i64)>,
}
fn local_transcript(db: &rusqlite::Connection, local: &str) -> Result<TranscriptState> {
    let body = super::repository::transcript(db, local)?;
    let mut dirty = false;
    let mut revisions = Vec::new();
    let mut query = db.prepare("SELECT space_id,local_revision,pending_json FROM backup_space_assets WHERE local_asset_id=? ORDER BY space_id")?;
    for row in query.query_map([local], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?))
    })? {
        let (space, revision, pending) = row?;
        dirty |= serde_json::from_str::<PendingChanges>(&pending)?.transcript_dirty();
        revisions.push((space, revision));
    }
    Ok(TranscriptState {
        body,
        dirty,
        revisions,
    })
}
// Covers are shared by all memberships of a local asset. A lazy read from one
// space must not replace a choice or local edit waiting to publish in another.
struct CoverState {
    body: Option<Vec<u8>>,
    dirty: bool,
    revisions: Vec<(String, i64)>,
}
fn local_cover(db: &rusqlite::Connection, local: &str) -> Result<CoverState> {
    let body = db
        .query_row(
            "SELECT body FROM asset_covers WHERE asset_id=?",
            [local],
            |r| r.get(0),
        )
        .optional()?;
    let dirty = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_space_assets m WHERE m.local_asset_id=?1 AND (json_extract(m.pending_json,'$.cover') IS NOT NULL OR EXISTS(SELECT 1 FROM json_each(m.pending_json,'$.choices') c WHERE json_extract(c.value,'$.adopt')=1 AND json_extract(c.value,'$.conflict.field')='cover')))",
        [local], |r| r.get(0),
    )?;
    let revisions = db.prepare("SELECT space_id,local_revision FROM backup_space_assets WHERE local_asset_id=? ORDER BY space_id")?
        .query_map([local], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(CoverState {
        body,
        dirty,
        revisions,
    })
}

pub async fn cover(core: &Core, provider: &dyn BackupProvider, local: &str) -> Result<Vec<u8>> {
    let previous = core.backup_db(|db| local_cover(db, local))?;
    if previous.dirty {
        return previous.body.context("COVER_UNAVAILABLE");
    }
    let deleted:bool=core.backup_db(|db|Ok(db.query_row("SELECT remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![provider.scope().space_id,local],|r|r.get(0))?))?;
    if deleted {
        return cached_cover(core, &provider.scope().space_id, local)?.context("COVER_UNAVAILABLE");
    }
    let record = cached(core, &provider.scope().space_id, local)?;
    let reference = record.cover.context("COVER_UNAVAILABLE")?;
    if let Some(body) = previous
        .body
        .as_ref()
        .filter(|b| format!("{:x}", Sha256::digest(b)) == reference.sha256)
    {
        return Ok(body.clone());
    }
    let body = small(core, provider, &reference).await?;
    if crate::files::image_extension(&body).is_none() {
        bail!("INVALID_MEDIA_CONTENT");
    }
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let current = local_cover(&tx, local)?;
        // Also catch an edit that has already been uploaded while this read was
        // in flight, so clearing pending flags cannot resurrect the old bytes.
        if current.dirty || current.revisions != previous.revisions || current.body != previous.body {
            return current.body.context("COVER_UNAVAILABLE");
        }
        tx.execute("INSERT INTO asset_covers VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET body=excluded.body", params![local,body])?;
        tx.commit()?;
        Ok(body)
    })
}

fn active(core: &Core, scope: &Scope, operation: &str) -> Result<()> {
    if *core.backup_cancellation().borrow() {
        bail!("CANCELED");
    }
    core.backup_db(|db|{
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations o JOIN backup_spaces s USING(space_id) JOIN backup_current_space c USING(space_id) WHERE o.operation_id=?1 AND o.space_id=?2 AND o.state='running' AND s.enabled=1 AND s.library_id=?3)",params![operation,scope.space_id,scope.library_id],|r|r.get(0))?;
        if !valid {bail!("CANCELED");}Ok(())
    })
}
fn checkpoint(
    core: &Core,
    scope: &Scope,
    operation: &str,
    phase: &str,
    payload: &Value,
) -> Result<()> {
    active(core, scope, operation)?;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET phase=?2,payload_json=?3 WHERE operation_id=?1",
            params![operation, phase, payload.to_string()],
        )?;
        Ok(())
    })
}

async fn recover_restore_network(
    core: &Core,
    scope: &Scope,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let work = crate::telegram_accounts::recover_network(
        core,
        crate::telegram_accounts::Purpose::Backup,
        cancel,
    );
    tokio::pin!(work);
    loop {
        let current = super::spaces::get(core)?.context("BACKUP_SPACE_INACTIVE")?;
        if !super::spaces::enabled(core)?
            || !current.enabled
            || current.space_id != scope.space_id
            || current.library_id != scope.library_id
        {
            bail!("CANCELED");
        }
        tokio::select! {
            result = &mut work => return result,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

pub async fn ensure(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
) -> Result<PathBuf> {
    if let Some(path) = restore::local(core, local).await? {
        return Ok(path);
    }
    let mut shutdown = core.backup_cancellation();
    let _slot = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),slot=core.backup_restore_lock.lock()=>slot};
    let _files = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),guard=core.restore_file_guard(local)=>guard};
    if let Some(path) = restore::local(core, local).await? {
        return Ok(path);
    }
    let scope = provider.scope();
    let existing = scheduler::active_restore(core, &scope.space_id, local)?;
    let (operation, mut payload) = existing
        .map(|o| (o.operation_id, o.payload))
        .unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), json!({"scope":scope})));
    let owned: Scope =
        serde_json::from_value(payload["scope"].clone()).context("BACKUP_INVALID_OPERATION")?;
    scope.check(&owned)?;
    core.backup_db(|db|{let tx=db.unchecked_transaction()?;tx.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'restore','running','preparing',?4) ON CONFLICT(operation_id) DO UPDATE SET state='running',last_error=NULL",params![operation,scope.space_id,local,payload.to_string()])?;tx.execute("UPDATE asset_local_files SET local_state='restoring' WHERE asset_id=?",[local])?;tx.commit()?;Ok(())})?;
    let preparation_stop = shutdown.clone();
    let mut work = Box::pin(async {
        let remote = loop {
            match latest(core, provider, local).await {
                Err(error)
                    if scope.provider_type == ProviderType::Telegram
                        && crate::telegram_reconnect::network_error(&error.to_string()) =>
                {
                    checkpoint(core, scope, &operation, "reconnecting", &payload)?;
                    recover_restore_network(core, scope, preparation_stop.clone()).await?;
                }
                result => break result?,
            }
        };
        if remote.snapshot.deleted {
            if let Some(path) = restore::local(core, local).await? {
                core.backup_db(|db| { db.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL WHERE operation_id=?", [&operation])?; Ok(()) })?;
                return Ok(path);
            }
            super::sync_delete::confirm_on_read(core, provider, local, &remote).await?;
            bail!("BACKUP_ASSET_DELETED");
        }
        let reference = remote
            .snapshot
            .media
            .clone()
            .context("BACKUP_RECORD_UNAVAILABLE")?;
        let (_stop, cancel) = tokio::sync::watch::channel(false);
        Box::pin(transfer(
            core,
            provider,
            local,
            &remote.snapshot,
            &reference,
            &operation,
            &mut payload,
            cancel,
        ))
        .await
    });
    let result = loop {
        tokio::select! {result=&mut work=>break result,_=shutdown.changed()=>break Err(anyhow::anyhow!("CANCELED")),_=tokio::time::sleep(Duration::from_millis(100))=>{if let Err(error)=active(core,scope,&operation) {break Err(error);}}}
    };
    if let Err(error) = &result {
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1 AND state='running'",params![operation,error.to_string()])?;db.execute("UPDATE asset_local_files SET local_state='missing' WHERE asset_id=? AND local_state='restoring'",[local])?;Ok(())})?;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn transfer(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
    record: &AssetSnapshot,
    reference: &MediaRef,
    operation: &str,
    payload: &mut Value,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<PathBuf> {
    let mut refreshed = false;
    loop {
        match transfer_once(
            core,
            provider,
            local,
            record,
            reference,
            operation,
            payload,
            cancel.clone(),
        )
        .await
        {
            Err(error)
                if !refreshed
                    && provider.scope().provider_type == ProviderType::Telegram
                    && error.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" =>
            {
                // Refresh the request token while preserving the validated content prefix.
                refreshed = true;
                checkpoint(
                    core,
                    provider.scope(),
                    operation,
                    "refreshing_reference",
                    payload,
                )?;
            }
            Err(error)
                if provider.scope().provider_type == ProviderType::Telegram
                    && crate::telegram_reconnect::network_error(&error.to_string()) =>
            {
                active(core, provider.scope(), operation)?;
                checkpoint(core, provider.scope(), operation, "reconnecting", payload)?;
                recover_restore_network(core, provider.scope(), cancel.clone()).await?;
            }
            Err(error)
                if provider.scope().provider_type == ProviderType::Telegram
                    && error.to_string() == "TELEGRAM_NETWORK_CHANGED" =>
            {
                active(core, provider.scope(), operation)?;
                checkpoint(core, provider.scope(), operation, "reconnecting", payload)?;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            result => return result,
        }
    }
}
#[allow(clippy::too_many_arguments)]
async fn transfer_once(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
    record: &AssetSnapshot,
    reference: &MediaRef,
    operation: &str,
    payload: &mut Value,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<PathBuf> {
    let scope = provider.scope();
    let directory = restore::staging_directory(&core.settings().download_directory)?;
    let part = directory.join(format!("{operation}.part"));
    if payload["part"]
        .as_str()
        .is_some_and(|old| Path::new(old) != part)
    {
        bail!("BACKUP_RESTORE_DIRECTORY_CHANGED");
    }
    payload["part"] = part.to_string_lossy().into_owned().into();
    if let Some(target) = payload["target"].as_str().map(PathBuf::from) {
        if target.parent() != directory.parent() {
            bail!("BACKUP_UNSAFE_FILE");
        }
        if restore::regular(&target)?.is_some() {
            let (hash, source) = restore::fingerprint(
                target.clone(),
                core.backup_cancellation(),
                cancel.clone(),
                reference.container.clone(),
            )
            .await?;
            if hash != reference.sha256 || source["size"] != reference.bytes {
                bail!("BACKUP_LOCAL_CHANGED");
            }
            let newest = latest(core, provider, local).await?;
            if newest.snapshot.deleted {
                bail!("BACKUP_ASSET_DELETED");
            }
            core.commit_asset_file(local, &target, &hash, &source)?;
            core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL WHERE operation_id=?",[operation])?;Ok(())})?;
            return Ok(target);
        }
    }
    let mut offset = restore::regular(&part)?.map_or(0, |m| m.len());
    if payload["reference"] != serde_json::to_value(reference)? || offset > reference.bytes {
        offset = 0;
    }
    payload["reference"] = serde_json::to_value(reference)?;
    payload["total_bytes"] = reference.bytes.into();
    checkpoint(core, scope, operation, "downloading", payload)?;
    if offset < reference.bytes {
        let mut stream = provider.download_media(reference.clone(), offset).await?;
        anyhow::ensure!(stream.offset <= offset, "BACKUP_INVALID_RANGE");
        offset = stream.offset;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(offset == 0)
            .open(&part)
            .await?;
        file.set_len(offset).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut buffer = vec![0; 256 * 1024];
        let mut last = offset;
        loop {
            active(core, scope, operation)?;
            let n = match tokio::time::timeout(
                provider.media_read_timeout(),
                stream.reader.read(&mut buffer),
            )
            .await
            {
                Ok(Ok(n)) => n,
                Ok(Err(error))
                    if scope.provider_type == ProviderType::Telegram
                        && error.to_string().starts_with("TELEGRAM_") =>
                {
                    file.flush().await?;
                    file.sync_all().await?;
                    return Err(error.into());
                }
                _ => {
                    file.flush().await?;
                    file.sync_all().await?;
                    bail!("BACKUP_NETWORK_ERROR");
                }
            };
            if n == 0 {
                break;
            }
            if offset + n as u64 > reference.bytes {
                bail!("BACKUP_SIZE_MISMATCH");
            }
            file.write_all(&buffer[..n]).await?;
            offset += n as u64;
            if offset - last >= 1024 * 1024 {
                file.flush().await?;
                payload["completed_bytes"] = offset.into();
                checkpoint(core, scope, operation, "downloading", payload)?;
                last = offset;
            }
        }
        file.flush().await?;
        file.sync_all().await?;
    }
    payload["completed_bytes"] = offset.into();
    checkpoint(core, scope, operation, "verifying", payload)?;
    if offset != reference.bytes {
        bail!("BACKUP_SIZE_MISMATCH");
    }
    let (hash, mut source) = match restore::fingerprint(
        part.clone(),
        core.backup_cancellation(),
        cancel,
        reference.container.clone(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            if error.to_string() != "CANCELED" {
                payload["reference"] = Value::Null;
                checkpoint(core, scope, operation, "verification_failed", payload)?;
            }
            return Err(error);
        }
    };
    if hash != reference.sha256 || source["size"] != reference.bytes {
        payload["reference"] = Value::Null;
        checkpoint(core, scope, operation, "verification_failed", payload)?;
        bail!("BACKUP_CHECKSUM_MISMATCH");
    }
    let newest = latest(core, provider, local).await?;
    if newest.snapshot.deleted {
        bail!("BACKUP_ASSET_DELETED");
    }
    if newest.snapshot.media.as_ref() != Some(reference) {
        bail!("BACKUP_SOURCE_CHANGED");
    }
    active(core, scope, operation)?;
    let filename = record
        .fields
        .get("original_filename")
        .and_then(Value::as_str)
        .context("BACKUP_INVALID_FILENAME")?;
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let name = crate::naming::render("{content}", &json!({"title":stem}), "")?;
    let target = crate::naming::destination(
        directory.parent().context("IO_ERROR")?,
        &name,
        Some(&reference.container),
    )?;
    payload["target"] = target.to_string_lossy().into_owned().into();
    checkpoint(core, scope, operation, "committing", payload)?;
    tempfile::TempPath::try_from_path(part)?
        .persist_noclobber(&target)
        .map_err(|e| {
            let _ = e.path.keep();
            anyhow::anyhow!("IO_ERROR")
        })?;
    source["path"] = target.canonicalize()?.to_string_lossy().into_owned().into();
    // A successfully committed local copy is retained if a tombstone arrives afterwards.
    core.commit_asset_file(local, &target, &hash, &source)?;
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL WHERE operation_id=?",[operation])?;Ok(())})?;
    Ok(target)
}

pub fn list(core: &Core, space: &str) -> Result<Vec<super::reconcile::Entry>> {
    core.backup_display_db(|db| {
        let tx = db.unchecked_transaction()?;
        let mut assets = super::repository::list_assets(&tx)?;
        for asset in &mut assets {
            if asset.path.as_ref().is_some_and(|p| !p.is_file()) {
                asset.path = None;
                asset.local_state = "missing".into();
            }
        }
        list_in(&tx, space, assets)
    })
}

pub(super) fn list_in(
    db: &rusqlite::Connection,
    space: &str,
    assets: Vec<super::model::Asset>,
) -> Result<Vec<super::reconcile::Entry>> {
    let mut query = db.prepare("SELECT m.local_asset_id,m.remote_deleted,m.pending_json,
        (SELECT COUNT(*) FROM backup_conflicts c WHERE c.space_id=m.space_id AND c.remote_asset_id=m.remote_asset_id),
        h.asset_id IS NOT NULL,d.asset_id IS NOT NULL,o.remote_state,r.record_json,
        c.asset_id IS NOT NULL,t.asset_id IS NOT NULL
        FROM backup_space_assets m
        LEFT JOIN asset_hidden_history h ON h.asset_id=m.local_asset_id
        LEFT JOIN asset_deletion_hidden_history d ON d.space_id=m.space_id AND d.asset_id=m.local_asset_id
        LEFT JOIN asset_remote_objects o ON o.space_id=m.space_id AND o.asset_id=m.local_asset_id
        LEFT JOIN asset_remote_records r ON r.space_id=m.space_id AND r.asset_id=m.local_asset_id
        LEFT JOIN asset_covers c ON c.asset_id=m.local_asset_id
        LEFT JOIN asset_transcripts t ON t.asset_id=m.local_asset_id WHERE m.space_id=?1")?;
    let mut members = query
        .query_map([space], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as usize,
                    r.get::<_, bool>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, bool>(9)?,
                ),
            ))
        })?
        .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
    let mut entries = Vec::new();
    for asset in assets {
        let Some((
            deleted,
            pending,
            conflicts,
            hidden,
            deletion_hidden,
            remote_state,
            raw,
            local_cover,
            local_text,
        )) = members.remove(&asset.asset_id)
        else {
            continue;
        };
        let pending = !serde_json::from_str::<PendingChanges>(&pending)?.is_empty();
        let deleting = remote_state.as_deref() == Some("pending_delete");
        if (hidden || deletion_hidden) && !deleting {
            continue;
        }
        if deleted && asset.path.is_none() && !pending && conflicts == 0 && !deleting {
            continue;
        }
        let snapshot = raw
            .map(|v| serde_json::from_str::<AssetSnapshot>(&v))
            .transpose()
            .context("BACKUP_RECORD_UNAVAILABLE")?;
        entries.push(super::reconcile::Entry {
            has_cover: local_cover || snapshot.as_ref().is_some_and(|r| r.cover.is_some()),
            has_transcript: local_text || snapshot.as_ref().is_some_and(|r| r.transcript.is_some()),
            asset,
            space_id: space.into(),
            remote_state: if deleting {
                "pending_delete"
            } else if deleted {
                "deleted"
            } else if pending {
                "pending"
            } else {
                "backed_up"
            }
            .into(),
            remote_deleted: deleted,
            has_pending_local_changes: pending,
            conflict_count: conflicts,
            remote_revision: snapshot.as_ref().map_or(0, |r| r.revision),
        });
    }
    Ok(entries)
}

pub fn cached_transcript(core: &Core, space: &str, local: &str) -> Result<Option<AssetTranscript>> {
    let Some(mut value) = core.asset_transcript(local)? else {
        return Ok(None);
    };
    let deleted:bool=core.backup_db(|db|Ok(db.query_row("SELECT remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|r.get(0)).optional()?.unwrap_or(true)))?;
    if deleted {
        return core.asset_transcript(local);
    }
    let pending:Option<String>=core.backup_db(|db|Ok(db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|r.get(0)).optional()?))?;
    if pending
        .map(|v| serde_json::from_str::<PendingChanges>(&v))
        .transpose()?
        .is_some_and(|p| p.transcript_dirty())
    {
        return core.asset_transcript(local);
    }
    let record = cached(core, space, local)?;
    if record.deleted {
        return core.asset_transcript(local);
    }
    let Some(reference) = record.transcript else {
        return core.asset_transcript(local);
    };
    value.asset_id = record.asset_id;
    if format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)) == reference.sha256 {
        return core.asset_transcript(local);
    }
    Ok(None)
}
pub fn cached_cover(core: &Core, space: &str, local: &str) -> Result<Option<Vec<u8>>> {
    let state = core.backup_db(|db| local_cover(db, local))?;
    if state.dirty {
        return Ok(state.body);
    }
    let Some(body) = state.body else {
        return Ok(None);
    };
    let raw: Option<String> = core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",
                params![space, local],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    let record = raw
        .map(|v| serde_json::from_str::<AssetSnapshot>(&v))
        .transpose()?;
    if record
        .and_then(|r| r.cover)
        .is_none_or(|r| format!("{:x}", Sha256::digest(&body)) == r.sha256)
    {
        return Ok(Some(body));
    }
    Ok(None)
}
