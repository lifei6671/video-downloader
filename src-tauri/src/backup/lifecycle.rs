//! Local release and durable, exact-key deletion. No bucket-wide cleanup.
use super::{
    model::{Role, Space},
    record::{Deleted, Record},
    repository, restore,
    scheduler::{self, Operation},
    store::{Client, Condition, ObjectStore, Write},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde_json::json;
use std::sync::Arc;

fn space(core: &Core, id: &str) -> Result<Space> {
    core.backup_db(repository::spaces)?
        .into_iter()
        .find(|s| s.space_id == id && s.enabled)
        .context("BACKUP_SPACE_DISABLED")
}

async fn stop_readers(core: &Core, id: &str) -> Result<()> {
    core.cancel_asset_restore(id)?;
    core.cancel_asset_transcription(id).await
}

pub(crate) async fn remove_local(core: &Core, id: &str) -> Result<()> {
    remove_local_checked(core,id,|_|Ok(())).await
}
pub(crate) async fn remove_local_checked(core:&Core,id:&str,check:impl Fn(&rusqlite::Connection)->Result<()>)->Result<()> {
    // Before the first content hash exists, retain the source captured at registration.
    let registered: (Option<String>, Option<String>, Option<String>) = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT path,sha256,source_json FROM asset_local_files WHERE asset_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    })?;
    if let (Some(path), None, Some(source)) = registered {
        let path = std::path::Path::new(&path);
        if restore::regular(path)?.is_some()
            && serde_json::to_string(&crate::desktop_core::transcription::Source::read(path)?)?
                != source
        {
            bail!("BACKUP_LOCAL_CHANGED");
        }
    }
    // local() verifies the registered source and content before unlinking it.
    if let Some(path) = restore::local(core, id).await? {
        core.backup_db(|db|{
            check(db)?;
            let shared:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM asset_local_files WHERE asset_id<>?1 AND path=?2)",params![id,path.to_string_lossy()],|r|r.get(0))?;
            if !shared {std::fs::remove_file(&path).context("IO_ERROR")?;}
            Ok(())
        })?;
    }
    cleanup_restore_files(core, id).await?;
    core.mark_asset_missing(id)
}

async fn cleanup_restore_files(core: &Core, id: &str) -> Result<()> {
    let operations = scheduler::operations(core)?.into_iter().filter(|o| {
        o.asset_id.as_deref() == Some(id) && o.kind == "restore" && o.state != "completed"
    });
    for op in operations {
        let Some(raw) = op.payload["part"].as_str() else {
            continue;
        };
        let directory = restore::staging_directory(&core.settings().download_directory)?;
        let part = directory.join(format!("{}.part", op.operation_id));
        if part != std::path::Path::new(raw) {
            bail!("BACKUP_RESTORE_DIRECTORY_CHANGED");
        }
        let exists = restore::regular(&part)?.is_some();
        // Only a committed move with no remaining part can own this target.
        if !exists && op.phase == "committing" {
            if let Some(raw) = op.payload["target"].as_str() {
                let target = std::path::PathBuf::from(raw);
                if target.parent() != directory.parent() {
                    bail!("BACKUP_UNSAFE_FILE");
                }
                if restore::regular(&target)?.is_some() {
                    let path = target.clone();
                    let cancel = core.backup_cancellation();
                    let (hash, source) = tokio::task::spawn_blocking(move || {
                        repository::fingerprint_with_cancel(&path, || *cancel.borrow())
                    })
                    .await??;
                    let asset = core
                        .backup_assets()?
                        .into_iter()
                        .find(|a| a.asset_id == id)
                        .context("BACKUP_ASSET_NOT_FOUND")?;
                    if asset.content_sha256.as_deref() != Some(&hash)
                        || source["size"].as_u64() != Some(asset.content_bytes)
                    {
                        bail!("BACKUP_LOCAL_CHANGED");
                    }
                    std::fs::remove_file(target).context("IO_ERROR")?;
                }
            }
        }
        if exists {
            std::fs::remove_file(part).context("IO_ERROR")?;
        }
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET phase='local_cleaned',payload_json='{}' WHERE operation_id=?",[&op.operation_id])?;Ok(())})?;
    }
    Ok(())
}

pub async fn release(core: &Arc<Core>, id: &str) -> Result<()> {
    if super::spaces::get(core)?.is_some_and(|s|s.protocol==2) {return super::sync_delete::release(core,id).await;}
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_LOCAL_RELEASED' WHERE asset_id=? AND kind='upload' AND state IN ('pending','running','retry_wait','canceled')",[id])?;Ok(())})?;
    stop_readers(core, id).await?;
    let _writer = core.backup_write_lock.lock().await;
    let _restore = core.backup_restore_lock.lock().await;
    remove_local(core, id).await
}

