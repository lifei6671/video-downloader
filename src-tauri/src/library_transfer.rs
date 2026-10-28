use super::{
    library_api::{LibraryChange, LibraryScope},
    Core, Status, TaskEvent,
};
use crate::library::{
    self, BackupReport, ContentFormat, ContentSelection, RestoreReport, TransferCallback,
};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

impl Core {
    pub fn export_library_content(
        &self,
        asset: &str,
        selection: &ContentSelection,
        format: ContentFormat,
        target: &Path,
    ) -> Result<()> {
        let (body, protected) = {
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            let body = library::export_content(&inner.db, asset, selection, format)
                .map_err(super::library_api::database_error)?;
            let mut query=inner.db.prepare("SELECT path FROM asset_files UNION SELECT json_extract(source_json,'$.path') FROM asset_documents WHERE source_json IS NOT NULL UNION SELECT json_extract(source_json,'$.path') FROM asset_clips UNION SELECT json_extract(source_json,'$.path') FROM asset_keyframes").context("DATABASE_ERROR")?;
            let protected: Vec<PathBuf> = query
                .query_map([], |r| Ok(PathBuf::from(r.get::<_, String>(0)?)))
                .context("DATABASE_ERROR")?
                .collect::<rusqlite::Result<_>>()
                .context("DATABASE_ERROR")?;
            (body, protected)
        };
        match format {
            ContentFormat::Json => {
                super::transcription::export_bytes(body.as_bytes(), target, &protected)
            }
            _ => super::transcription::export_text(&body, target, &protected),
        }
    }

    /// The caller may cancel by returning CANCELED from the progress callback.
    /// Large media copies run outside the state mutex and notify every 64 KiB.
    pub fn backup_library(
        &self,
        target: &Path,
        include_media: bool,
        notify: &mut TransferCallback<'_>,
    ) -> Result<BackupReport> {
        let snapshot = {
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            library::capture_backup(&inner.db, include_media).context("DATABASE_ERROR")?
        };
        library::write_backup(snapshot, target, &mut |progress| {
            if self.inner.lock().unwrap().closing {
                bail!("INVALID_STATE");
            }
            notify(progress)
        })
    }

    pub fn restore_library(
        &self,
        package: &Path,
        target: &Path,
        notify: &mut TransferCallback<'_>,
    ) -> Result<RestoreReport> {
        let ready = |inner: &super::Inner| -> Result<()> {
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if inner.video_import.is_some()
                || inner.transcription.is_some()
                || inner.tasks.iter().any(|t| {
                    matches!(
                        t.status,
                        Status::Parsing | Status::Queued | Status::Downloading
                    )
                })
            {
                bail!("LIBRARY_BUSY");
            }
            library::require_empty_library(&inner.db)
        };
        ready(&self.inner.lock().unwrap())?;
        let prepared = library::prepare_restore(package, target, &mut |progress| {
            if self.inner.lock().unwrap().closing {
                bail!("INVALID_STATE");
            }
            notify(progress)
        })?;
        let result = {
            let mut inner = self.inner.lock().unwrap();
            ready(&inner)?;
            library::commit_restore(&mut inner.db, prepared)?
        };
        (self.emit)(TaskEvent::LibraryChanged(LibraryChange {
            scope: LibraryScope::Assets,
            asset_ids: vec![],
        }));
        Ok(result)
    }
}
