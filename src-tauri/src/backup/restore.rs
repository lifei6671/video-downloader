//! A single restore slot, durable range downloads and no-clobber file commits.
use super::{
    reconcile,
    record::{Active, Record},
    repository, scheduler, settings,
    store::{Client, ObjectStore},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

pub type Outcome = std::result::Result<PathBuf, String>;
pub(crate) struct Shared {
    pub result: watch::Receiver<Option<Outcome>>,
    pub cancel: watch::Sender<bool>,
}

pub(crate) fn regular(path: &Path) -> Result<Option<std::fs::Metadata>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("IO_ERROR"),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("BACKUP_UNSAFE_FILE");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            bail!("BACKUP_UNSAFE_FILE");
        }
    }
    Ok(Some(meta))
}

pub(crate) async fn fingerprint(
    path: PathBuf,
    cancel: watch::Receiver<bool>,
    stop: watch::Receiver<bool>,
    container: String,
) -> Result<(String, Value)> {
    tokio::task::spawn_blocking(move || {
        regular(&path)?.context("FILE_MISSING")?;
        let mut head = [0u8; 12];
        use std::io::Read;
        std::fs::File::open(&path)?
            .read_exact(&mut head)
            .context("INVALID_MEDIA_CONTENT")?;
        if !crate::files::video_header_valid(&container, &head) {
            bail!("INVALID_MEDIA_CONTENT");
        }
        repository::fingerprint_with_cancel(&path, || {
            *cancel.borrow() || *stop.borrow() || stop.has_changed().is_err()
        })
    })
    .await?
}

pub async fn local(core: &Core, id: &str) -> Result<Option<PathBuf>> {
    let asset = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == id)
        .context("BACKUP_ASSET_NOT_FOUND")?;
    let Some(path) = asset.path else {
        return Ok(None);
    };
    let Some(_) = regular(&path)? else {
        return Ok(None);
    };
    let source = serde_json::to_value(crate::desktop_core::transcription::Source::read(&path)?)?;
    let cached: (Option<String>, Option<String>) = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT sha256,source_json FROM asset_local_files WHERE asset_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    if cached.0.is_some() && cached.1.as_deref() == Some(&source.to_string()) {
        return Ok(Some(path));
    }
    let (_sender, stop) = watch::channel(false);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .context("INVALID_MEDIA_CONTENT")?
        .to_ascii_lowercase();
    let (hash, source) = fingerprint(path.clone(), core.backup_cancellation(), stop, ext).await?;
    let expected: Option<String> = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT sha256 FROM asset_local_files WHERE asset_id=?",
            [id],
            |r| r.get(0),
        )?)
    })?;
    if expected
        .or(asset.content_sha256)
        .is_some_and(|old| old != hash)
    {
        bail!("BACKUP_LOCAL_CHANGED");
    }
    core.commit_asset_file(id, &path, &hash, &source)?;
    Ok(Some(path))
}

pub async fn ensure_current(core: &Arc<Core>, id: &str) -> Result<PathBuf> {
    if let Some(path) = local(core, id).await? {
        return Ok(path);
    }
    if let Some(space)=super::spaces::get(core)?.filter(|s|s.enabled&&s.protocol==2) {
        let provider=super::spaces::provider(core,&space.space_id)?;
        return super::sync_read::ensure(core,provider.as_ref(),id).await;
    }
    let mut shutdown = core.backup_cancellation();
    let _slot = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),slot=core.backup_restore_lock.lock()=>slot};
    if let Some(path) = local(core, id).await? {
        return Ok(path);
    }
    let space_id:String=core.backup_db(|db|Ok(db.query_row("SELECT r.space_id FROM asset_remote_objects r JOIN backup_spaces s USING(space_id) WHERE r.asset_id=? AND s.enabled=1 ORDER BY s.rowid DESC LIMIT 1",[id],|r|r.get(0))?))?;
    let client = settings::client(core, &space_id)?;
    tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=restore(core, &client, id)=>result}
}

pub async fn ensure_with<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    id: &str,
) -> Result<PathBuf> {
    if let Some(path) = local(core, id).await? {
        return Ok(path);
    }
    let mut shutdown = core.backup_cancellation();
    let _slot = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),slot=core.backup_restore_lock.lock()=>slot};
    if let Some(path) = local(core, id).await? {
        return Ok(path);
    }
    tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=restore(core, client, id)=>result}
}

fn active(core: &Core, id: &str, operation: &str) -> Result<()> {
    if *core.backup_cancellation().borrow() {
        bail!("CANCELED");
    }
    core.backup_db(|db|{
        let good:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM media_assets a JOIN backup_operations o ON o.asset_id=a.asset_id JOIN backup_spaces s USING(space_id) WHERE a.asset_id=?1 AND a.asset_state='active' AND o.operation_id=?2 AND o.state='running' AND s.enabled=1)",params![id,operation],|r|r.get(0))?;
        if !good {bail!("CANCELED");}Ok(())
    })
}
fn checkpoint(core: &Core, id: &str, op: &str, phase: &str, payload: &Value) -> Result<()> {
    active(core, id, op)?;
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET phase=?2,payload_json=?3,updated_at=unixepoch() WHERE operation_id=?1 AND state='running'",params![op,phase,payload.to_string()])?;Ok(())})
}

