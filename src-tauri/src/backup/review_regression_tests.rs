use super::*;
use crate::backup::reconcile;

fn png(marker: u8) -> Vec<u8> {
    let mut body = b"\x89PNG\r\n\x1a\n".to_vec();
    body.extend([marker; 24]);
    body
}

#[tokio::test]
async fn cover_read_preserves_adopted_conflict_before_and_after_fetch() {
    for (finish_upload_before_read, cached_a) in [(false, false), (true, false), (true, true)] {
        let (f, provider, engine) = sync_fixture().await;
        let a = png(1);
        let b = png(2);
        f.core
            .backup_db(|db| {
                db.execute("INSERT INTO asset_covers VALUES(?1,?2)", params![f.id, a])?;
                Ok(())
            })
            .unwrap();
        engine
            .execute_upload(&claim(&engine, &f.core))
            .await
            .unwrap();
        let old = provider.read_asset(&f.id).await.unwrap().unwrap().snapshot;
        let conflict = ConflictVersion {
            field: "cover".into(),
            value: serde_json::to_value(old.cover.as_ref().unwrap()).unwrap(),
            device_id: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        };
        let key = format!("{}media/{}/cover-{}.png", f.space.prefix, f.id, tag(&b));
        // Use the provider wire encoder so lazy reads and conflict adoption see
        // the same valid published record as a second device would create.
        let mut wire: serde_json::Value = serde_json::from_slice(
            &f.store
                .get(&format!("{}records/{}.json", f.space.prefix, f.id))
                .await
                .unwrap()
                .unwrap()
                .body,
        )
        .unwrap();
        wire["cover"] =
            serde_json::json!({"key":key,"sha256":tag(&b),"bytes":b.len(),"container":"png"});
        let mut wire_conflict = serde_json::to_value(&conflict).unwrap();
        wire_conflict["value"] = serde_json::from_slice::<serde_json::Value>(
            &f.store
                .get(&format!("{}records/{}.json", f.space.prefix, f.id))
                .await
                .unwrap()
                .unwrap()
                .body,
        )
        .unwrap()["cover"]
            .clone();
        wire["conflicts"] = serde_json::json!([wire_conflict]);
        wire["record_revision"] = serde_json::json!(old.revision + 1);
        {
            let mut state = f.store.0.lock().unwrap();
            state.objects.insert(key, (b, None));
            state.objects.insert(
                format!("{}records/{}.json", f.space.prefix, f.id),
                (serde_json::to_vec(&wire).unwrap(), None),
            );
        }
        engine.refresh().await.unwrap();
        if !cached_a {
            f.core
                .backup_db(|db| {
                    db.execute("DELETE FROM asset_covers WHERE asset_id=?", [&f.id])?;
                    Ok(())
                })
                .unwrap();
        }
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        f.store.0.lock().unwrap().hold_stream = Some((started.clone(), release.clone()));
        let read = sync_read::cover(&f.core, provider.as_ref(), &f.id);
        let adopt = async {
            started.notified().await;
            let id = conflicts::local_data(&f.core, &f.space.space_id, &f.id)
                .unwrap()
                .conflicts[0]
                .id
                .clone();
            conflicts::resolve(&f.core, provider.as_ref(), &f.id, &id, true)
                .await
                .unwrap();
            assert_eq!(
                sync_read::cached_cover(&f.core, &f.space.space_id, &f.id).unwrap(),
                Some(a.clone())
            );
            assert_eq!(
                sync_read::cover(&f.core, provider.as_ref(), &f.id)
                    .await
                    .unwrap(),
                a
            );
            if finish_upload_before_read {
                engine
                    .execute_upload(&claim(&engine, &f.core))
                    .await
                    .unwrap();
            }
            release.notify_one();
        };
        let (read, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(read, adopt)
        })
        .await
        .unwrap();
        assert_eq!(read.unwrap(), a);
        if !finish_upload_before_read {
            engine
                .execute_upload(&claim(&engine, &f.core))
                .await
                .unwrap();
        }
        let final_record = provider.read_asset(&f.id).await.unwrap().unwrap().snapshot;
        assert_eq!(final_record.cover.unwrap().sha256, tag(&a));
        assert!(final_record.conflicts.is_empty());
    }
}

