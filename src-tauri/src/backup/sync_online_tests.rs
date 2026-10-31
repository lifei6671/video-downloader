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
struct Config {
    schema_version: u32,
    environment: String,
    endpoint: String,
    bucket: String,
    prefix: String,
    region: String,
    #[serde(default = "yes")]
    path_style: bool,
    #[serde(alias = "primary")]
    credentials: settings::Credentials,
}
fn yes() -> bool {
    true
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
    f.space.role = Role::Member;
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
        f.core.backup_db(|db|{db.execute("UPDATE backup_spaces SET endpoint=?1,bucket=?2,prefix=?3,role='member' WHERE space_id=?4",params![f.space.endpoint,f.space.bucket,f.space.prefix,f.space.space_id])?;db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?1",[serde_json::to_string(&caps)?])?;db.execute("INSERT INTO backup_current_space VALUES(1,?)",[&f.space.space_id])?;sync_engine::register_download(db,&f.id)})?;
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