/// Persist intent before stopping workers or touching local/remote files.
pub fn intent(core: &Core, space_id: &str, id: &str) -> Result<String> {
    let space = space(core, space_id)?;
    if space.role != Role::Primary {
        bail!("BACKUP_READ_ONLY");
    }
    core.backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == id)
        .context("BACKUP_ASSET_NOT_FOUND")?;
    core.backup_db(|db| {
        let tx=db.unchecked_transaction()?;
        let payload=json!({"endpoint":space.endpoint,"bucket":space.bucket,"prefix":space.prefix});
        tx.execute("INSERT OR IGNORE INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'delete','pending','local',?4)",params![uuid::Uuid::new_v4().to_string(),space_id,id,payload.to_string()])?;
        let operation:String=tx.query_row("SELECT operation_id FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='delete' AND state NOT IN ('completed','canceled')",params![space_id,id],|r|r.get(0))?;
        tx.execute("UPDATE media_assets SET asset_state='deleted' WHERE asset_id=?",[id])?;
        tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_ASSET_DELETED' WHERE asset_id=? AND kind='upload' AND state<>'completed'",[id])?;
        // References and every checkpointed immutable version remain durable until cleanup.
        tx.execute("INSERT OR IGNORE INTO asset_remote_objects(space_id,asset_id,record_key,remote_state) VALUES(?1,?2,?3,'pending_delete')",params![space_id,id,format!("{}records/{id}.json",space.prefix)])?;
        tx.execute("UPDATE asset_remote_objects SET remote_state='pending_delete' WHERE space_id=?1 AND asset_id=?2",params![space_id,id])?;
        for column in ["media_key","cover_key","transcript_key"] {
            tx.execute(&format!("INSERT OR IGNORE INTO backup_owned_objects SELECT space_id,asset_id,{column} FROM asset_remote_objects WHERE space_id=?1 AND asset_id=?2 AND {column} IS NOT NULL"),params![space_id,id])?;
        }
        tx.commit()?;Ok(operation)
    })
}

pub async fn delete(core: &Arc<Core>, space_id: &str, id: &str) -> Result<()> {
    let operation = intent(core, space_id, id)?;
    let op = scheduler::operations(core)?
        .into_iter()
        .find(|o| o.operation_id == operation)
        .context("BACKUP_INVALID_OPERATION")?;
    prepare_local(core, &op).await
}

pub async fn prepare_local(core: &Arc<Core>, op: &Operation) -> Result<()> {
    let id = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    stop_readers(core, id).await?;
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
    if let Err(error) = remove_local(core, id).await {
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1",
                params![op.operation_id, error.to_string()],
            )?;
            Ok(())
        })?;
        return Err(error);
    }
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state=CASE WHEN state='running' THEN 'running' ELSE 'pending' END,phase='tombstone',last_error=NULL WHERE operation_id=?",[&op.operation_id])?;Ok(())})
}

fn own_key(space: &Space, asset: &str, key: &str) -> Result<()> {
    if ![
        format!("{}media/{asset}/", space.prefix),
        format!("{}transcripts/{asset}/", space.prefix),
    ]
    .iter()
    .any(|p| key.starts_with(p))
        || key.split('/').any(|p| p == ".." || p == ".")
    {
        bail!("BACKUP_INVALID_OBJECT_KEY");
    }
    Ok(())
}

fn writable(core: &Core, op: &Operation) -> Result<()> {
    core.backup_db(|db|{let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations o JOIN backup_spaces s USING(space_id) WHERE o.operation_id=? AND o.kind='delete' AND o.state='running' AND s.enabled=1 AND s.role='primary')",[&op.operation_id],|r|r.get(0))?;if !valid {bail!("CANCELED");}Ok(())})
}
async fn write<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    op: &Operation,
    action: Write,
) -> Result<String> {
    writable(core, op)?;
    client.write(action).await
}

pub async fn remote<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    op: &Operation,
) -> Result<()> {
    if client.space().role != Role::Primary {
        bail!("BACKUP_READ_ONLY");
    }
    prepare_local(core, op).await?;
    let completed=core.backup_db(|db|{
        let state:String=db.query_row("SELECT state FROM backup_operations WHERE operation_id=?",[&op.operation_id],|r|r.get(0))?;
        if state=="completed" {return Ok(true);}
        db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=? AND state IN ('pending','retry_wait')",[&op.operation_id])?;
        Ok(false)
    })?;
    if completed {
        return Ok(());
    }
    writable(core, op)?;
    let mut cancel = core.backup_cancellation();
    let work = remote_body(core, client, op);
    tokio::pin!(work);
    loop {
        tokio::select! {result=&mut work=>return result,_=cancel.changed()=>bail!("CANCELED"),_=tokio::time::sleep(std::time::Duration::from_millis(100))=>writable(core,op)?,}
    }
}

