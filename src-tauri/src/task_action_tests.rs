use super::*;
use crate::desktop_core::task_actions::Action;

fn terminal(app: &Core, status: &str) -> Task {
    let task: Task = serde_json::from_value(json!({
        "id":Uuid::new_v4().to_string(),"revision":1,"status":status,"video":null,
        "bytes":0,"total_bytes":null,"speed_bps":null,"error":null,"path":null,
        "created_at":1,"completed_at":null,"canceling":false,"download_requested":true
    }))
    .unwrap();
    publish(&mut app.inner.lock().unwrap(), task.clone(), &*app.emit).unwrap();
    task
}

async fn pending(app: &Core, id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !app.task_actions.lock().unwrap().contains_key(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn task_action_busy_removal_waits_coalesces_and_preserves_files() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let mut task = terminal(&app, "completed");
    let path = root.path().join("keep.mp4");
    std::fs::write(&path, b"keep source").unwrap();
    task.path = Some(path.clone());
    publish(&mut app.inner.lock().unwrap(), task.clone(), &*app.emit).unwrap();
    app.inner.lock().unwrap().downloads.insert(task.id.clone());
    let a = app.clone();
    let id = task.id.clone();
    let first = tokio::spawn(async move { a.task_action(&id, Action::Remove).await });
    pending(&app, &task.id).await;
    let a = app.clone();
    let id = task.id.clone();
    let second = tokio::spawn(async move { a.task_action(&id, Action::Remove).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!first.is_finished() && !second.is_finished());
    assert_eq!(app.list().len(), 1);
    app.inner.lock().unwrap().downloads.remove(&task.id);
    assert!(first.await.unwrap().unwrap().is_none());
    assert!(second.await.unwrap().unwrap().is_none());
    assert!(app.list().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"keep source");
    assert!(app.task_actions.lock().unwrap().is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn task_action_busy_cancel_and_shutdown_stop_without_mutation() {
    for (action, shutdown) in [
        (Action::Remove, false),
        (Action::Remove, true),
        (Action::Retry, false),
        (Action::Retry, true),
        (Action::Delete, false),
        (Action::Delete, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = terminal(&app, "failed");
        app.inner.lock().unwrap().downloads.insert(task.id.clone());
        let a = app.clone();
        let id = task.id.clone();
        let work = tokio::spawn(async move { a.task_action(&id, action).await });
        pending(&app, &task.id).await;
        if shutdown {
            app.shutdown().await;
        } else {
            app.cancel(&task.id).await.unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(1), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.err().unwrap().to_string(), "CANCELED");
        app.inner.lock().unwrap().downloads.remove(&task.id);
        assert_eq!(app.list()[0].revision, task.revision);
        assert_eq!(app.list()[0].status, Status::Failed);
        assert!(app.task_actions.lock().unwrap().is_empty());
        app.shutdown().await;
    }
}

#[tokio::test]
async fn task_action_busy_stale_delete_cannot_delete_a_new_revision() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let mut task = terminal(&app, "completed");
    let path = root.path().join("new-generation.mp4");
    std::fs::write(&path, b"new generation").unwrap();
    task.path = Some(path.clone());
    publish(&mut app.inner.lock().unwrap(), task.clone(), &*app.emit).unwrap();
    app.inner.lock().unwrap().downloads.insert(task.id.clone());
    let a = app.clone();
    let id = task.id.clone();
    let work = tokio::spawn(async move { a.task_action(&id, Action::Delete).await });
    pending(&app, &task.id).await;
    task.revision += 1;
    {
        let mut inner = app.inner.lock().unwrap();
        publish(&mut inner, task.clone(), &*app.emit).unwrap();
        inner.downloads.remove(&task.id);
    }
    assert_eq!(
        work.await.unwrap().err().unwrap().to_string(),
        "TASK_ACTION_STALE"
    );
    assert!(path.exists());
    assert_eq!(app.list()[0].revision, task.revision);
    app.shutdown().await;
}

#[tokio::test]
async fn task_action_busy_retry_waits_for_other_parser_and_starts_once() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = app.resolve("valid".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.cancel(&task.id).await.unwrap();
    let blocker = terminal(&app, "parsing");
    let a = app.clone();
    let id = task.id.clone();
    let first = tokio::spawn(async move { a.task_action(&id, Action::Retry).await });
    pending(&app, &task.id).await;
    let a = app.clone();
    let id = task.id.clone();
    let second = tokio::spawn(async move { a.task_action(&id, Action::Retry).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!first.is_finished() && !second.is_finished());
    let mut done = blocker;
    done.status = Status::Canceled;
    publish(&mut app.inner.lock().unwrap(), done, &*app.emit).unwrap();
    let first = first.await.unwrap().unwrap().unwrap();
    let second = second.await.unwrap().unwrap().unwrap();
    assert_eq!(
        (first.id.as_str(), first.revision),
        (second.id.as_str(), second.revision)
    );
    assert_eq!(app.list().len(), 2);
    app.shutdown().await;
}

#[tokio::test]
async fn task_action_busy_non_busy_errors_are_not_replayed() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = terminal(&app, "failed");
    assert_eq!(
        app.task_action(&task.id, Action::Retry)
            .await
            .err()
            .unwrap()
            .to_string(),
        "REPARSE_REQUIRED"
    );
    assert_eq!(app.list()[0].revision, task.revision);
    assert!(app.task_actions.lock().unwrap().is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn task_action_busy_dropped_owner_releases_followers_and_resources() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let task = terminal(&app, "completed");
    app.inner.lock().unwrap().downloads.insert(task.id.clone());
    let a = app.clone();
    let id = task.id.clone();
    let first = tokio::spawn(async move { a.task_action(&id, Action::Remove).await });
    pending(&app, &task.id).await;
    let a = app.clone();
    let id = task.id.clone();
    let follower = tokio::spawn(async move { a.task_action(&id, Action::Remove).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    first.abort();
    assert!(first.await.err().unwrap().is_cancelled());
    assert_eq!(
        follower.await.unwrap().err().unwrap().to_string(),
        "CANCELED"
    );
    assert!(app.task_actions.lock().unwrap().is_empty());
    assert_eq!(app.list().len(), 1);
    app.inner.lock().unwrap().downloads.remove(&task.id);
    assert!(app
        .task_action(&task.id, Action::Remove)
        .await
        .unwrap()
        .is_none());
    assert!(app.list().is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn task_action_busy_shutdown_closes_admission_before_resources_are_released() {
    let root = tempfile::tempdir().unwrap();
    let app = core(root.path());
    let first_task = terminal(&app, "completed");
    let later_task = terminal(&app, "failed");
    app.inner
        .lock()
        .unwrap()
        .downloads
        .insert(first_task.id.clone());
    let a = app.clone();
    let id = first_task.id.clone();
    let first = tokio::spawn(async move { a.task_action(&id, Action::Remove).await });
    pending(&app, &first_task.id).await;
    // Pause at shutdown's admission boundary, before network/worker cleanup.
    app.close_task_action_admission();
    assert!(app.inner.lock().unwrap().closing);
    for action in [Action::Remove, Action::Retry, Action::Delete] {
        assert_eq!(
            app.task_action(&later_task.id, action)
                .await
                .err()
                .unwrap()
                .to_string(),
            "CANCELED"
        );
    }
    assert_eq!(first.await.unwrap().err().unwrap().to_string(), "CANCELED");
    app.inner.lock().unwrap().downloads.remove(&first_task.id);
    assert_eq!(app.list().len(), 2);
    assert!(app.task_actions.lock().unwrap().is_empty());
    app.shutdown().await;
}