async fn latest<S: ObjectStore>(core: &Core, client: &Client<S>, id: &str) -> Result<Active> {
    let library = client.connect().await?;
    let key = format!("{}records/{id}.json", client.space().prefix);
    let value = client.get(&key).await?.context("BACKUP_OBJECT_NOT_FOUND")?;
    let record: Record = serde_json::from_slice(&value.body).context("BACKUP_INVALID_RECORD")?;
    record.validate(&library, &client.space().prefix)?;
    if record.asset_id() != id {
        bail!("BACKUP_INVALID_RECORD");
    }
    reconcile::import(core, &client.space().space_id, &key, &value.etag, &record)?;
    match record {
        Record::Active(v) => Ok(*v),
        Record::Deleted(_) => bail!("BACKUP_ASSET_DELETED"),
    }
}

async fn restore<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    id: &str,
) -> Result<PathBuf> {
    let existing = scheduler::operations(core)?.into_iter().find(|o| {
        o.space_id == client.space().space_id
            && o.asset_id.as_deref() == Some(id)
            && o.kind == "restore"
            && o.state != "completed"
    });
    let (op, mut payload) = existing
        .map(|o| (o.operation_id, o.payload))
        .unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), json!({})));
    core.backup_db(|db|{let tx=db.unchecked_transaction()?;tx.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'restore','running','preparing',?4) ON CONFLICT(operation_id) DO UPDATE SET state='running',last_error=NULL",params![op,client.space().space_id,id,payload.to_string()])?;tx.execute("UPDATE asset_local_files SET local_state='restoring' WHERE asset_id=?",[id])?;tx.commit()?;Ok(())})?;
    let (stop, cancel) = watch::channel(false);
    let mut shutdown = core.backup_cancellation();
    let work = async {
        let record = latest(core, client, id).await?;
        transfer(core, client, &record, &op, &mut payload, cancel).await
    };
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            value=&mut work=>break value,
            _=shutdown.changed()=>{stop.send_replace(true);break Err(anyhow::anyhow!("CANCELED"));},
            _=tokio::time::sleep(Duration::from_millis(200))=>{if let Err(e)=active(core,id,&op) {stop.send_replace(true);break Err(e);}},
        }
    };
    if let Err(error) = &result {
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET state=CASE WHEN state='canceled' THEN state ELSE 'blocked' END,last_error=?2 WHERE operation_id=?1",params![op,error.to_string()])?;db.execute("UPDATE asset_local_files SET local_state='missing' WHERE asset_id=? AND local_state='restoring'",[id])?;Ok(())})?;
    }
    result
}

pub(crate) fn staging_directory(root: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(root).context("IO_ERROR")?;
    let root = root.canonicalize().context("IO_ERROR")?;
    let directory = root.join(".yingqu-restore");
    match std::fs::create_dir(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).context("IO_ERROR"),
    }
    let meta = std::fs::symlink_metadata(&directory)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || directory.canonicalize()? != directory {
        bail!("BACKUP_UNSAFE_FILE");
    }
    Ok(directory)
}

