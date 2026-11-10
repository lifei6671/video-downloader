use super::*;

#[tokio::test]
async fn late_transcript_preserves_new_text_after_upload_acknowledges_it() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    local_text(&f.core, &f.id, "old remote text");
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    f.core
        .backup_db(|db| {
            db.execute("DELETE FROM asset_transcripts WHERE asset_id=?", [&f.id])?;
            Ok(())
        })
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_stream = Some((started.clone(), release.clone()));
    let read = sync_read::transcript(&f.core, provider.as_ref(), &f.id);
    let edit = async {
        started.notified().await;
        local_text(&f.core, &f.id, "new acknowledged local text");
        engine
            .execute_upload(&claim(&engine, &f.core))
            .await
            .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(read, edit)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().unwrap().text, "new acknowledged local text");
    assert_eq!(
        f.core
            .backup_db(|db| repository::transcript(db, &f.id))
            .unwrap()
            .unwrap()
            .text,
        "new acknowledged local text"
    );
    f.core.shutdown().await;
}

#[tokio::test]
async fn unchanged_import_skips_writes_but_local_changes_invalidate_receipt() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let remote = provider.read_asset(&f.id).await.unwrap().unwrap();
    f.core
        .backup_db(|db| {
            let before = db.total_changes();
            sync_engine::import_asset(db, provider.scope(), &remote)?;
            assert_eq!(db.total_changes(), before);
            let patch = std::collections::BTreeMap::from([(
                "title".into(),
                serde_json::json!("local title"),
            )]);
            sync_engine::mark_local(db, &f.id, patch, false)?;
            let changed = db.total_changes();
            sync_engine::import_asset(db, provider.scope(), &remote)?;
            assert!(db.total_changes() > changed);
            assert_eq!(
                repository::asset(db, &f.id)?.unwrap().metadata["title"],
                "local title"
            );
            let stable = db.total_changes();
            sync_engine::import_asset(db, provider.scope(), &remote)?;
            assert_eq!(db.total_changes(), stable);
            Ok(())
        })
        .unwrap();
    f.core.shutdown().await;
}

