//! Shared local-file verification and the current provider restore entry point.
use super::repository;
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;
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
        .backup_asset(id)?
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
    let space = super::spaces::get(core)?
        .filter(|s| s.enabled)
        .context("BACKUP_SPACE_INACTIVE")?;
    let provider = super::spaces::provider(core, &space.space_id)?;
    super::sync_read::ensure(core, provider.as_ref(), id).await
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

pub fn cancel(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db|{let tx=db.unchecked_transaction()?;
        tx.execute("UPDATE backup_operations SET state='canceled',last_error='CANCELED' WHERE asset_id=? AND kind='restore' AND state IN ('pending','running','blocked')",[id])?;
        tx.execute("UPDATE asset_local_files SET local_state='missing' WHERE asset_id=? AND local_state='restoring'",[id])?;
        tx.commit()?;Ok(())
    })
}
