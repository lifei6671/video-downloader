use super::{cleanup_staging, transcription::Source, Core};
use crate::{
    library::{self, ImportedVideo, LocalMetadata},
    python_bridge::PythonBridge,
};
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

pub(super) struct ActiveImport {
    pub operation: super::workbench::WorkbenchOperation,
    pub cancel: watch::Sender<bool>,
    pub done: watch::Receiver<bool>,
}

impl Core {
    pub async fn import_local_video(self: &Arc<Self>, path: PathBuf) -> Result<ImportedVideo> {
        self.probe_local_video(path, None).await
    }

    pub async fn relink_local_video(
        self: &Arc<Self>,
        asset_id: String,
        file_id: String,
        path: PathBuf,
    ) -> Result<ImportedVideo> {
        self.probe_local_video(path, Some((asset_id, file_id)))
            .await
    }

    async fn probe_local_video(
        self: &Arc<Self>,
        path: PathBuf,
        target: Option<(String, String)>,
    ) -> Result<ImportedVideo> {
        let source = Source::read(&path)?;
        if !source
            .path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| {
                ["mp4", "mov", "mkv", "webm"]
                    .iter()
                    .any(|ext| v.eq_ignore_ascii_case(ext))
            })
        {
            bail!("LOCAL_VIDEO_INVALID");
        }
        let (reply, response) = oneshot::channel();
        let id = Uuid::new_v4().to_string();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if inner.video_import.is_some() {
                bail!("LIBRARY_IMPORT_BUSY");
            }
            if inner.dependency_cancel.is_some() {
                bail!("DEPENDENCY_BUSY");
            }
            if let Some((asset_id, file_id)) = &target {
                library::asset_file(&inner.db, asset_id, file_id)?;
            }
            let (cancel, mut cancellation) = watch::channel(false);
            let (done, finished) = watch::channel(false);
            inner.last_import_failure = None;
            inner.video_import = Some(ActiveImport {
                operation: super::workbench::WorkbenchOperation {
                    id: id.clone(),
                    kind: "import",
                    title: if target.is_some() {
                        "重新关联视频"
                    } else {
                        "导入视频"
                    }
                    .into(),
                    state: "running".into(),
                    phase: None,
                    error: None,
                    asset_id: target.as_ref().map(|v| v.0.clone()),
                    file_id: target.as_ref().map(|v| v.1.clone()),
                    task_id: None,
                },
                cancel,
                done: finished,
            });
            let core = self.clone();
            inner.workers.retain(|worker| !worker.is_finished());
            inner.workers.push(tokio::spawn(async move {
            let stage_id=format!("import-{id}");
            let outcome: Result<LocalMetadata> = async {
                let stage=tempfile::Builder::new().prefix(&format!("yingqu-{stage_id}-")).tempdir().context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT INTO task_staging VALUES(?1,?2)",rusqlite::params![stage_id,stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let result=PythonBridge::operation_cancellable(&core.program,&core.args,&id,"probe_local_video",
                    json!({"source":source.path,"staging_dir":stage.path(),"runtime_directory":core.runtime_directory}), |_|{}, async {let _=cancellation.wait_for(|v|*v).await;}).await;
                // The bridge has reaped FFprobe before its owned files are removed.
                cleanup_staging(&core.inner.lock().unwrap().db,&stage_id,stage.path())?;
                serde_json::from_value(result?).context("INVALID_ENGINE_RESULT")
            }.await;
            let mut inner=core.inner.lock().unwrap();
            let outcome=outcome.and_then(|metadata| {
                if inner.closing || *cancellation.borrow() { bail!("CANCELED"); }
                if Source::read(&source.path).ok().as_ref()!=Some(&source) { bail!("LIBRARY_SOURCE_CHANGED"); }
                match target {
                    Some((asset_id,file_id))=>library::relink_local(&inner.db,&asset_id,&file_id,&source,&metadata),
                    None=>library::register_local(&inner.db,&source,&metadata),
                }
            });
            if let Err(error) = &outcome {
                if error.to_string() != "CANCELED" {
                    let mut operation = inner.video_import.as_ref().unwrap().operation.clone();
                    operation.state = "failed".into(); operation.error = Some(error.to_string());
                    inner.last_import_failure = Some(operation);
                }
            }
            inner.video_import=None;
            (core.emit)(super::TaskEvent::LibraryChanged(super::library_api::LibraryChange{scope:super::library_api::LibraryScope::Operations,asset_ids:vec![]}));
            if let Ok(imported)=&outcome {(core.emit)(super::TaskEvent::LibraryChanged(super::library_api::LibraryChange{scope:super::library_api::LibraryScope::Assets,asset_ids:vec![imported.asset_id.clone()]}));}
            let _=reply.send(outcome);
            let _=done.send(true);
        }));
        }
        (self.emit)(super::TaskEvent::LibraryChanged(
            super::library_api::LibraryChange {
                scope: super::library_api::LibraryScope::Operations,
                asset_ids: vec![],
            },
        ));
        response.await.context("IO_ERROR")?
    }

    pub async fn cancel_local_import(&self) {
        let finished = {
            let inner = self.inner.lock().unwrap();
            inner.video_import.as_ref().map(|active| {
                let _ = active.cancel.send(true);
                active.done.clone()
            })
        };
        if let Some(mut finished) = finished {
            let _ = finished.wait_for(|v| *v).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_core::transcription::Snapshot;
    use std::{path::Path, time::Duration};

    fn core(root: &Path) -> Arc<Core> {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) {
            ".venv/Scripts/python.exe"
        } else {
            ".venv/bin/python"
        });
        Core::open(
            &root.join("tasks.sqlite"),
            root.join("downloads"),
            python.to_string_lossy().into(),
            vec![project
                .join("tests/fixtures/desktop_engine.py")
                .to_string_lossy()
                .into()],
            |_| {},
        )
        .unwrap()
    }

    async fn finished(app: &Core, asset: &str, file: &str) -> Snapshot {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = app.asset_transcript(asset, file).unwrap().unwrap();
                if snapshot.state != "running" && app.inner.lock().unwrap().transcription.is_none()
                {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn local_import_is_atomic_deduplicated_and_does_not_create_download_tasks() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let path = root.path().join("speech.mp4");
        std::fs::write(&path, b"1234").unwrap();
        let first = app.import_local_video(path.clone()).await.unwrap();
        assert!(!first.already_present);
        let second = app.import_local_video(path.clone()).await.unwrap();
        assert!(second.already_present);
        assert_eq!(first.asset_id, second.asset_id);
        let alias = root.path().join("alias.mp4");
        std::fs::hard_link(&path, &alias).unwrap();
        assert_eq!(
            app.import_local_video(alias).await.unwrap().file_id,
            first.file_id
        );
        assert!(app.list().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"1234");
        {
            let inner = app.inner.lock().unwrap();
            let asset = library::asset(&inner.db, &first.asset_id).unwrap();
            assert!(asset.platform.is_none() && asset.canonical_url.is_none());
            let file = library::asset_file(&inner.db, &first.asset_id, &first.file_id).unwrap();
            assert_eq!(file.width, Some(1280));
            assert!(!file.missing);
        }
        for (name, expected) in [
            ("invalid.mp4", "LOCAL_VIDEO_INVALID"),
            ("changed.mp4", "LIBRARY_SOURCE_CHANGED"),
            ("video.m3u8", "LOCAL_VIDEO_INVALID"),
        ] {
            let file = root.path().join(name);
            std::fs::write(&file, b"1234").unwrap();
            assert_eq!(
                app.import_local_video(file).await.unwrap_err().to_string(),
                expected
            );
        }
        let failed = app.workbench().unwrap().operations;
        assert!(failed.iter().any(|op| op.kind == "import"
            && op.state == "failed"
            && op.error.as_deref() == Some("LIBRARY_SOURCE_CHANGED")));
        assert_eq!(
            library::list_assets(&app.inner.lock().unwrap().db, false, 0, 100)
                .unwrap()
                .len(),
            1
        );
        app.shutdown().await;
    }

    #[tokio::test]
    async fn local_import_cancel_and_shutdown_reap_probe_without_partial_assets() {
        for shutdown in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let path = root.path().join("hold.mp4");
            std::fs::write(&path, b"1234").unwrap();
            let running = app.clone();
            let operation = tokio::spawn(async move { running.import_local_video(path).await });
            tokio::time::timeout(Duration::from_secs(10), async {
                while !root.path().join("probe-started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                app.import_local_video(root.path().join("hold.mp4"))
                    .await
                    .unwrap_err()
                    .to_string(),
                "LIBRARY_IMPORT_BUSY"
            );
            if shutdown {
                app.shutdown().await;
            } else {
                app.cancel_local_import().await;
            }
            assert_eq!(
                operation.await.unwrap().unwrap_err().to_string(),
                "CANCELED"
            );
            {
                let inner = app.inner.lock().unwrap();
                assert!(inner.video_import.is_none());
                assert!(library::list_assets(&inner.db, false, 0, 100)
                    .unwrap()
                    .is_empty());
                assert_eq!(
                    inner
                        .db
                        .query_row("SELECT count(*) FROM task_staging", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            }
            if !shutdown {
                app.shutdown().await;
            }
            drop(app);
            let app = core(root.path());
            assert!(app.list().is_empty());
            app.shutdown().await;
        }
    }

    #[tokio::test]
    async fn imported_asset_transcribes_without_history_and_stores_body_only_in_documents() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let path = root.path().join("speech.mp4");
        std::fs::write(&path, b"1234").unwrap();
        let imported = app.import_local_video(path.clone()).await.unwrap();
        app.start_asset_transcription(&imported.asset_id, &imported.file_id, false)
            .unwrap();
        let success = finished(&app, &imported.asset_id, &imported.file_id).await;
        assert_eq!(success.state, "completed");
        assert_eq!(success.result.unwrap().text, "欢迎体验。\nEnglish text.");
        {
            let inner = app.inner.lock().unwrap();
            let docs = library::documents(&inner.db, &imported.asset_id).unwrap();
            library::save_edited(
                &inner.db,
                &imported.asset_id,
                "我的整理稿",
                0,
                Some(&docs[0].id),
            )
            .unwrap();
            let json: String = inner
                .db
                .query_row(
                    "SELECT snapshot FROM asset_transcriptions WHERE file_id=?1",
                    [&imported.file_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["result"].is_null());
            assert_eq!(
                inner
                    .db
                    .query_row("SELECT count(*) FROM video_transcripts", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        let replacement = root.path().join("replacement.mp4");
        std::fs::write(&replacement, b"another video").unwrap();
        let relinked = app
            .relink_local_video(
                imported.asset_id.clone(),
                imported.file_id.clone(),
                replacement.clone(),
            )
            .await
            .unwrap();
        assert_eq!(relinked.file_id, imported.file_id);
        assert!(
            path.exists(),
            "original A remains unchanged when the asset points to B"
        );
        assert!(
            app.asset_transcript(&imported.asset_id, &imported.file_id)
                .unwrap()
                .unwrap()
                .source_changed
        );
        let invalid = root.path().join("invalid.mp4");
        std::fs::write(&invalid, b"not a video").unwrap();
        assert_eq!(
            app.relink_local_video(imported.asset_id.clone(), imported.file_id.clone(), invalid)
                .await
                .unwrap_err()
                .to_string(),
            "LOCAL_VIDEO_INVALID"
        );
        assert_eq!(
            library::asset_file(
                &app.inner.lock().unwrap().db,
                &imported.asset_id,
                &imported.file_id
            )
            .unwrap()
            .path,
            replacement.canonicalize().unwrap()
        );
        let mut path = replacement;
        for (name, state) in [
            ("fail.mp4", "failed"),
            ("silent.mp4", "no_speech"),
            ("hold.mp4", "canceled"),
        ] {
            let next = path.with_file_name(name);
            std::fs::rename(&path, &next).unwrap();
            library::relink_file(&app.inner.lock().unwrap().db, &imported.file_id, &next).unwrap();
            path = next;
            let running = app
                .start_asset_transcription(&imported.asset_id, &imported.file_id, true)
                .unwrap();
            if state == "canceled" {
                app.cancel_asset_transcription(&imported.file_id, &running.operation_id)
                    .await
                    .unwrap();
            }
            let result = finished(&app, &imported.asset_id, &imported.file_id).await;
            assert_eq!(result.state, state);
            assert!(result.result.is_some());
        }
        assert!(app.clear_history(None).unwrap().is_empty());
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert!(app
            .asset_transcript(&imported.asset_id, &imported.file_id)
            .unwrap()
            .unwrap()
            .result
            .is_some());
        {
            let inner = app.inner.lock().unwrap();
            let docs = library::documents(&inner.db, &imported.asset_id).unwrap();
            assert_eq!(docs.len(), 2);
            assert!(docs.iter().any(|doc| doc.text == "我的整理稿"));
        }
        std::fs::remove_file(&path).unwrap();
        assert!(
            app.asset_transcript(&imported.asset_id, &imported.file_id)
                .unwrap()
                .unwrap()
                .source_changed
        );
        assert_eq!(
            app.start_asset_transcription(&imported.asset_id, &imported.file_id, true)
                .err()
                .unwrap()
                .to_string(),
            "FILE_MISSING"
        );
        app.shutdown().await;
    }
}
