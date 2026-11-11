use super::*;

#[tokio::test]
async fn redownload_album_upgrades_in_place_and_preserves_extra_files_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let static_fixture = root.path().join("album-static");
    std::fs::write(&static_fixture, "").unwrap();
    let app = core(root.path());
    app.set_filename_template("album".into()).unwrap();
    let original = app.resolve("album".into()).unwrap();
    wait(&app, &original.id, Status::Ready).await;
    app.download(&original.id, false).unwrap();
    let original = wait(&app, &original.id, Status::Completed).await;
    let path = original.path.as_ref().unwrap();
    assert!(!path.join("001.mp4").exists());
    std::fs::remove_file(static_fixture).unwrap();
    app.set_filename_template("renamed".into()).unwrap();
    let preview = app.resolve("album".into()).unwrap();
    wait(&app, &preview.id, Status::Ready).await;
    assert_eq!(
        app.submit_download(&preview.id, true, Some("stale")).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    assert_eq!(app.submit_download(&preview.id, true, Some(&original.id)).unwrap(), original.id);
    let updated = wait(&app, &original.id, Status::Completed).await;
    assert_eq!(updated.insertion_id, original.insertion_id);
    assert_eq!(updated.path.as_ref(), Some(path));
    assert!(path.join("001.mp4").is_file());
    assert!(updated.files[0].video.is_some());
    assert_eq!(app.list().len(), 1);

    let extra = path.join("keep.txt");
    std::fs::write(&extra, "user file").unwrap();
    let preview = app.resolve("album".into()).unwrap();
    wait(&app, &preview.id, Status::Ready).await;
    app.submit_download(&preview.id, true, Some(&original.id)).unwrap();
    let preserved = wait(&app, &original.id, Status::Completed).await;
    assert_eq!(preserved.error.as_deref(), Some("REDOWNLOAD_EXTRA_FILES"));
    assert!(path.join("001.mp4").is_file());
    assert_eq!(std::fs::read_to_string(&extra).unwrap(), "user file");
    assert_eq!(app.list().len(), 1);
    app.shutdown().await;
    assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[tokio::test]
async fn redownload_cancel_restores_existing_task_and_file() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    app.set_filename_template("{link_id}".into()).unwrap();
    let task = enqueue(&app, "8922").await;
    release(root.path(), "8922");
    let original = wait(&app, &task.id, Status::Completed).await;
    std::fs::remove_file(root.path().join("release-8922")).unwrap();
    std::fs::remove_file(root.path().join("started-8922")).unwrap();
    let preview = app.resolve("hold:8922".into()).unwrap();
    wait(&app, &preview.id, Status::Ready).await;
    app.submit_download(&preview.id, true, Some(&task.id)).unwrap();
    started(root.path(), "8922").await;
    let path = original.path.as_ref().unwrap();
    let old = std::fs::read(path).unwrap();
    assert_eq!(app.list().len(), 1);
    app.cancel(&task.id).await.unwrap();
    let restored = wait(&app, &task.id, Status::Completed).await;
    assert_eq!(restored.completed_at, original.completed_at);
    assert_eq!(restored.path.as_ref(), Some(path));
    assert_eq!(std::fs::read(path).unwrap(), old);
    app.shutdown().await;
}