async fn transfer<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    record: &Active,
    op: &str,
    payload: &mut Value,
    cancel: watch::Receiver<bool>,
) -> Result<PathBuf> {
    let id = &record.asset_id;
    let directory = staging_directory(&core.settings().download_directory)?;
    let part = directory.join(format!("{op}.part"));
    if let Some(old) = payload["part"].as_str() {
        if Path::new(old) != part {
            bail!("BACKUP_RESTORE_DIRECTORY_CHANGED");
        }
    }
    payload["part"] = part.to_string_lossy().into_owned().into();
    // A prior atomic move may have succeeded before the SQLite commit.
    if let Some(target) = payload["target"].as_str().map(PathBuf::from) {
        if target.parent() != directory.parent() {
            bail!("BACKUP_UNSAFE_FILE");
        }
        if regular(&target)?.is_some() {
            let (hash, source) = fingerprint(
                target.clone(),
                core.backup_cancellation(),
                cancel.clone(),
                record.media.container.clone(),
            )
            .await?;
            if hash == record.media.sha256 && source["size"].as_u64() == Some(record.media.bytes) {
                return complete(core, client, record, op, &target, &hash, &source).await;
            }
            bail!("BACKUP_LOCAL_CHANGED");
        }
    }
    let head = client
        .head(&record.media.key)
        .await?
        .context("BACKUP_OBJECT_NOT_FOUND")?;
    if head.bytes != record.media.bytes {
        bail!("BACKUP_SIZE_MISMATCH");
    }
    let mut offset = regular(&part)?.map_or(0, |m| m.len());
    if payload["key"] != record.media.key || payload["etag"] != head.etag || offset > head.bytes {
        offset = 0;
    }
    payload["key"] = record.media.key.clone().into();
    payload["etag"] = head.etag.clone().into();
    payload["total_bytes"] = head.bytes.into();
    payload["completed_bytes"] = offset.into();
    checkpoint(core, id, op, "downloading", payload)?;
    if offset < head.bytes {
        let mut stream = client
            .stream(&record.media.key, offset, Some(head.etag.clone()))
            .await?;
        if offset > 0
            && stream.content_range.as_deref()
                != Some(&format!("bytes {offset}-{}/{}", head.bytes - 1, head.bytes))
        {
            offset = 0;
            stream = client
                .stream(&record.media.key, 0, Some(head.etag.clone()))
                .await?;
        }
        if stream.info.etag != head.etag || stream.info.bytes != head.bytes - offset {
            bail!("BACKUP_SIZE_MISMATCH");
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(offset == 0)
            .open(&part)
            .await
            .context("IO_ERROR")?;
        if offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        let mut buffer = vec![0; 256 * 1024];
        let mut last_checkpoint = offset;
        loop {
            active(core, id, op)?;
            let n = match tokio::time::timeout(
                Duration::from_secs(30),
                stream.reader.read(&mut buffer),
            )
            .await
            {
                Ok(Ok(n)) => n,
                _ => {
                    file.flush().await.context("IO_ERROR")?;
                    file.sync_all().await.context("IO_ERROR")?;
                    payload["completed_bytes"] = offset.into();
                    checkpoint(core, id, op, "downloading", payload)?;
                    bail!("BACKUP_NETWORK_ERROR");
                }
            };
            if n == 0 {
                break;
            }
            if offset + n as u64 > head.bytes {
                bail!("BACKUP_SIZE_MISMATCH");
            }
            file.write_all(&buffer[..n]).await.context("IO_ERROR")?;
            offset += n as u64;
            if offset - last_checkpoint >= 1024 * 1024 {
                file.flush().await?;
                payload["completed_bytes"] = offset.into();
                checkpoint(core, id, op, "downloading", payload)?;
                last_checkpoint = offset;
            }
        }
        file.flush().await?;
        file.sync_all().await?;
    }
    if offset != head.bytes {
        bail!("BACKUP_SIZE_MISMATCH");
    }
    checkpoint(core, id, op, "verifying", payload)?;
    let verified = fingerprint(
        part.clone(),
        core.backup_cancellation(),
        cancel,
        record.media.container.clone(),
    )
    .await;
    let (hash, source) = match verified {
        Ok(v) => v,
        Err(error) => {
            payload["etag"] = Value::Null;
            checkpoint(core, id, op, "verification_failed", payload)?;
            return Err(error);
        }
    };
    if hash != record.media.sha256 || source["size"].as_u64() != Some(record.media.bytes) {
        payload["etag"] = Value::Null;
        checkpoint(core, id, op, "verification_failed", payload)?;
        bail!("BACKUP_CHECKSUM_MISMATCH");
    }
    let newest = latest(core, client, id).await?;
    if newest.media != record.media {
        bail!("BACKUP_SOURCE_CHANGED");
    }
    active(core, id, op)?;
    let stem = Path::new(&record.original_filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let name = crate::naming::render("{content}", &json!({"title":stem}), "")?;
    let target = crate::naming::destination(
        directory.parent().context("IO_ERROR")?,
        &name,
        Some(&record.media.container),
    )?;
    payload["target"] = target.to_string_lossy().into_owned().into();
    payload["completed_bytes"] = offset.into();
    checkpoint(core, id, op, "committing", payload)?;
    tempfile::TempPath::try_from_path(part)
        .context("IO_ERROR")?
        .persist_noclobber(&target)
        .map_err(|error| {
            let _ = error.path.keep();
            anyhow::anyhow!("IO_ERROR")
        })?;
    let mut final_source = source;
    final_source["path"] = target.canonicalize()?.to_string_lossy().into_owned().into();
    complete(core, client, record, op, &target, &hash, &final_source).await
}

async fn complete<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    record: &Active,
    op: &str,
    path: &Path,
    hash: &str,
    source: &Value,
) -> Result<PathBuf> {
    active(core, &record.asset_id, op)?;
    // Revalidate immediately before registering an already committed file after a restart.
    let newest = latest(core, client, &record.asset_id).await?;
    if newest.media != record.media {
        bail!("BACKUP_SOURCE_CHANGED");
    }
    active(core, &record.asset_id, op)?;
    core.commit_asset_file(&record.asset_id, path, hash, source)?;
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL WHERE operation_id=?",[op])?;Ok(())})?;
    Ok(path.to_owned())
}

pub fn cancel(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db|{let tx=db.unchecked_transaction()?;
        tx.execute("UPDATE backup_operations SET state='canceled',last_error='CANCELED' WHERE asset_id=? AND kind='restore' AND state IN ('pending','running','blocked')",[id])?;
        tx.execute("UPDATE asset_local_files SET local_state='missing' WHERE asset_id=? AND local_state='restoring'",[id])?;
        tx.commit()?;Ok(())
    })
}