async fn remote_body<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    op: &Operation,
) -> Result<()> {
    if client.space().role != Role::Primary {
        bail!("BACKUP_READ_ONLY");
    }
    let _writer = core.backup_write_lock.lock().await;
    let id = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    let space = space(core, &op.space_id)?;
    if space.space_id != client.space().space_id
        || op.payload["endpoint"] != space.endpoint
        || op.payload["bucket"] != space.bucket
        || op.payload["prefix"] != space.prefix
    {
        bail!("BACKUP_SPACE_CHANGED");
    }
    let library = client.connect().await?;
    let key = format!("{}records/{id}.json", space.prefix);
    let mut tombstone = None;
    for _ in 0..3 {
        let current = client.get(&key).await?;
        let mut revision = 1;
        let mut condition = Condition::Absent;
        if let Some(current) = current {
            let record: Record = serde_json::from_slice(&current.body)?;
            record.validate(&library, &space.prefix)?;
            if record.asset_id() != id {
                bail!("BACKUP_INVALID_RECORD");
            }
            if let Record::Deleted(_) = record {
                tombstone = Some(record);
                break;
            }
            if let Record::Active(active) = &record {
                core.backup_db(|db| {
                    let tx = db.unchecked_transaction()?;
                    for object in std::iter::once(&active.media)
                        .chain(active.cover.iter())
                        .chain(active.transcript.iter())
                    {
                        tx.execute(
                            "INSERT OR IGNORE INTO backup_owned_objects VALUES(?1,?2,?3)",
                            params![space.space_id, id, object.key],
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })?;
            }
            revision = record.revision() + 1;
            condition = Condition::Match(current.etag);
        }
        let record = Record::Deleted(Deleted {
            schema_version: 1,
            library_id: library.library_id.clone(),
            asset_id: id.into(),
            record_revision: revision,
            writer_epoch: library.writer_epoch,
            deleted_at: chrono::Utc::now().to_rfc3339(),
            delete_operation_id: op.operation_id.clone(),
        });
        record.validate(&library, &space.prefix)?;
        match write(
            core,
            client,
            op,
            Write::Put {
                key: key.clone(),
                body: serde_json::to_vec(&record)?,
                condition,
            },
        )
        .await
        {
            Ok(_) => {
                tombstone = Some(record);
                break;
            }
            Err(e) if e.to_string() == "BACKUP_PRECONDITION_FAILED" => continue,
            Err(e) => return Err(e),
        }
    }
    let record = tombstone.context("BACKUP_PRECONDITION_FAILED")?;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET phase='objects' WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })?;
    let keys: Vec<String> = core.backup_db(|db| {
        let mut q = db.prepare(
            "SELECT object_key FROM backup_owned_objects WHERE space_id=?1 AND asset_id=?2",
        )?;
        let rows = q.query_map(params![space.space_id, id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    for key in keys {
        own_key(&space, id, &key)?;
        for upload_id in client.uploads(&key).await? {
            match write(
                core,
                client,
                op,
                Write::AbortMultipart {
                    key: key.clone(),
                    upload_id,
                },
            )
            .await
            {
                Ok(_) => {}
                Err(e) if e.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                Err(e) => return Err(e),
            }
        }
        write(core, client, op, Write::Delete { key: key.clone() }).await?;
        if client.head(&key).await?.is_some() || !client.uploads(&key).await?.is_empty() {
            bail!("BACKUP_CLEANUP_PENDING");
        }
    }
    writable(core, op)?;
    core.backup_db(|db|{let tx=db.unchecked_transaction()?;
        tx.execute("UPDATE backup_operations SET state='completed',phase='completed',last_error=NULL WHERE operation_id=?",[&op.operation_id])?;
        tx.execute("UPDATE asset_remote_objects SET remote_state='deleted',media_key=NULL,cover_key=NULL,transcript_key=NULL,record_revision=?3 WHERE space_id=?1 AND asset_id=?2",params![space.space_id,id,i64::try_from(record.revision())?])?;
        tx.execute("INSERT INTO asset_remote_records VALUES(?1,?2,?3) ON CONFLICT(space_id,asset_id) DO UPDATE SET record_json=excluded.record_json",params![space.space_id,id,serde_json::to_string(&record)?])?;
        tx.commit()?;Ok(())})
}

pub async fn delete_task(core: &Arc<Core>, task_id: &str) -> Result<()> {
    let task = core
        .list()
        .into_iter()
        .find(|t| t.id == task_id)
        .context("TASK_NOT_FOUND")?;
    if let Some(id) = &task.asset_id {
        if let Some(current)=super::spaces::get(core)?.filter(|s|s.protocol==2) {
            let member:bool=core.backup_db(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2)",params![current.space_id,id],|r|r.get(0))?))?;
            if current.enabled&&member {super::sync_delete::delete(core,&current.space_id,id).await?;}
            else {
                stop_readers(core,id).await?;
                let _writer=core.backup_write_lock.lock().await;let _restore=core.backup_restore_lock.lock().await;
                remove_local(core,id).await?;
            }
            return core.remove_after_transcription(task_id,false).await;
        }
        // An unupgraded or unselected library cannot publish through the legacy writer path.
        stop_readers(core,id).await?;
        let _writer=core.backup_write_lock.lock().await;
        let _restore=core.backup_restore_lock.lock().await;
        remove_local(core,id).await?;
        core.remove_after_transcription(task_id,false).await
    } else {
        core.remove_after_transcription(task_id, true).await
    }
}

pub fn hide_history(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db|{db.execute("INSERT OR IGNORE INTO asset_hidden_history SELECT asset_id FROM media_assets WHERE asset_id=?",[id])?;Ok(())})
}