#[tokio::test]
async fn redownload_recovery_rolls_back_an_uncommitted_replacement() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = app.resolve("valid".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.download(&task.id, false).unwrap();
    let original = wait(&app, &task.id, Status::Completed).await;
    app.shutdown().await;
    drop(app);
    let target = original.path.as_ref().unwrap();
    let previous = std::fs::read(target).unwrap();
    let stage = target.parent().unwrap().join(format!("yingqu-{}-interrupted", task.id));
    let source = stage.join("media/video.mp4");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    let replacement = b"\0\0\0\x18ftypmp42new-version";
    std::fs::write(&source, replacement).unwrap();
    let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
    redownload::remember(&db, &original, Some("preview")).unwrap();
    let mut attempt = original.clone();
    attempt.status = Status::Downloading;
    attempt.path = None;
    persist_new_attempt(&db, &attempt).unwrap();
    db.execute("INSERT INTO task_staging VALUES(?1,?2)", params![task.id, stage.to_string_lossy()]).unwrap();
    db.execute("INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,?4)",
        params![task.id, source.to_string_lossy(), target.to_string_lossy(), i64::try_from(replacement.len()).unwrap()]).unwrap();
    redownload::replace(&source, target, &original).unwrap();
    assert_eq!(std::fs::read(target).unwrap(), replacement);
    drop(db);

    let app = core(root.path());
    assert_eq!(std::fs::read(target).unwrap(), previous);
    assert!(!stage.exists());
    assert_eq!(app.list()[0].error.as_deref(), Some("INTERRUPTED"));
    app.start_backup_service();
    let recovered = wait(&app, &task.id, Status::Completed).await;
    assert_eq!(recovered.path.as_ref(), Some(target));
    assert_eq!(app.list().len(), 1);
    app.shutdown().await;
}

#[tokio::test]
async fn redownload_rollback_failure_is_retained_even_when_canceling() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = app.resolve("valid".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.download(&task.id, false).unwrap();
    let original = wait(&app, &task.id, Status::Completed).await;
    {
        let mut inner = app.inner.lock().unwrap();
        let mut attempt = original.clone();
        attempt.status = Status::Downloading;
        attempt.canceling = true;
        redownload::remember(&inner.db, &original, Some("preview")).unwrap();
        persist_new_attempt(&inner.db, &attempt).unwrap();
        *inner.tasks.iter_mut().find(|task| task.id == original.id).unwrap() = attempt;
    }
    app.finish(&original.id, Err(anyhow::anyhow!("REDOWNLOAD_RESTORE_FAILED")), true);
    let failed = wait(&app, &original.id, Status::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("REDOWNLOAD_RESTORE_FAILED"));
    assert!(redownload::original(&app.inner.lock().unwrap().db, &original.id).unwrap().is_some());
    assert!(app.remove_history(&original.id).is_err());
    app.shutdown().await;
    drop(app);
    let app = core(root.path());
    app.start_backup_service();
    let restored = &app.list()[0];
    assert_eq!(restored.status, Status::Completed);
    assert_eq!(restored.path, original.path);
    assert!(redownload::original(&app.inner.lock().unwrap().db, &original.id).unwrap().is_none());
    app.shutdown().await;
}

