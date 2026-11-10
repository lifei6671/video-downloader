use super::*;
use crate::desktop_core::{Status, Task};
use rusqlite::OptionalExtension;

fn historical(root: &std::path::Path, kind: &str) -> Task {
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.mp4"));
    std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();
    serde_json::from_value(serde_json::json!({"id":id,"revision":0,"status":"completed","video":{"kind":kind,"platform":"youtube","video_id":id,"title":"history"},"bytes":24,"total_bytes":24,"speed_bps":null,"error":null,"path":path,"created_at":1,"completed_at":2,"canceling":false})).unwrap()
}

#[tokio::test]
async fn history_registration_queues_only_unbacked_videos_and_keeps_existing_receipts_and_choices()
{
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let new = historical(f.root.path(), "video");
    let album = historical(f.root.path(), "image_album");
    let mut failed = historical(f.root.path(), "video");
    failed.status = Status::Failed;
    let missing = historical(f.root.path(), "video");
    std::fs::remove_file(missing.path.as_ref().unwrap()).unwrap();
    let mut detached = historical(f.root.path(), "video");
    let mut foreign = historical(f.root.path(), "video");
    let mut canceled = historical(f.root.path(), "video");
    let mut deleted = historical(f.root.path(), "video");
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.library_id = uuid::Uuid::new_v4().to_string();
    other.prefix = "other/".into();
    other.enabled = false;
    f.core.backup_db(|db| {
        repository::save_space(db,&other)?;
        db.execute("INSERT OR REPLACE INTO backup_sync_preference VALUES(1,0)",[])?;
        for task in [&mut detached,&mut foreign,&mut canceled,&mut deleted] {repository::associate(db,task)?;}
        db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id) VALUES(?1,?2,?2)",params![other.space_id,foreign.asset_id])?;
        db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'upload','canceled','queued','{}')",params![uuid::Uuid::new_v4().to_string(),f.space.space_id,canceled.asset_id])?;
        db.execute("UPDATE media_assets SET asset_state='deleted' WHERE asset_id=?",[&deleted.asset_id])?;
        // Simulate the historical missing task link without removing the durable receipt.
        foreign.asset_id=None;
        for task in [&new,&album,&failed,&missing,&detached,&foreign,&canceled,&deleted] {db.execute("INSERT INTO download_tasks(id,snapshot) VALUES(?1,?2)",params![task.id,serde_json::to_string(task)?])?;}
        for (task,stale) in [(&new,false),(&detached,true)] {
            let mut source=crate::desktop_core::transcription::Source::read(task.path.as_deref().unwrap())?;
            if stale {source.size+=1;}
            let snapshot=serde_json::json!({"task_id":task.id,"operation_id":"old-asr","sequence":1,"state":"completed","phase":null,"error_code":null,"updated_at":2,"has_result":true,"source_changed":false,"result":{"text":"保留历史文案","source":source,"model_revision":"fixture-model","pipeline_revision":"fixture-pipeline","completed_at":2}});
            db.execute("INSERT INTO video_transcripts VALUES(?1,?2)",params![task.id,snapshot.to_string()])?;
        }
        Ok(())
    }).unwrap();
    drop(engine);
    let Fixture {
        root, core, space, ..
    } = f;
    core.shutdown().await;
    drop(core);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        core.register_backup_history(&space.space_id).unwrap(),
        0,
        "disabled backup never backfills history"
    );
    spaces::set_enabled(&core, true).await.unwrap();
    assert_eq!(
        core.register_backup_history(&other.space_id).unwrap(),
        0,
        "viewing another space never registers history"
    );
    assert_eq!(
        core.register_backup_history(&space.space_id).unwrap(),
        3,
        "two pending videos and one repaired old link"
    );
    let tasks = core.list();
    for task in [&new, &detached, &foreign] {
        assert!(tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .asset_id
            .is_some());
        assert_eq!(
            tasks.iter().find(|t| t.id == task.id).unwrap().revision,
            task.revision + 1
        );
    }
    for task in [&album, &failed, &missing] {
        assert!(tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .asset_id
            .is_none());
    }
    let members: Vec<String> = core
        .backup_db(|db| {
            let mut q =
                db.prepare("SELECT local_asset_id FROM backup_space_assets WHERE space_id=?")?;
            let rows = q
                .query_map([&space.space_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .unwrap();
    assert!(members.contains(&new.id));
    assert!(members.contains(detached.asset_id.as_ref().unwrap()));
    assert!(!members.contains(&foreign.id));
    assert!(!members.contains(canceled.asset_id.as_ref().unwrap()));
    assert!(!members.contains(deleted.asset_id.as_ref().unwrap()));
    let engine = SyncEngine::new(core.clone(), provider);
    engine.enqueue().unwrap();
    engine.enqueue().unwrap();
    let pending = crate::backup::scheduler::operations(&core)
        .unwrap()
        .into_iter()
        .filter(|o| o.state == "pending")
        .collect::<Vec<_>>();
    assert_eq!(
        pending.len(),
        2,
        "one durable queue entry per missing video"
    );
    assert!(
        pending
            .iter()
            .all(|o| o.asset_id.as_deref() == Some(new.id.as_str())
                || o.asset_id == detached.asset_id)
    );
    assert_eq!(
        core.asset_transcript(&new.id).unwrap().unwrap().text,
        "保留历史文案"
    );
    assert!(
        core.asset_transcript(detached.asset_id.as_deref().unwrap())
            .unwrap()
            .is_none(),
        "a stale source transcript never joins the backup"
    );
    let operation = &pending
        .iter()
        .find(|o| o.asset_id.as_deref() == Some(new.id.as_str()))
        .unwrap()
        .operation_id;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [operation],
        )?;
        Ok(())
    })
    .unwrap();
    engine.execute_upload(operation).await.unwrap();
    assert!(
        engine
            .provider()
            .read_asset(&new.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .transcript
            .is_some(),
        "the validated historical transcript is uploaded with its original video"
    );
    assert_eq!(core.register_backup_history(&space.space_id).unwrap(), 0);
    let expected = core
        .list()
        .into_iter()
        .find(|t| t.id == new.id)
        .unwrap()
        .asset_id;
    drop(engine);
    core.shutdown().await;
    drop(core);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        core.list()
            .into_iter()
            .find(|t| t.id == new.id)
            .unwrap()
            .asset_id,
        expected
    );
    assert_eq!(
        core.register_backup_history(&space.space_id).unwrap(),
        0,
        "restart preserves links without duplicate uploads"
    );
    core.shutdown().await;
}

