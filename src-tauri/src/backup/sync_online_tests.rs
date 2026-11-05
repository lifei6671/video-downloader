//! Explicit opt-in; every object/session belongs to a newly journaled test run prefix.
use super::*;
use crate::backup::{
    provider::*,
    s3_provider::{probe, S3Provider},
    sync_engine::{self, SyncEngine},
};
#[tokio::test]
#[ignore = "authorized dedicated-test s3-r2-test.local.json required"]
async fn p6_real_r2_v2_two_devices() {
    scenario(false, false).await;
}
#[tokio::test]
#[ignore = "authorized dedicated-test s3-r2-test.local.json required; 100 MiB upload"]
async fn p6_real_r2_v2_multipart_restart() {
    scenario(false, true).await;
}
#[tokio::test]
#[ignore = "separately authorized dedicated-test s3-sync-test.local.json required"]
async fn p6_real_generic_s3_v2_two_devices() {
    scenario(true, false).await;
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    environment: String,
    endpoint: String,
    bucket: String,
    prefix: String,
    region: String,
    path_style: bool,
    credentials: settings::Credentials,
}

/// Uses a new run prefix and two newly created databases; never opens an application profile.
#[tokio::test]
#[ignore = "explicitly authorized current-format R2 config and synthetic media required"]
async fn real_r2_current_upload_readback_fresh_database() -> Result<()> {
    use crate::backup::{library::LibraryMetadata, sync_read};
    use serde_json::json;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("repository root")?;
    let work = root.join(".local/r2-current-validation");
    let raw = zeroize::Zeroizing::new(std::fs::read_to_string(work.join("config.private.json"))?);
    let config: Config = serde_json::from_str(&raw).context("CURRENT_R2_TEST_CONFIG_INVALID")?;
    anyhow::ensure!(config.schema_version == 1 && config.environment == "dedicated-test");
    crate::backup::s3_client::validate_target(&config.endpoint, &config.bucket, &config.prefix)?;
    let run = uuid::Uuid::new_v4().to_string();
    let directory = work.join(format!("run-{run}"));
    std::fs::create_dir(&directory)?;
    let video = directory.join("original.mp4");
    std::fs::copy(
        root.join(".local/telegram-transcript-first-validation/retained-original.mp4"),
        &video,
    )?;
    let cover = std::fs::read(root.join(".local/telegram-t2-validation/input/cover.png"))?;
    let (hash, signature) = repository::fingerprint(&video)?;
    let bytes = std::fs::metadata(&video)?.len();
    let sql_bytes = i64::try_from(bytes)?;
    anyhow::ensure!(
        bytes == 20_498_841
            && hash == "90a00390ad62fd7674c8691ae003d0ecc9b8bf3afe02ad03c27fcbca9868a972",
        "SYNTHETIC_VIDEO_CHANGED"
    );
    anyhow::ensure!(
        tag(&cover) == "4a55b5f936fd0481bd9dcb1b355e441ee181b5f1c2104fe1d310b9bb02adaec1",
        "SYNTHETIC_COVER_CHANGED"
    );
    let asset = uuid::Uuid::new_v4().to_string();
    let space = Space {
        space_id: uuid::Uuid::new_v4().to_string(),
        library_id: uuid::Uuid::new_v4().to_string(),
        endpoint: config.endpoint.clone(),
        bucket: config.bucket.clone(),
        prefix: format!("{}current-run-{run}/", config.prefix),
        credential_ref: None,
        enabled: true,
    };
    std::fs::write(
        directory.join("scope.private.json"),
        serde_json::to_vec_pretty(
            &json!({"prefix":space.prefix,"library_id":space.library_id,"asset_id":asset}),
        )?,
    )?;
    let make = |target: &Space, audit| -> Result<ObservedR2> {
        Ok(ObservedR2 {
            inner: crate::backup::s3_client::S3Store::configured(
                target,
                &config.credentials.access_key_id,
                &config.credentials.secret_access_key,
                &config.region,
                config.path_style,
            )?,
            audit,
        })
    };
    let control = make(&space, Default::default())?;
    let core = Core::open(
        &directory.join("source.sqlite"),
        directory.join("source-downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )?;
    let result = async {
        let caps = probe(&control, &space.prefix, &uuid::Uuid::new_v4().to_string()).await?;
        let source_device = core.backup_db(repository::device_id)?;
        let video_id = &asset.replace('-', "")[..11];
        let text = "R2 当前格式上传、原字节回读与全新数据库重建。Chinese English 123。";
        let metadata = json!({"title":"R2 首次建库恢复验证","author":"映取验证作者","author_profile":{"id":"synthetic-author","nickname":"映取验证作者","handle":"synthetic-author","kind":"user"},"platform":"youtube","video_id":video_id,"canonical_url":format!("https://www.youtube.com/watch?v={video_id}"),"duration_ms":2000,"width":640,"height":360,"completed_at":1});
        core.backup_db(|db| {
            let tx = db.unchecked_transaction()?;
            repository::save_space(&tx, &space)?;
            tx.execute("UPDATE backup_provider_config SET region=?1,path_style=?2,capabilities_json=?3 WHERE space_id=?4", params![config.region,config.path_style,serde_json::to_string(&caps)?,space.space_id])?;
            tx.execute("INSERT INTO backup_current_space VALUES(1,?)", [&space.space_id])?;
            tx.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'r2-synthetic.mp4',?3,?4,'active')",params![asset,metadata.to_string(),hash,sql_bytes])?;
            tx.execute("INSERT INTO asset_local_files VALUES(?1,?2,?3,?4,?5,unixepoch(),'present')",params![asset,video.to_string_lossy(),sql_bytes,hash,signature.to_string()])?;
            tx.execute("INSERT INTO asset_covers VALUES(?1,?2)",params![asset,cover])?;
            tx.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,?3,'synthetic-test','r2-current-test',1)",params![asset,hash,text])?;
            sync_engine::register_download(&tx,&asset)?;
            tx.commit()?;
            Ok(())
        })?;
        let source_audit = Arc::new(Mutex::new(R2Audit::default()));
        let provider = Arc::new(S3Provider::new(make(&space, source_audit.clone())?,space.clone(),caps.clone(),source_device.clone()));
        provider.open_library(true).await?;
        let engine = SyncEngine::new(core.clone(),provider.clone());
        core.verify_backup_asset(&asset).await?;
        engine.enqueue()?;
        let operation = scheduler::operations(&core)?.into_iter().find(|o|o.kind=="upload" && o.state=="pending").context("UPLOAD_NOT_QUEUED")?;
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=?",[&operation.operation_id])?;Ok(())})?;
        engine.execute_upload(&operation.operation_id).await?;
        anyhow::ensure!(scheduler::operations(&core)?.iter().any(|o|o.operation_id==operation.operation_id && o.state=="completed"),"UPLOAD_NOT_COMMITTED");
        let remote = provider.read_asset(&asset).await?.context("REMOTE_RECORD_MISSING")?;
        let prefix = format!("{}media/{asset}/",space.prefix);
        for (kind,reference) in [("video",remote.snapshot.media.as_ref()),("cover",remote.snapshot.cover.as_ref()),("transcript",remote.snapshot.transcript.as_ref())] {
            let reference = reference.context("REMOTE_ATTACHMENT_MISSING")?;
            let MediaLocation::S3Object {locator} = &reference.location else {bail!("R2_REFERENCE_EXPECTED")};
            anyhow::ensure!(locator==&format!("{prefix}{kind}-{}.{}",reference.sha256,reference.container),"NON_CANONICAL_MEDIA_KEY");
            let head=control.head(locator).await?.context("REMOTE_ATTACHMENT_HEAD_MISSING")?;
            anyhow::ensure!(head.bytes==reference.bytes,"REMOTE_ATTACHMENT_SIZE_CHANGED");
            if kind!="video" {
                let body=sync_read::small(&core,provider.as_ref(),reference).await?;
                if kind=="cover" {anyhow::ensure!(body==cover,"COVER_BYTES_CHANGED");}
                else {let value:super::super::model::AssetTranscript=serde_json::from_slice(&body)?;anyhow::ensure!(value.text==text && value.content_sha256.as_deref()==Some(&hash),"TRANSCRIPT_CHANGED");}
            }
        }
        let library:LibraryMetadata=serde_json::from_slice(&control.get(&format!("{}library.json",space.prefix)).await?.context("REMOTE_LIBRARY_MISSING")?.body)?;
        library.validate()?;
        let fresh_path=directory.join("fresh.sqlite");
        anyhow::ensure!(!fresh_path.exists(),"FRESH_DATABASE_ALREADY_EXISTS");
        let fresh=Core::open(&fresh_path,directory.join("fresh-downloads"),"unused".into(),vec![],|_|{})?;
        let rebuilt=async {
            anyhow::ensure!(fresh.backup_assets()?.is_empty() && fresh.list().is_empty(),"NEW_DATABASE_NOT_EMPTY");
            let fresh_device=fresh.backup_db(repository::device_id)?;
            anyhow::ensure!(fresh_device!=source_device,"DEVICE_ID_COPIED");
            let mut target=space.clone();target.space_id=uuid::Uuid::new_v4().to_string();target.library_id=library.library_id;
            fresh.backup_db(|db|{repository::save_space(db,&target)?;db.execute("UPDATE backup_provider_config SET region=?1,path_style=?2,capabilities_json=?3 WHERE space_id=?4",params![config.region,config.path_style,serde_json::to_string(&caps)?,target.space_id])?;db.execute("INSERT INTO backup_current_space VALUES(1,?)",[&target.space_id])?;Ok(())})?;
            let audit=Arc::new(Mutex::new(R2Audit::default()));
            let reader=Arc::new(S3Provider::new(make(&target,audit.clone())?,target.clone(),caps,fresh_device));
            reader.open_library(false).await?;
            SyncEngine::new(fresh.clone(),reader.clone()).pull().await?;
            let assets=fresh.backup_assets()?;
            anyhow::ensure!(assets.len()==1,"REBUILT_ASSET_COUNT_CHANGED");
            let local=&assets[0];
            anyhow::ensure!(local.path.is_none(),"METADATA_REPLAY_DOWNLOADED_VIDEO");
            anyhow::ensure!(local.metadata["title"]==metadata["title"] && local.metadata["author"]==metadata["author"] && local.metadata["author_profile"]==metadata["author_profile"],"REBUILT_AUTHOR_TITLE_CHANGED");
            anyhow::ensure!(sync_read::cover(&fresh,reader.as_ref(),&local.asset_id).await?==cover,"REBUILT_COVER_CHANGED");
            anyhow::ensure!(sync_read::transcript(&fresh,reader.as_ref(),&local.asset_id).await?.context("REBUILT_TRANSCRIPT_MISSING")?.text==text,"REBUILT_TRANSCRIPT_CHANGED");
            anyhow::ensure!(audit.lock().unwrap().video_streams==0,"SMALL_READS_FETCHED_VIDEO");
            fresh.backup_db(|db| {
                let columns=|table:&str| -> Result<usize> {Ok(db.prepare(&format!("PRAGMA table_info({table})"))?.query_map([],|_|Ok(()))?.collect::<rusqlite::Result<Vec<_>>>()?.len())};
                anyhow::ensure!(columns("backup_spaces")?==7 && columns("asset_remote_objects")?==4,"SCHEMA_NOT_CURRENT");
                anyhow::ensure!(db.query_row("SELECT version FROM backup_schema WHERE id=1",[],|r|r.get::<_,i64>(0))?==6);
                Ok(())
            })?;
            let restored=sync_read::ensure(&fresh,reader.as_ref(),&local.asset_id).await?;
            anyhow::ensure!(std::fs::metadata(&restored)?.len()==bytes && repository::fingerprint(&restored)?.0==hash,"RESTORED_VIDEO_CHANGED");
            let streams=audit.lock().unwrap().video_streams;
            anyhow::ensure!(streams==1 && sync_read::ensure(&fresh,reader.as_ref(),&local.asset_id).await?==restored && audit.lock().unwrap().video_streams==streams,"LOCAL_RESTORE_NOT_REUSED");
            anyhow::ensure!(audit.lock().unwrap().calls.get("WRITE").copied().unwrap_or(0)==0,"REBUILD_WROTE_REMOTE_OBJECTS");
            let (video_writes, uploaded_parts) = {let audit=source_audit.lock().unwrap();(audit.video_writes,audit.uploaded_parts.clone())};
            Ok::<_,anyhow::Error>(json!({"result":"PASSED","video_bytes":bytes,"video_sha256":hash,"cover_bytes":cover.len(),"cover_sha256":tag(&cover),"three_current_shallow_keys":true,"title_author_profile_transcript_restored":true,"fresh_database_created":true,"fresh_device_identity":true,"schema_version":6,"space_columns":7,"remote_state_columns":4,"metadata_reads_video_streams":0,"on_demand_video_streams":streams,"local_video_reused":true,"rebuild_remote_writes":0,"source_video_upload_calls":video_writes,"uploaded_parts":uploaded_parts}))
        }.await;
        fresh.shutdown().await;
        rebuilt
    }.await;
    core.shutdown().await;
    let cleanup = current_run_cleanup(&control, &space.prefix).await;
    let mut report = match &result {
        Ok(value) => value.clone(),
        Err(error) => json!({"result":"FAILED","error":error.to_string()}),
    };
    report["cleanup"] = match &cleanup {
        Ok(count) => {
            json!({"result":"PASSED","deleted_objects":count,"remaining_objects":0,"remaining_multipart_sessions":0})
        }
        Err(error) => json!({"result":"FAILED","error":error.to_string()}),
    };
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(work.join("latest-run.txt"), format!("run-{run}"))?;
    result?;
    cleanup?;
    println!("Current R2 upload, exact-byte reads, fresh database reconstruction and lazy video restore passed; owned run prefix cleaned");
    Ok(())
}