#[tokio::test]
async fn cover_read_honors_pending_edits_in_another_space() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.prefix = "other/".into();
    let body = png(3);
    f.core.backup_db(|db| {
        repository::save_space(db,&other)?;
        db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id,pending_json) VALUES(?1,?2,?2,'{\"cover\":1}')",params![other.space_id,f.id])?;
        db.execute("INSERT INTO asset_covers VALUES(?1,?2)",params![f.id,body])?;
        Ok(())
    }).unwrap();
    assert_eq!(
        sync_read::cover(&f.core, provider.as_ref(), &f.id)
            .await
            .unwrap(),
        body
    );
    assert_eq!(
        sync_read::cached_cover(&f.core, &f.space.space_id, &f.id).unwrap(),
        Some(body)
    );
}

#[tokio::test]
async fn deleting_one_membership_keeps_other_space_visible_and_restorable_after_restart() {
    for migrate in [false, true] {
        let (f, provider, engine) = sync_fixture().await;
        engine
            .execute_upload(&claim(&engine, &f.core))
            .await
            .unwrap();
        let mut other = f.space.clone();
        other.space_id = uuid::Uuid::new_v4().to_string();
        other.library_id = uuid::Uuid::new_v4().to_string();
        other.prefix = "other/".into();
        f.core.backup_db(|db| {
        repository::save_space(db,&other)?;
        db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?2 WHERE space_id=?1",params![other.space_id,serde_json::to_string(&capabilities())?])?;
        spaces::select_local(db,&other.space_id,true)
    }).unwrap();
        let other_provider = Arc::new(S3Provider::new(
            f.store.clone(),
            other.clone(),
            capabilities(),
            f.core.backup_db(repository::device_id).unwrap(),
        ));
        other_provider.open_library(true).await.unwrap();
        sync_engine::request_upload_with(&f.core, other_provider.clone(), &f.id).unwrap();
        let other_engine = SyncEngine::new(f.core.clone(), other_provider.clone());
        other_engine
            .execute_upload(&claim(&other_engine, &f.core))
            .await
            .unwrap();
        f.core
            .backup_db(|db| spaces::select_local(db, &f.space.space_id, true))
            .unwrap();
        sync_delete::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        let operation = deletion_operation(&f.core);
        delete_pass(&f.core, provider.as_ref(), &operation)
            .await
            .unwrap();
        assert!(sync_read::list(&f.core, &f.space.space_id)
            .unwrap()
            .is_empty());
        assert_eq!(sync_read::list(&f.core, &other.space_id).unwrap().len(), 1);
        assert_eq!(
            reconcile::list_for_display(&f.core, &other.space_id)
                .unwrap()
                .len(),
            1
        );
        let Fixture { root, core, id, .. } = f;
        drop(engine);
        drop(other_engine);
        drop(core);
        if migrate {
            let db = rusqlite::Connection::open(root.path().join("app.sqlite")).unwrap();
            db.execute("UPDATE backup_schema SET version=6", [])
                .unwrap();
            db.execute("DROP TABLE asset_deletion_hidden_history", [])
                .unwrap();
            db.execute("INSERT INTO asset_hidden_history VALUES(?)", [&id])
                .unwrap();
        }
        let reopened = Core::open(
            &root.path().join("app.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        reopened
            .backup_db(|db| spaces::select_local(db, &other.space_id, true))
            .unwrap();
        assert_eq!(
            reconcile::list_for_display(&reopened, &other.space_id)
                .unwrap()
                .len(),
            1
        );
        let restored = sync_read::ensure(&reopened, other_provider.as_ref(), &id)
            .await
            .unwrap();
        assert_eq!(std::fs::metadata(restored).unwrap().len(), 24);
        crate::backup::lifecycle::hide_history(&reopened, &id).unwrap();
        assert!(sync_read::list(&reopened, &other.space_id)
            .unwrap()
            .is_empty());
        assert!(reconcile::list_for_display(&reopened, &other.space_id)
            .unwrap()
            .is_empty());
    }
}
