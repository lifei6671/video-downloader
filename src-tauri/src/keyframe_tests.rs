use super::*;
use std::{fs, path::Path, time::Duration};

fn core(root: &Path) -> Arc<Core> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    Core::open(
        &root.join("test.sqlite"),
        root.join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into()],
        |_| {},
    )
    .unwrap()
}
fn seed(core: &Core, root: &Path, name: &str) -> library::ImportedVideo {
    let path = root.join(format!("{name}.mp4"));
    fs::write(&path, b"source video").unwrap();
    library::register_local(
        &core.inner.lock().unwrap().db,
        &Source::read(&path).unwrap(),
        &library::LocalMetadata {
            width: 90,
            height: 160,
            duration_ms: Some(1000),
        },
    )
    .unwrap()
}
async fn terminal(core: &Core) -> KeyframeSnapshot {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let s = core.keyframe_operation().unwrap();
            if s.state != "running" {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn completed_frames_survive_restart_and_failed_batches_leave_old_resources() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let item = seed(&app, root.path(), "valid");
    let start = app
        .start_keyframes(
            &item.asset_id,
            KeyframeRequest::Position {
                file_id: item.file_id.clone(),
                position_ms: 500,
            },
        )
        .unwrap();
    let done = terminal(&app).await;
    assert_eq!(done.state, "completed", "{:?}", done.error);
    assert_eq!(done.id, start.id);
    assert_eq!(done.completed, 1);
    let frame = app.library_keyframes(&item.asset_id).unwrap().remove(0);
    assert_eq!(frame.actual_ms, 500);
    assert!(!app
        .library_keyframe_image(&item.asset_id, &frame.data.id)
        .unwrap()
        .is_empty());
    for mode in ["bad-frame", "changed", "fail-frame"] {
        let bad = seed(&app, root.path(), mode);
        app.start_keyframes(
            &bad.asset_id,
            KeyframeRequest::Position {
                file_id: bad.file_id,
                position_ms: 500,
            },
        )
        .unwrap();
        assert_eq!(terminal(&app).await.state, "failed");
        assert!(app.library_keyframes(&bad.asset_id).unwrap().is_empty());
        assert_eq!(
            app.library_keyframes(&item.asset_id).unwrap()[0].data.id,
            frame.data.id
        );
    }
    let edited = app
        .save_library_document(&item.asset_id, "manual", 0, None)
        .unwrap();
    let original = app.library_asset(&item.asset_id).unwrap().files[0]
        .path
        .clone();
    let replacement = root.path().join("replacement.mp4");
    fs::write(&replacement, b"replacement").unwrap();
    library::relink_file(&app.inner.lock().unwrap().db, &item.file_id, &replacement).unwrap();
    assert_eq!(
        app.export_library_content(
            &item.asset_id,
            &library::ContentSelection::Document(edited.clone()),
            library::ContentFormat::Txt,
            &original
        )
        .unwrap_err()
        .to_string(),
        "TRANSCRIPTION_EXPORT_SOURCE"
    );
    assert_eq!(
        app.export_library_document(&item.asset_id, &edited, &original)
            .unwrap_err()
            .to_string(),
        "TRANSCRIPTION_EXPORT_SOURCE"
    );
    app.shutdown().await;
    drop(app);
    let reopened = core(root.path());
    assert_eq!(
        reopened.library_keyframes(&item.asset_id).unwrap()[0]
            .data
            .id,
        frame.data.id
    );
    assert!(reopened.library_keyframes(&item.asset_id).unwrap()[0].source_changed);
    reopened.shutdown().await;
}
#[tokio::test]
async fn cancel_and_shutdown_reap_engine_and_actual_descendant_before_staging_cleanup() {
    for shutdown in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let item = seed(&app, root.path(), "hold-frames");
        let run = app
            .start_keyframes(
                &item.asset_id,
                KeyframeRequest::Position {
                    file_id: item.file_id.clone(),
                    position_ms: 0,
                },
            )
            .unwrap();
        let marker = root.path().join("keyframes-started.json");
        tokio::time::timeout(Duration::from_secs(15), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let started: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        assert_eq!(
            app.start_keyframes(
                &item.asset_id,
                KeyframeRequest::Position {
                    file_id: item.file_id,
                    position_ms: 0
                }
            )
            .unwrap_err()
            .to_string(),
            "KEYFRAME_BUSY"
        );
        assert!(app
            .workbench()
            .unwrap()
            .operations
            .iter()
            .any(|op| op.kind == "keyframes" && op.asset_id.as_deref() == Some(&item.asset_id)));
        if shutdown {
            app.shutdown().await;
        } else {
            app.cancel_keyframes(&run.id).await.unwrap();
        }
        let ended = terminal(&app).await;
        assert_eq!(ended.state, "canceled", "{:?}", ended.error);
        assert!(app.library_keyframes(&item.asset_id).unwrap().is_empty());
        assert!(!Path::new(started["stage"].as_str().unwrap()).exists());
        assert_eq!(
            app.inner
                .lock()
                .unwrap()
                .db
                .query_row("SELECT count(*) FROM task_staging", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        #[cfg(windows)]
        for key in ["engine", "child"] {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, STILL_ACTIVE},
                System::Threading::{
                    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                },
            };
            // SAFETY: Test-owned PID only, handle checked before querying and closed once.
            unsafe {
                let handle = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    started[key].as_u64().unwrap() as u32,
                );
                if !handle.is_null() {
                    let mut code = 0;
                    assert_ne!(GetExitCodeProcess(handle, &mut code), 0);
                    CloseHandle(handle);
                    assert_ne!(code, STILL_ACTIVE as u32, "{key} survived cancellation");
                }
            }
        }
        println!(
            "reaped engine/child, actual_ffmpeg={}, shutdown={shutdown}",
            started["ffmpeg"]
        );
        if !shutdown {
            app.shutdown().await;
        }
    }
}

#[tokio::test]
async fn cleanup_failure_does_not_commit_images_and_restart_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let item = seed(&app, root.path(), "valid");
    app.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER fail_cleanup BEFORE DELETE ON task_staging BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    app.start_keyframes(
        &item.asset_id,
        KeyframeRequest::Position {
            file_id: item.file_id.clone(),
            position_ms: 500,
        },
    )
    .unwrap();
    assert_eq!(terminal(&app).await.state, "failed");
    assert!(app.library_keyframes(&item.asset_id).unwrap().is_empty());
    app.inner
        .lock()
        .unwrap()
        .db
        .execute_batch("DROP TRIGGER fail_cleanup")
        .unwrap();
    app.shutdown().await;
    drop(app);
    let reopened = core(root.path());
    assert_eq!(
        reopened
            .inner
            .lock()
            .unwrap()
            .db
            .query_row("SELECT count(*) FROM task_staging", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    reopened
        .start_keyframes(
            &item.asset_id,
            KeyframeRequest::Position {
                file_id: item.file_id,
                position_ms: 500,
            },
        )
        .unwrap();
    assert_eq!(terminal(&reopened).await.state, "completed");
    reopened.shutdown().await;
}