#[tokio::test]
async fn thousands_of_assets_use_reverse_indexes_and_skip_unchanged_import_writes() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let template = provider.read_asset(&f.id).await.unwrap().unwrap();
    let mut remotes = Vec::new();
    for size in [5_000, 10_000] {
        f.core
            .backup_db(|db| {
                let tx = db.unchecked_transaction()?;
                while remotes.len() < size {
                    let mut remote = template.clone();
                    remote.snapshot.asset_id = uuid::Uuid::new_v4().to_string();
                    sync_engine::import_asset(&tx, provider.scope(), &remote)?;
                    remotes.push(remote);
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            f.core.register_backup_history(&f.space.space_id).unwrap(),
            0
        );
        let registration = started.elapsed();
        let started = std::time::Instant::now();
        f.core
            .backup_db(|db| {
                let before = db.total_changes();
                let tx = db.unchecked_transaction()?;
                for remote in &remotes {
                    sync_engine::import_asset(&tx, provider.scope(), remote)?;
                }
                tx.commit()?;
                assert_eq!(
                    db.total_changes(),
                    before,
                    "unchanged pull must not rewrite assets"
                );
                for (table, column, index) in [
                    (
                        "backup_space_assets",
                        "local_asset_id",
                        "backup_space_assets_local",
                    ),
                    ("backup_operations", "asset_id", "backup_operations_asset"),
                ] {
                    let plan: String = db.query_row(
                        &format!("EXPLAIN QUERY PLAN SELECT 1 FROM {table} WHERE {column}=?"),
                        [&f.id],
                        |r| r.get(3),
                    )?;
                    assert!(plan.contains(index), "{plan}");
                }
                Ok(())
            })
            .unwrap();
        eprintln!(
            "library scale={size}: history={registration:?}, unchanged import={:?}, writes=0",
            started.elapsed()
        );
    }
    f.core.shutdown().await;
}

// Delivers a few bytes after each delay to distinguish total duration from idle time.
pub(super) struct SlowReader {
    inner: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    delay: std::time::Duration,
    waiting: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}
impl SlowReader {
    pub(super) fn new(
        inner: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
        delay: std::time::Duration,
    ) -> Self {
        Self {
            inner,
            delay,
            waiting: None,
        }
    }
}
impl tokio::io::AsyncRead for SlowReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        output: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::{future::Future, task::Poll};
        if self.waiting.is_none() {
            self.waiting = Some(Box::pin(tokio::time::sleep(self.delay)));
        }
        if self
            .waiting
            .as_mut()
            .unwrap()
            .as_mut()
            .poll(cx)
            .is_pending()
        {
            return Poll::Pending;
        }
        let mut buffer = [0u8; 8];
        let capacity = output.remaining().min(buffer.len());
        let mut part = tokio::io::ReadBuf::new(&mut buffer[..capacity]);
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, &mut part) {
            Poll::Ready(Ok(())) => {
                output.put_slice(part.filled());
                self.waiting = None;
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[tokio::test(start_paused = true)]
async fn small_reads_allow_continuous_slow_s3_and_long_telegram_chunks() {
    let (f, inner, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let source = inner
        .read_asset(&f.id)
        .await
        .unwrap()
        .unwrap()
        .snapshot
        .media
        .unwrap();
    for (kind, seconds) in [(ProviderType::S3, 20), (ProviderType::Telegram, 40)] {
        let mut scope = inner.scope().clone();
        scope.provider_type = kind;
        let mut reference = source.clone();
        reference.scope = scope.clone();
        if kind == ProviderType::Telegram {
            reference.location = MediaLocation::TelegramMessage {
                account_id: "fixture".into(),
                channel_id: "fixture".into(),
                message_id: 1,
                document_id: 1,
            };
        }
        let provider = TelegramReadFixture {
            inner: inner.clone(),
            scope,
            source: source.clone(),
            delay: Some(std::time::Duration::from_secs(seconds)),
            read_failures: 0.into(),
            pull_failures: 0.into(),
        };
        let started = tokio::time::Instant::now();
        let bytes = sync_read::small(&f.core, &provider, &reference)
            .await
            .unwrap();
        assert_eq!(tag(&bytes), source.sha256);
        assert!(started.elapsed() > std::time::Duration::from_secs(30));
    }
    f.core.shutdown().await;
}

#[tokio::test]
async fn unchanged_space_reimports_after_another_space_changes_shared_projection() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let mut a = provider.read_asset(&f.id).await.unwrap().unwrap();
    a.snapshot
        .fields
        .insert("title".into(), serde_json::json!("space A"));
    let mut b = a.clone();
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.library_id = uuid::Uuid::new_v4().to_string();
    other.prefix = "other/".into();
    let mut scope_b = provider.scope().clone();
    scope_b.space_id = other.space_id.clone();
    scope_b.library_id = other.library_id.clone();
    b.snapshot.library_id = scope_b.library_id.clone();
    b.version.scope = scope_b.clone();
    b.snapshot
        .fields
        .insert("title".into(), serde_json::json!("space B"));
    for reference in b
        .snapshot
        .media
        .iter_mut()
        .chain(b.snapshot.cover.iter_mut())
        .chain(b.snapshot.transcript.iter_mut())
    {
        reference.scope = scope_b.clone();
    }
    f.core.backup_db(|db| {
        repository::save_space(db,&other)?;
        db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id) VALUES(?1,?2,?2)",params![other.space_id,f.id])?;
        for (scope,remote,title) in [(provider.scope(),&a,"space A"),(&scope_b,&b,"space B"),(provider.scope(),&a,"space A")] {
            sync_engine::import_asset(db,scope,remote)?;
            assert_eq!(repository::asset(db,&f.id)?.unwrap().metadata["title"],title);
        }
        Ok(())
    }).unwrap();
    f.core.shutdown().await;
}