#[tokio::test]
#[ignore = "explicit opt-in for the protected database copy; no sessions or cloud requests"]
async fn authorized_history_copy_registration() -> Result<()> {
    anyhow::ensure!(
        std::env::var("YINGQU_HISTORY_COPY_VALIDATION").as_deref() == Ok("registration-only"),
        "EXPLICIT_COPY_OPT_IN_REQUIRED"
    );
    let work = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("repository")?
        .join(".local/backup-history-recovery");
    let path = work.join("validation-history.sqlite");
    anyhow::ensure!(path.is_file(), "AUTHORIZED_COPY_MISSING");
    let core = Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let result=async {
        let selected=spaces::current_id(&core)?.context("BACKUP_SPACE_NOT_FOUND")?;
        let before=core.list();
        let mut expected_texts=vec![];
        for task in before.iter().filter(|t|t.asset_id.is_none()&&t.status==Status::Completed&&t.video.as_ref().is_some_and(|v|v["kind"]=="video")) {
            let raw:Option<String>=core.backup_db(|db|Ok(db.query_row("SELECT snapshot FROM video_transcripts WHERE task_id=?",[&task.id],|r|r.get(0)).optional()?))?;
            if let Some(raw)=raw {
                let snapshot:crate::desktop_core::transcription::Snapshot=serde_json::from_str(&raw)?;
                if let Some(result)=snapshot.result {
                    if result.source==crate::desktop_core::transcription::Source::read(task.path.as_deref().context("FILE_MISSING")?)? {expected_texts.push((task.id.clone(),result.text));}
                }
            }
        }
        let originals=core.backup_assets()?;
        let old_operations=crate::backup::scheduler::operations(&core)?.into_iter().map(serde_json::to_value).collect::<serde_json::Result<Vec<_>>>()?;
        let old_libraries:i64=core.backup_db(|db|Ok(db.query_row("SELECT COUNT(*) FROM library_assets",[],|r|r.get(0))?))?;
        let old_links:i64=core.backup_db(|db|Ok(db.query_row("SELECT COUNT(*) FROM asset_task_links",[],|r|r.get(0))?))?;
        let registered=core.register_backup_history(&selected)?;
        let after=core.list();
        let display=crate::backup::reconcile::list_for_display(&core,&selected)?;
        let mut newly_linked=0;
        for task in before {
            let updated=after.iter().find(|t|t.id==task.id).context("TASK_MISSING")?;
            let mut old=serde_json::to_value(&task)?;let mut new=serde_json::to_value(updated)?;
            if task.asset_id!=updated.asset_id {
                anyhow::ensure!(task.status==Status::Completed&&task.asset_id.is_none()&&task.video.as_ref().is_some_and(|v|v["kind"]=="video"),"UNEXPECTED_TASK_REWRITE");
                anyhow::ensure!(updated.revision==task.revision+1,"TASK_REVISION_NOT_ADVANCED");
                let entry=display.iter().find(|a|Some(&a.entry.asset.asset_id)==updated.asset_id.as_ref()).context("BACKUP_ASSET_NOT_FOUND")?;
                anyhow::ensure!(entry.in_current_space&&entry.entry.remote_state=="pending","HISTORY_NOT_PENDING");newly_linked+=1;
            }
            old.as_object_mut().unwrap().remove("asset_id");new.as_object_mut().unwrap().remove("asset_id");old.as_object_mut().unwrap().remove("revision");new.as_object_mut().unwrap().remove("revision");anyhow::ensure!(old==new,"TASK_CONTENT_CHANGED");
        }
        let assets=core.backup_assets()?;
        for asset in originals {
            let now=assets.iter().find(|a|a.asset_id==asset.asset_id).context("BACKUP_ASSET_NOT_FOUND")?;
            anyhow::ensure!(serde_json::to_value(&asset)?==serde_json::to_value(now)?,"EXISTING_ASSET_CHANGED");
        }
        let operations=crate::backup::scheduler::operations(&core)?.into_iter().map(serde_json::to_value).collect::<serde_json::Result<Vec<_>>>()?;
        anyhow::ensure!(operations==old_operations,"EXISTING_OPERATIONS_CHANGED");
        anyhow::ensure!(registered==newly_linked&&registered==8,"UNEXPECTED_HISTORY_COUNT");
        for (id,text) in &expected_texts {let task=after.iter().find(|t|t.id==*id).context("TASK_MISSING")?;let transcript=core.asset_transcript(task.asset_id.as_deref().context("BACKUP_ASSET_NOT_FOUND")?)?.context("TRANSCRIPT_MISSING")?;anyhow::ensure!(transcript.text==*text,"TRANSCRIPT_CHANGED");}
        anyhow::ensure!(core.register_backup_history(&selected)?==0,"DUPLICATE_HISTORY_REGISTRATION");
        core.backup_db(|db|{let libraries:i64=db.query_row("SELECT COUNT(*) FROM library_assets",[],|r|r.get(0))?;let links:i64=db.query_row("SELECT COUNT(*) FROM asset_task_links",[],|r|r.get(0))?;anyhow::ensure!(libraries==old_libraries&&links==old_links,"COLLECTION_CHANGED");Ok(())})?;
        let summary=serde_json::json!({"registered_history_videos":registered,"preserved_valid_transcripts":expected_texts.len(),"existing_assets_unchanged":true,"existing_operations_unchanged":true,"collection_unchanged":true,"idempotent":true,"cloud_requests":0});
        std::fs::write(work.join("copy-validation.redacted.json"),serde_json::to_vec_pretty(&summary)?)?;println!("{summary}");Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}

#[tokio::test(start_paused = true)]
async fn missing_history_file_is_rechecked_without_task_snapshot_changes() {
    let (f, _, _) = sync_fixture().await;
    let task = historical(f.root.path(), "video");
    let path = task.path.clone().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    f.core
        .backup_db(|db| {
            db.execute(
                "INSERT INTO download_tasks(id,snapshot) VALUES(?1,?2)",
                params![task.id, serde_json::to_string(&task)?],
            )?;
            Ok(())
        })
        .unwrap();
    f.core.shutdown().await;
    drop(f.core);
    let core = Core::open(
        &f.root.path().join("app.sqlite"),
        f.root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(core.register_backup_history(&f.space.space_id).unwrap(), 0);
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(core.register_backup_history(&f.space.space_id).unwrap(), 0);
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    assert_eq!(core.register_backup_history(&f.space.space_id).unwrap(), 1);
    assert_eq!(core.register_backup_history(&f.space.space_id).unwrap(), 0);
    core.shutdown().await;
}
