//! Current local release, deletion and history controls.
use super::{repository, restore, scheduler};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use std::sync::Arc;
async fn stop_readers(core: &Core, id: &str) -> Result<()> {
    core.cancel_asset_restore(id)?;
    core.cancel_asset_transcription(id).await
}

pub(crate) async fn remove_local(core: &Core, id: &str) -> Result<()> {
    remove_local_checked(core, id, |_| Ok(())).await
}
pub(crate) async fn remove_local_checked(
    core: &Core,
    id: &str,
    check: impl Fn(&rusqlite::Connection) -> Result<()>,
) -> Result<()> {
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
        core.backup_db(|db| {
            check(db)?;
            let shared: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_local_files WHERE asset_id<>?1 AND path=?2)",
                params![id, path.to_string_lossy()],
                |r| r.get(0),
            )?;
            if !shared {
                std::fs::remove_file(&path).context("IO_ERROR")?;
            }
            Ok(())
        })?;
    }
    cleanup_restore_files(core, id).await?;
    core.mark_asset_missing(id)
}

async fn cleanup_restore_files(core: &Core, id: &str) -> Result<()> {
    for op in scheduler::unfinished_restores(core, id)? {
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
                        .backup_asset(id)?
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
    super::sync_delete::release(core, id).await
}
pub async fn delete_task(core: &Arc<Core>, task_id: &str) -> Result<()> {
    let task = core
        .list()
        .into_iter()
        .find(|t| t.id == task_id)
        .context("TASK_NOT_FOUND")?;
    if let Some(id) = &task.asset_id {
        if let Some(current) = super::spaces::get(core)? {
            let member:bool=core.backup_db(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2)",params![current.space_id,id],|r|r.get(0))?))?;
            if current.enabled && member {
                return super::sync_delete::delete(core, &current.space_id, id).await.map(|_| ());
            } else {
                stop_readers(core, id).await?;
                let _writer = core.backup_write_lock.write().await;
                let _restore = core.backup_restore_lock.lock().await;
                remove_local(core, id).await?;
            }
            return core.remove_after_transcription(task_id, false).await;
        }
        stop_readers(core, id).await?;
        let _writer = core.backup_write_lock.write().await;
        let _restore = core.backup_restore_lock.lock().await;
        remove_local(core, id).await?;
        core.remove_after_transcription(task_id, false).await
    } else {
        core.remove_after_transcription(task_id, true).await
    }
}

pub fn hide_history(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db|{db.execute("INSERT OR IGNORE INTO asset_hidden_history SELECT asset_id FROM media_assets WHERE asset_id=?",[id])?;Ok(())})
}
