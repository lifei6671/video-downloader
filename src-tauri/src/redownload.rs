use super::Task;
use crate::files::{move_noclobber, validate_album, video_recovery_valid, AlbumFile};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub(super) fn original(db: &Connection, id: &str) -> Result<Option<Task>> {
    let snapshot: Option<String> = db
        .query_row(
            "SELECT original FROM download_replacements WHERE task_id=?",
            [id],
            |row| row.get(0),
        )
        .optional()
        .context("DATABASE_ERROR")?;
    snapshot
        .map(|snapshot| serde_json::from_str(&snapshot).context("DATABASE_ERROR"))
        .transpose()
}

pub(super) fn remember(db: &Connection, original: &Task, preview_id: Option<&str>) -> Result<()> {
    let mut original = original.clone();
    if let Some(video) = original.video.as_mut() {
        video["cover_url"] = serde_json::Value::Null;
    }
    db.execute(
        "INSERT INTO download_replacements(task_id,original) VALUES(?1,?2)",
        params![original.id, serde_json::to_string(&original)?],
    )
    .context("DATABASE_ERROR")?;
    db.execute("DELETE FROM commit_intents WHERE task_id=?", [&original.id])
        .context("DATABASE_ERROR")?;
    if let Some(preview_id) = preview_id {
        db.execute("DELETE FROM download_tasks WHERE id=?", [preview_id])
            .context("DATABASE_ERROR")?;
    }
    Ok(())
}

pub(super) fn directory(original: &Task) -> Result<PathBuf> {
    original
        .path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_owned)
        .context("FILE_MISSING")
}

fn parked_original(source: &Path) -> Result<PathBuf> {
    Ok(source
        .parent()
        .context("INVALID_STAGING_PATH")?
        .join("replaced-original"))
}

fn only_album_files(directory: &Path, files: &[AlbumFile]) -> Result<bool> {
    let expected = files.len() + files.iter().filter(|file| file.video.is_some()).count();
    let entries = std::fs::read_dir(directory)
        .context("IO_ERROR")?
        .try_fold(0, |count, entry| entry.map(|_| count + 1))
        .context("IO_ERROR")?;
    Ok(entries == expected)
}

pub(super) fn replace(source: &Path, target: &Path, original: &Task) -> Result<()> {
    let previous = original.path.as_deref().context("FILE_MISSING")?;
    let album = !original.files.is_empty();
    if album {
        if validate_album(previous, &original.files).is_err() {
            bail!("REDOWNLOAD_SOURCE_CHANGED");
        }
    } else if !video_recovery_valid(previous, original.bytes) {
        bail!("REDOWNLOAD_SOURCE_CHANGED");
    }
    let parked = parked_original(source)?;
    move_noclobber(previous, &parked)?;
    if album && !only_album_files(&parked, &original.files)? {
        bail!("REDOWNLOAD_EXTRA_FILES");
    }
    move_noclobber(source, target)
}

// A replacement is committed only when the new task and removal of its
// original snapshot commit together. Until then the staged original wins.
pub(super) fn rollback(db: &Connection, original: &Task) -> Result<()> {
    let intent = db
        .query_row(
            "SELECT source,target,size,manifest FROM commit_intents WHERE task_id=?",
            [&original.id],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .context("DATABASE_ERROR")?;
    let Some((source, target, size, manifest)) = intent else {
        return Ok(());
    };
    let size = u64::try_from(size).context("DATABASE_ERROR")?;
    let parked = parked_original(&source)?;
    if parked.try_exists().context("IO_ERROR")? {
        if !source.try_exists().context("IO_ERROR")? && target.try_exists().context("IO_ERROR")? {
            let valid = match manifest {
                Some(manifest) => {
                    let files: Vec<AlbumFile> =
                        serde_json::from_str(&manifest).context("DATABASE_ERROR")?;
                    validate_album(&target, &files).is_ok_and(|bytes| bytes == size)
                        && only_album_files(&target, &files)?
                }
                None => video_recovery_valid(&target, size),
            };
            if !valid {
                bail!("REDOWNLOAD_RESTORE_FAILED");
            }
            move_noclobber(&target, &source)?;
        }
        move_noclobber(&parked, original.path.as_deref().context("FILE_MISSING")?)?;
    }
    db.execute("DELETE FROM commit_intents WHERE task_id=?", [&original.id])
        .context("DATABASE_ERROR")?;
    Ok(())
}

pub(super) fn forget(db: &Connection, id: &str) -> Result<()> {
    db.execute("DELETE FROM download_replacements WHERE task_id=?", [id])
        .context("DATABASE_ERROR")?;
    Ok(())
}

pub(super) fn retire_local_asset(db: &Connection, original: &Task) -> Result<()> {
    if let Some(asset) = original.asset_id.as_deref() {
        db.execute(
            "INSERT OR IGNORE INTO asset_hidden_history(asset_id) VALUES(?)",
            [asset],
        )
        .context("DATABASE_ERROR")?;
        db.execute(
            "UPDATE media_assets SET task_id=NULL WHERE asset_id=? AND task_id=?",
            params![asset, original.id],
        )
        .context("DATABASE_ERROR")?;
        db.execute("UPDATE asset_local_files SET path=NULL,local_state='missing' WHERE asset_id=? AND path=?",
            params![asset, original.path.as_ref().map(|path| path.to_string_lossy())]).context("DATABASE_ERROR")?;
    }
    Ok(())
}