#[tokio::test]
async fn current_r2_cleanup_keeps_other_prefix_objects_and_sessions() -> Result<()> {
    let store = Memory::default();
    for key in ["tests/current-run-owned/video.mp4", "tests/other/video.mp4"] {
        store
            .write(Write::Put {
                key: key.into(),
                body: vec![1],
                condition: Condition::Absent,
            })
            .await?;
        store
            .write(Write::CreateMultipart {
                key: key.into(),
                sha256: tag(&[1]),
            })
            .await?;
    }
    anyhow::ensure!(current_run_cleanup(&store, "tests/current-run-owned/").await? == 1);
    anyhow::ensure!(store.get("tests/other/video.mp4").await?.is_some());
    anyhow::ensure!(store.multipart_sessions("tests/other/").await?.len() == 1);
    Ok(())
}

async fn current_run_cleanup(store: &dyn ObjectStore, prefix: &str) -> Result<usize> {
    for session in store.multipart_sessions(prefix).await? {
        anyhow::ensure!(session.key.starts_with(prefix), "CLEANUP_SCOPE_MISMATCH");
        store
            .write(Write::AbortMultipart {
                key: session.key,
                upload_id: session.upload_id,
            })
            .await?;
    }
    let mut token = None;
    let mut keys = Vec::new();
    loop {
        let page = store.list(prefix, token).await?;
        for object in page.objects {
            anyhow::ensure!(object.key.starts_with(prefix), "CLEANUP_SCOPE_MISMATCH");
            keys.push(object.key);
        }
        token = page.next;
        if token.is_none() {
            break;
        }
    }
    let count = keys.len();
    for key in keys {
        store.write(Write::Delete { key }).await?;
    }
    anyhow::ensure!(
        store.list(prefix, None).await?.objects.is_empty()
            && store.multipart_sessions(prefix).await?.is_empty(),
        "CLEANUP_INCOMPLETE"
    );
    Ok(count)
}
fn claim(engine: &SyncEngine, core: &Core) -> String {
    engine.enqueue().unwrap();
    let op = scheduler::operations(core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "upload" && o.state == "pending")
        .expect("ONLINE_UPLOAD_MISSING");
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })
    .unwrap();
    op.operation_id
}
fn edit(core: &Core, id: &str, field: &str, value: &str) -> Result<()> {
    core.backup_db(|db| {
        sync_engine::mark_local(
            db,
            id,
            [(field.into(), serde_json::json!(value))]
                .into_iter()
                .collect(),
            false,
        )
    })
}
async fn scenario(generic: bool, restart: bool) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let raw = zeroize::Zeroizing::new(
        std::fs::read_to_string(root.join(if generic {
            "s3-sync-test.local.json"
        } else {
            "s3-r2-test.local.json"
        }))
        .expect("AUTHORIZED_TEST_CONFIG_UNAVAILABLE"),
    );
    let config: Config = serde_json::from_str(&raw).expect("INVALID_TEST_CONFIG");
    assert_eq!(config.schema_version, 1);
    assert_eq!(config.environment, "dedicated-test");
    assert!(
        !config.prefix.trim_matches('/').is_empty(),
        "TEST_PREFIX_REQUIRED"
    );
    if generic {
        crate::backup::s3_client::validate_s3_target(
            &config.endpoint,
            &config.bucket,
            &config.prefix,
        )
        .unwrap();
    } else {
        crate::backup::s3_client::validate_target(&config.endpoint, &config.bucket, &config.prefix)
            .unwrap();
    }
    let mut f = Fixture::new(if restart { 100 * 1024 * 1024 } else { 24 }).await;
    f.space.endpoint = config.endpoint.clone();
    f.space.bucket = config.bucket.clone();
    f.space.prefix = format!("{}v2-run-{}/", config.prefix, uuid::Uuid::new_v4());
    let make = |s: &Space, audit: Arc<Mutex<R2Audit>>| ObservedR2 {
        inner: crate::backup::s3_client::S3Store::configured(
            s,
            &config.credentials.access_key_id,
            &config.credentials.secret_access_key,
            &config.region,
            config.path_style,
        )
        .unwrap(),
        audit,
    };
    let cleanup = make(&f.space, Default::default());
    std::fs::create_dir_all(root.join(".local")).unwrap();
    {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(".local/sync-v2-test-runs.jsonl"))
            .unwrap();
        writeln!(log,"{}",serde_json::json!({"provider":"s3","profile":if generic{"s3"}else{"r2"},"prefix":f.space.prefix,"library_id":f.space.library_id,"asset_id":f.id})).unwrap();
    }
    let audit = Arc::new(Mutex::new(R2Audit {
        lose_first_part_response: restart,
        ..Default::default()
    }));
    let result=async{
        let caps=probe(&cleanup,&f.space.prefix,&uuid::Uuid::new_v4().to_string()).await?;
        f.core.backup_db(|db|{db.execute("UPDATE backup_spaces SET endpoint=?1,bucket=?2,prefix=?3 WHERE space_id=?4",params![f.space.endpoint,f.space.bucket,f.space.prefix,f.space.space_id])?;db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?1",[serde_json::to_string(&caps)?])?;db.execute("INSERT INTO backup_current_space VALUES(1,?)",[&f.space.space_id])?;sync_engine::register_download(db,&f.id)})?;
        let provider=Arc::new(S3Provider::new(make(&f.space,audit.clone()),f.space.clone(),caps.clone(),f.core.backup_db(repository::device_id)?));provider.open_library(true).await?;
        let mut engine=SyncEngine::new(f.core.clone(),provider.clone());let op=claim(&engine,&f.core);
        if restart{
            anyhow::ensure!(engine.execute_upload(&op).await.is_err(),"EXPECTED_LOST_PART_RESPONSE");drop(engine);
            let placeholder=Core::open(&f.root.path().join("placeholder.sqlite"),f.root.path().join("placeholder"),"unused".into(),vec![],|_|{})?;
            let retired=std::mem::replace(&mut f.core,placeholder);anyhow::ensure!(Arc::strong_count(&retired)==1);drop(retired);
            f.core=Core::open(&f.root.path().join("app.sqlite"),f.root.path().join("downloads"),"unused".into(),vec![],|_|{})?;engine=SyncEngine::new(f.core.clone(),provider.clone());
            let resumed=claim(&engine,&f.core);anyhow::ensure!(resumed==op);engine.execute_upload(&resumed).await?;
            anyhow::ensure!(audit.lock().unwrap().uploaded_parts==vec![1,1,2,3,4,5,6,7],"WHOLE_UPLOAD_DID_NOT_RESTART");
        }else{engine.execute_upload(&op).await?;}
        let br=tempfile::tempdir()?;let bc=Core::open(&br.path().join("b.sqlite"),br.path().join("b-downloads"),"unused".into(),vec![],|_|{})?;
        let mut bs=f.space.clone();bs.space_id=uuid::Uuid::new_v4().to_string();
        bc.backup_db(|db|{repository::save_space(db,&bs)?;db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?1",[serde_json::to_string(&caps)?])?;db.execute("INSERT INTO backup_current_space VALUES(1,?)",[&bs.space_id])?;Ok(())})?;
        anyhow::ensure!(bc.backup_db(repository::device_id)?!=f.core.backup_db(repository::device_id)?);
        let b_audit=Arc::new(Mutex::new(R2Audit::default()));
        let b=Arc::new(S3Provider::new(make(&bs,b_audit.clone()),bs.clone(),caps,bc.backup_db(repository::device_id)?));let be=SyncEngine::new(bc.clone(),b.clone());be.pull().await?;
        let bid=bc.backup_assets()?.first().context("B_ASSET_MISSING")?.asset_id.clone();anyhow::ensure!(bc.backup_assets()?[0].path.is_none());
        edit(&f.core,&f.id,"title","A title")?;edit(&bc,&bid,"author","B author")?;
        let ao=claim(&engine,&f.core);let bo=claim(&be,&bc);let(ar,br)=tokio::join!(engine.execute_upload(&ao),be.execute_upload(&bo));ar?;br?;
        engine.pull().await?;be.pull().await?;let active=provider.read_asset(&f.id).await?.context("ACTIVE_MISSING")?;
        anyhow::ensure!(active.snapshot.fields["title"]=="A title"&&active.snapshot.fields["author"]=="B author");
        let a_video_writes=audit.lock().unwrap().video_writes;
        bc.backup_db(|db|{db.execute("INSERT OR REPLACE INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'B cloud text','test','test',1)",params![bid,active.snapshot.media.as_ref().map(|m|&m.sha256)])?;sync_engine::mark_local(db,&bid,Default::default(),true)})?;
        be.execute_upload(&claim(&be,&bc)).await?;engine.pull().await?;
        anyhow::ensure!(crate::backup::sync_read::transcript(&f.core,provider.as_ref(),&f.id).await?.context("CLOUD_TEXT_MISSING")?.text=="B cloud text");
        anyhow::ensure!(audit.lock().unwrap().video_writes==a_video_writes&&b_audit.lock().unwrap().video_writes==0,"METADATA_REUPLOADED_VIDEO");
        anyhow::ensure!(audit.lock().unwrap().video_streams==0&&b_audit.lock().unwrap().video_streams==0,"BROWSING_FETCHED_VIDEO");
        edit(&f.core,&f.id,"title","A conflict")?;edit(&bc,&bid,"title","B conflict")?;
        let ao=claim(&engine,&f.core);let bo=claim(&be,&bc);let(ar,br)=tokio::join!(engine.execute_upload(&ao),be.execute_upload(&bo));ar?;br?;be.pull().await?;
        anyhow::ensure!(!b.read_asset(&f.id).await?.context("CONFLICT_MISSING")?.snapshot.conflicts.is_empty());
        let conflicts=crate::backup::conflicts::local_data(&bc,&bs.space_id,&bid)?;
        let id=&conflicts.conflicts.first().context("CONFLICT_NOT_VISIBLE")?.id;
        crate::backup::conflicts::detail(&bc,b.as_ref(),&bid,id).await?;
        crate::backup::conflicts::resolve(&bc,b.as_ref(),&bid,id,true).await?;
        be.execute_upload(&claim(&be,&bc)).await?;engine.pull().await?;
        anyhow::ensure!(crate::backup::conflicts::local_data(&f.core,&f.space.space_id,&f.id)?.conflicts.is_empty(),"CONFLICT_NOT_CONVERGED");
        let restored=crate::backup::sync_read::ensure(&bc,b.as_ref(),&bid).await?;anyhow::ensure!(restored.is_file());
        bc.backup_db(|db|{db.execute("INSERT OR REPLACE INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'B unsynced text','test','test',1)",params![bid,active.snapshot.media.as_ref().map(|m|&m.sha256)])?;sync_engine::mark_local(db,&bid,Default::default(),true)})?;
        let delete=crate::backup::sync_delete::intent(&f.core,&f.space.space_id,&f.id)?;
        f.core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=?",[&delete])?;Ok(())})?;
        crate::backup::sync_delete::execute(&f.core,provider.as_ref(),&delete).await?;be.pull().await?;
        anyhow::ensure!(restored.is_file(),"OTHER_DEVICE_FILE_DELETED");anyhow::ensure!(crate::backup::sync_read::ensure(&bc,b.as_ref(),&bid).await?==restored);
        anyhow::ensure!(crate::backup::conflicts::export_data(&bc,&bs.space_id,&bid)?["transcript"]["text"]=="B unsynced text");
        anyhow::ensure!(b.read_asset(&f.id).await?.context("TOMBSTONE_MISSING")?.snapshot.deleted);
        be.enqueue()?;anyhow::ensure!(scheduler::operations(&bc)?.iter().all(|o|o.kind!="upload"||o.state=="completed"));
        eprintln!("v2 online passed: two databases/device IDs/directories; concurrent merge/conflict; lazy restore; tombstone/local-file/text protection; multipart_restart={restart}");Ok::<_,anyhow::Error>(())
    }.await;
    for session in cleanup
        .multipart_sessions(&f.space.prefix)
        .await
        .expect("ONLINE_CLEANUP_LIST_UPLOADS")
    {
        assert!(session.key.starts_with(&f.space.prefix));
        cleanup
            .write(Write::AbortMultipart {
                key: session.key,
                upload_id: session.upload_id,
            })
            .await
            .expect("ONLINE_CLEANUP_ABORT");
    }
    let mut token = None;
    let mut keys = Vec::new();
    loop {
        let page = cleanup
            .list(&f.space.prefix, token)
            .await
            .expect("ONLINE_CLEANUP_LIST");
        for o in page.objects {
            assert!(o.key.starts_with(&f.space.prefix));
            keys.push(o.key);
        }
        token = page.next;
        if token.is_none() {
            break;
        }
    }
    for key in keys {
        cleanup
            .write(Write::Delete { key })
            .await
            .expect("ONLINE_CLEANUP_DELETE");
    }
    assert!(cleanup
        .list(&f.space.prefix, None)
        .await
        .unwrap()
        .objects
        .is_empty());
    assert!(cleanup
        .multipart_sessions(&f.space.prefix)
        .await
        .unwrap()
        .is_empty());
    eprintln!("v2 online cleanup: isolated run prefix and multipart sessions empty");
    result.unwrap();
}