#[tokio::test]
async fn redownload_unknown_legacy_format_requires_a_separate_file() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = app.resolve("valid".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.download(&task.id, false).unwrap();
    let mut original = wait(&app, &task.id, Status::Completed).await;
    original.video.as_mut().unwrap()["selected_format_id"] = Value::Null;
    publish(&mut app.inner.lock().unwrap(), original.clone(), &*app.emit).unwrap();
    let preview = app.resolve("valid".into()).unwrap();
    wait(&app, &preview.id, Status::Ready).await;
    app.choose_format(&preview.id, &"2".repeat(24)).unwrap();
    assert_eq!(
        app.submit_download(&preview.id, true, Some(&original.id)).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    app.download(&preview.id, true).unwrap();
    let copied = wait(&app, &preview.id, Status::Completed).await;
    assert_ne!(copied.path, original.path);
    assert!(original.path.unwrap().is_file());
    assert_eq!(app.list().len(), 2);
    app.shutdown().await;
}

#[test]
fn duplicate_in_progress_only_matches_submitted_same_identity_and_variant() {
    let candidate: Task = serde_json::from_value(json!({
        "id":"candidate","revision":1,"status":"ready",
        "video":{"platform":"douyin","video_id":"42","kind":"video","selected_format_id":"a"},
        "bytes":0,"total_bytes":null,"speed_bps":null,"error":null,"path":null,
        "created_at":1,"completed_at":null,"canceling":false
    }))
    .unwrap();
    for status in [Status::Queued, Status::Downloading, Status::Parsing] {
        let mut existing = candidate.clone();
        existing.id = "existing".into();
        existing.status = status;
        existing.download_requested = true;
        assert!(previously_downloaded(&[existing.clone()], &candidate));
        existing.video.as_mut().unwrap()["selected_format_id"] = "b".into();
        assert!(!previously_downloaded(&[existing.clone()], &candidate));
        existing.video.as_mut().unwrap()["selected_format_id"] = "a".into();
        existing.video.as_mut().unwrap()["video_id"] = "43".into();
        assert!(!previously_downloaded(&[existing], &candidate));
    }
    for status in [
        Status::Ready,
        Status::Failed,
        Status::Canceled,
        Status::Parsing,
    ] {
        let mut existing = candidate.clone();
        existing.id = "existing".into();
        existing.status = status;
        existing.download_requested = false;
        assert!(!previously_downloaded(&[existing], &candidate));
    }
    assert!(!previously_downloaded(
        std::slice::from_ref(&candidate),
        &candidate
    ));
}

#[tokio::test]
async fn duplicate_in_progress_queued_and_downloading_require_explicit_confirmation() {
    for queued in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_max_downloads(1).unwrap();
        if queued {
            let blocker = enqueue(&app, "8910").await;
            started(root.path(), "8910").await;
            assert_eq!(
                app.list()
                    .iter()
                    .find(|t| t.id == blocker.id)
                    .unwrap()
                    .status,
                Status::Downloading
            );
        }
        let first = enqueue(&app, "8920").await;
        let second = app.resolve("hold:8920".into()).unwrap();
        wait(&app, &second.id, Status::Ready).await;
        assert_eq!(
            app.download(&second.id, false).unwrap_err().to_string(),
            "ALREADY_DOWNLOADED"
        );
        let rejected = app.list().into_iter().find(|t| t.id == second.id).unwrap();
        assert_eq!(rejected.status, Status::Ready);
        assert!(!rejected.download_requested);
        app.download(&second.id, true).unwrap();
        assert_eq!(
            app.list()
                .iter()
                .filter(|t| {
                    matches!(t.status, Status::Queued | Status::Downloading)
                        && t.video.as_ref().is_some_and(|v| v["video_id"] == "8920")
                })
                .count(),
            2
        );
        if queued {
            release(root.path(), "8910");
        }
        release(root.path(), "8920");
        let first = wait(&app, &first.id, Status::Completed).await;
        let second = wait(&app, &second.id, Status::Completed).await;
        assert_ne!(first.path, second.path);
        assert_eq!(
            std::fs::read(first.path.unwrap()).unwrap(),
            std::fs::read(second.path.unwrap()).unwrap()
        );
        app.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_in_progress_concurrent_submissions_admit_only_one_unconfirmed_task() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let first = app.resolve("hold:8930".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    let second = app.resolve("hold:8930".into()).unwrap();
    wait(&app, &second.id, Status::Ready).await;
    let a = app.clone();
    let first_id = first.id.clone();
    let b = app.clone();
    let second_id = second.id.clone();
    let (a, b) = tokio::join!(
        tokio::spawn(async move { a.download(&first_id, false) }),
        tokio::spawn(async move { b.download(&second_id, false) })
    );
    let results = [a.unwrap(), b.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .into_iter()
            .find_map(Result::err)
            .unwrap()
            .to_string(),
        "ALREADY_DOWNLOADED"
    );
    assert_eq!(
        app.list().iter().filter(|t| t.download_requested).count(),
        1
    );
    release(root.path(), "8930");
    app.shutdown().await;
}

#[tokio::test]
async fn duplicate_in_progress_retry_stops_ready_instead_of_automatically_queuing() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let first = enqueue(&app, "8940").await;
    let second = app.resolve("hold:8940".into()).unwrap();
    wait(&app, &second.id, Status::Ready).await;
    app.cancel(&second.id).await.unwrap();
    app.retry(&second.id).unwrap();
    let second = wait(&app, &second.id, Status::Ready).await;
    assert_eq!(
        app.download(&second.id, false).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    assert_eq!(
        app.list()
            .iter()
            .filter(|t| matches!(t.status, Status::Queued | Status::Downloading))
            .count(),
        1
    );
    release(root.path(), "8940");
    wait(&app, &first.id, Status::Completed).await;
    app.shutdown().await;
}
