use super::*;

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
