use super::*;
use crate::{desktop_core::transcription::Source, library::LocalMetadata};
use std::{
    path::Path,
    sync::{mpsc, Mutex},
};

fn fixture(root: &Path, emit: impl Fn(TaskEvent) + Send + Sync + 'static) -> Arc<Core> {
    Core::open(
        &root.join("test.sqlite"),
        root.join("downloads"),
        "unused".into(),
        vec![],
        emit,
    )
    .unwrap()
}
fn seed(core: &Core, root: &Path) {
    let path = root.join("video.mp4");
    std::fs::write(&path, vec![3u8; 1024 * 1024]).unwrap();
    library::register_local(
        &core.inner.lock().unwrap().db,
        &Source::read(&path).unwrap(),
        &LocalMetadata {
            width: 160,
            height: 90,
            duration_ms: Some(1000),
        },
    )
    .unwrap();
}
async fn terminal(core: &Core) -> TransferSnapshot {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = core.library_transfer().unwrap();
            if state.state != "running" {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn transfer_worker_reports_success_and_failure_and_preserves_restore_state() {
    let root = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let core = fixture(root.path(), move |e| {
        if let TaskEvent::LibraryTransfer(s) = e {
            captured.lock().unwrap().push(s);
        }
    });
    seed(&core, root.path());
    let info = core.library_backup_info().unwrap();
    assert_eq!(
        (
            info.assets,
            info.files,
            info.available_files,
            info.media_bytes
        ),
        (1, 1, 1, 1024 * 1024)
    );
    assert!(!info.empty);
    let package = root.path().join("package");
    let first = core.start_library_backup(package.clone(), true).unwrap();
    let finished = terminal(&core).await;
    assert_eq!(finished.id, first.id);
    assert_eq!(finished.state, "completed");
    assert!(finished.sequence > first.sequence);
    assert_eq!(finished.result.as_ref().unwrap()["files"], 1);
    assert!(package.join("manifest.json").is_file());
    assert!(events.lock().unwrap().iter().any(|s| s.bytes > 0));
    let bad = core.start_library_backup(package.clone(), false).unwrap();
    assert!(bad.order > first.order);
    let failed = terminal(&core).await;
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.error.as_deref(), Some("LIBRARY_TARGET_EXISTS"));
    assert_eq!(
        core.cancel_library_transfer(&first.id)
            .unwrap_err()
            .to_string(),
        "LIBRARY_TRANSFER_NOT_FOUND"
    );
    let dest = tempfile::tempdir().unwrap();
    let restored = fixture(dest.path(), |_| {});
    let target = dest.path().join("restored");
    restored
        .start_library_restore(package, target.clone())
        .unwrap();
    assert_eq!(terminal(&restored).await.state, "completed");
    assert!(target.is_dir());
    assert!(!restored.library_backup_info().unwrap().empty);
    assert_eq!(
        restored
            .start_library_restore(root.path().join("package"), dest.path().join("again"))
            .unwrap_err()
            .to_string(),
        "LIBRARY_NOT_EMPTY"
    );
    core.shutdown().await;
    restored.shutdown().await;
}

#[tokio::test]
async fn cancellation_and_shutdown_join_actual_copy_worker_and_remove_staging() {
    for shutdown in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (notify, notified) = tokio::sync::oneshot::channel();
        let notify = Mutex::new(Some(notify));
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let core = fixture(root.path(), move |event| {
            if let TaskEvent::LibraryTransfer(s) = event {
                if s.state == "running" && s.bytes > 0 {
                    let pending = notify.lock().unwrap().take();
                    if let Some(notify) = pending {
                        let _ = notify.send(());
                        resume.lock().unwrap().recv().unwrap();
                    }
                }
            }
        });
        seed(&core, root.path());
        let target = root.path().join("canceled");
        let operation = core.start_library_backup(target.clone(), true).unwrap();
        tokio::time::timeout(Duration::from_secs(10), notified)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            core.start_library_backup(root.path().join("second"), false)
                .unwrap_err()
                .to_string(),
            "LIBRARY_TRANSFER_BUSY"
        );
        if shutdown {
            let running = core.clone();
            let ending = tokio::spawn(async move {
                running.shutdown().await;
            });
            while !core.inner.lock().unwrap().closing {
                tokio::task::yield_now().await;
            }
            release.send(()).unwrap();
            ending.await.unwrap();
        } else {
            core.cancel_library_transfer(&operation.id).unwrap();
            assert!(core.library_transfer().unwrap().canceling);
            release.send(()).unwrap();
            assert_eq!(terminal(&core).await.state, "canceled");
            core.shutdown().await;
        }
        assert_eq!(core.library_transfer().unwrap().state, "canceled");
        assert!(!target.exists());
        assert!(!std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("yingqu-library-")));
        assert_eq!(
            std::fs::metadata(root.path().join("video.mp4"))
                .unwrap()
                .len(),
            1024 * 1024
        );
    }
}
