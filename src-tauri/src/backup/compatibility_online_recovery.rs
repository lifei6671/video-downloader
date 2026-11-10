//! Explicit, journal-preserving recovery of the three individually audited R2 jobs.
//! This entry point is ignored by default and never starts the background service.
use super::{
    provider::*,
    repository,
    settings::{Credentials, SystemVault, Vault},
    store::*,
    sync_engine::{self, SyncEngine},
};
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;

#[tokio::test]
#[ignore = "explicit existing-cloud audit authorization; local planning only, no network"]
async fn authorized_plan_telegram_evidence() -> Result<()> {
    use super::telegram_replay::{Cursor, Event};
    use super::telegram_transfer_journal::{CoreJournal, Journal, SendState};
    let (path, work) = super::compatibility_online_audit::paths()?;
    let core = crate::desktop_core::Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let result=async {
        let space=core.backup_db(repository::spaces)?.into_iter().find(|s|super::spaces::config(&core,&s.space_id).is_ok_and(|c|c.provider_type==ProviderType::Telegram)).context("BACKUP_SPACE_NOT_FOUND")?;
        let binding=super::telegram_spaces::binding(&core,&space.space_id)?.context("BACKUP_INVALID_SPACE")?.0;
        let target=super::telegram_transfer::Target {scope:Scope{space_id:space.space_id.clone(),library_id:space.library_id.clone(),provider_type:ProviderType::Telegram,protocol:2},account_id:binding.account_id,channel_id:binding.channel_id};
        let (rows,cursor)=core.backup_db(|db| {
            let mut q=db.prepare("SELECT document_json,revision,index_body FROM telegram_mapped_documents WHERE space_id=?")?;
            let rows=q.query_map([&space.space_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<Vec<u8>>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let cursor:String=db.query_row("SELECT cursor_json FROM telegram_observed_prefix WHERE space_id=?",[&space.space_id],|r|r.get(0))?;
            Ok((rows,serde_json::from_str::<Cursor>(&cursor)?))
        })?;
        let mut documents=vec![];let mut events=std::collections::BTreeMap::new();let mut head=None;
        for (doc,revision,body) in rows {
            let doc:super::telegram_transfer::RemoteDocument=serde_json::from_str(&doc)?;documents.push(doc.clone());
            let Some(revision)=revision else {continue;};let body=body.context("TELEGRAM_INDEX_INVALID")?;
            let value:Value=serde_json::from_slice(&body)?;
            let mut found=vec![];
            if value.get("change").is_some() {found.push((revision,body));}
            else {
                if head.as_ref().is_none_or(|(id,_,_):&(i32,Value,String)|*id<doc.message_id) {head=Some((doc.message_id,value.clone(),format!("{:x}",Sha256::digest(&body))));}
                for archived in value["archived_events"].as_array().into_iter().flatten() {found.push((archived["revision"].as_i64().context("TELEGRAM_INDEX_INVALID")?,serde_json::from_value::<Vec<u8>>(archived["body"].clone())?));}
                if !value["direct_delete"].is_null() {found.push(((i64::from(doc.message_id)<<32)|1,serde_json::from_value::<Vec<u8>>(value["direct_delete"].clone())?));}
            }
            for (revision,body) in found {if let Some(old)=events.insert(revision,body.clone()) {anyhow::ensure!(old==body,"TELEGRAM_INDEX_CHANGED");}}
        }
        let mut missing=vec![];let mut absent_documents=vec![];
        for (&revision,expected) in &cursor.evidence {
            if let Some(body)=events.get(&revision) {anyhow::ensure!(&format!("{:x}",Sha256::digest(body))==expected,"TELEGRAM_INDEX_CHANGED");continue;}
            let identities=core.backup_db(|db|{let mut q=db.prepare("SELECT operation_id,kind FROM telegram_media_sends WHERE json_extract(entry_json,'$.envelope.sha256')=?")?;let rows=q.query_map([expected],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)})?;
            anyhow::ensure!(identities.len()==1,"TELEGRAM_SEND_EVIDENCE_INVALID");
            let (operation,kind)=&identities[0];let entry=CoreJournal(&core).get(operation,kind)?.context("TELEGRAM_SEND_EVIDENCE_INVALID")?;
            anyhow::ensure!(entry.target==target&&entry.state==SendState::Confirmed&&kind=="event","TELEGRAM_SEND_EVIDENCE_INVALID");
            let document=entry.document.context("TELEGRAM_SEND_EVIDENCE_INVALID")?;
            anyhow::ensure!(i64::from(document.message_id)==revision,"TELEGRAM_SEND_EVIDENCE_INVALID");document.check(&entry.envelope)?;
            let (state,payload):(String,String)=core.backup_db(|db|Ok(db.query_row("SELECT state,payload_json FROM backup_operations WHERE operation_id=?",[operation],|r|Ok((r.get(0)?,r.get(1)?)))?))?;
            anyhow::ensure!(state=="completed","TELEGRAM_SEND_EVIDENCE_INVALID");
            let payload:Value=serde_json::from_str(&payload)?;let change:AssetChange=serde_json::from_value(payload["change"].clone())?;
            let body=Event::encode(&target,&change)?;
            anyhow::ensure!(format!("{:x}",Sha256::digest(&body))==*expected&&body.len() as u64==entry.envelope.bytes,"TELEGRAM_SEND_EVIDENCE_INVALID");
            missing.push(json!({"revision":revision,"body":body}));absent_documents.push(document);
            events.insert(revision,serde_json::from_value::<Vec<u8>>(missing.last().unwrap()["body"].clone())?);
        }
        anyhow::ensure!(missing.len()==2,"RECOVERY_EXPECTED_TWO_MISSING_EVENTS");
        let high=events.keys().next_back().copied().context("TELEGRAM_INDEX_INVALID")?;
        let projection=super::telegram_replay::replay_decoded(&target,high,events.into_iter().collect(),Some(&cursor),|body|Event::decode_with_documents(&target,body,&documents))?;
        let (head_id,mut library,head_sha)=head.context("TELEGRAM_INDEX_INVALID")?;
        library["previous_segment"]=json!({"message_id":head_id,"sha256":head_sha});library["archived_events"]=missing.into();library.as_object_mut().context("TELEGRAM_INDEX_INVALID")?.remove("direct_delete");
        let body=serde_json::to_vec(&library)?;let digest=format!("{:x}",Sha256::digest(&body));
        let plan_path=work.join("telegram-evidence-plan.private.json");
        let operation=if plan_path.exists() {let old:Value=serde_json::from_slice(&std::fs::read(&plan_path)?)?;anyhow::ensure!(old["body_sha256"]==digest,"RECOVERY_PLAN_CHANGED");old["operation_id"].as_str().context("RECOVERY_PLAN_INVALID")?.to_owned()} else {uuid::Uuid::new_v4().to_string()};
        let plan=json!({"target":target,"operation_id":operation,"body":body,"body_sha256":digest,"absent_documents":absent_documents,"assets":projection.assets,"observed_evidence":cursor.evidence.len()});
        std::fs::write(plan_path,serde_json::to_vec_pretty(&plan)?)?;
        println!("reconstructed 2 confirmed original event bodies with exact SHA-256; retained all {} observed byte digests; projected {} assets",cursor.evidence.len(),projection.assets.len());
        Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}

fn upgrade_cached_snapshot(raw: &mut Value, fresh: &AssetSnapshot) -> Result<usize> {
    anyhow::ensure!(
        raw["asset_id"] == fresh.asset_id && raw["library_id"] == fresh.library_id,
        "BACKUP_SCOPE_MISMATCH"
    );
    let new = serde_json::to_value(fresh)?;
    let mut count = 0;
    for kind in ["media", "cover", "transcript"] {
        if raw[kind]["location"]["provider_type"] != "telegram_message"
            || raw[kind]["location"].get("document_id").is_some()
        {
            continue;
        }
        for field in ["bytes", "sha256", "container"] {
            anyhow::ensure!(
                raw[kind][field] == new[kind][field],
                "TELEGRAM_MEDIA_CHANGED"
            );
        }
        for field in ["provider_type", "account_id", "channel_id", "message_id"] {
            anyhow::ensure!(
                raw[kind]["location"][field] == new[kind]["location"][field],
                "TELEGRAM_MEDIA_CHANGED"
            );
        }
        let id = new[kind]["location"]["document_id"]
            .as_i64()
            .filter(|v| *v != 0)
            .context("TELEGRAM_MEDIA_CHANGED")?;
        raw[kind]["location"]["document_id"] = id.into();
        count += 1;
    }
    Ok(count)
}

#[tokio::test]
#[ignore = "explicit authorization to append one index sealing two confirmed original events"]
async fn authorized_seal_telegram_evidence() -> Result<()> {
    use super::telegram_transfer::{ReceiptCheck, Transport};
    use super::telegram_transfer_journal::{CoreJournal, Journal, SendState};
    anyhow::ensure!(
        std::env::var("YINGQU_AUTHORIZED_BACKUP_RECOVERY").as_deref()
            == Ok("seal-two-confirmed-telegram-events"),
        "EXPLICIT_RECOVERY_OPT_IN_REQUIRED"
    );
    let (path, work) = super::compatibility_online_audit::paths()?;
    let plan: Value = serde_json::from_slice(&std::fs::read(
        work.join("telegram-evidence-plan.private.json"),
    )?)?;
    let target: super::telegram_transfer::Target = serde_json::from_value(plan["target"].clone())?;
    let operation = plan["operation_id"]
        .as_str()
        .context("RECOVERY_PLAN_INVALID")?
        .to_owned();
    let body: Vec<u8> = serde_json::from_value(plan["body"].clone())?;
    anyhow::ensure!(
        plan["body_sha256"] == format!("{:x}", Sha256::digest(&body)),
        "RECOVERY_PLAN_CHANGED"
    );
    let core = crate::desktop_core::Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let result=async {
        anyhow::ensure!(!super::spaces::enabled(&core)?,"RECOVERY_REQUIRES_PAUSED_SYNC");
        let cfg=super::spaces::config(&core,&target.scope.space_id)?;
        let provider=super::telegram_provider::TelegramProvider::new(&core,target.clone(),cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?);
        let expected:Value=serde_json::from_slice(&body)?;
        let absent:Vec<super::telegram_transfer::RemoteDocument>=serde_json::from_value(plan["absent_documents"].clone())?;
        let request_target=target.clone();let request_operation=operation.clone();
        provider.request("compatibility_recovery",None,move |core,transport,io|Box::pin(async move {
            let snapshot=Box::pin(transport.mapped_snapshot(core,&request_target,io)).await?;
            super::telegram_provider::segments::Catalog::read(&request_target,&snapshot,io)?;
            let head=snapshot.indexes.iter().filter(|index|serde_json::from_str::<super::telegram_transfer::Envelope>(&index.document.caption).is_ok_and(|e|e.kind=="library")).max_by_key(|index|index.document.message_id).context("BACKUP_LIBRARY_NOT_FOUND")?;
            let envelope:super::telegram_transfer::Envelope=serde_json::from_str(&head.document.caption)?;
            let already_sealed=envelope.operation_id==request_operation&&head.verified_body()?==body;
            if !already_sealed {
                anyhow::ensure!(expected["previous_segment"]["message_id"]==head.document.message_id&&expected["previous_segment"]["sha256"]==envelope.sha256,"RECOVERY_ARCHIVE_HEAD_CHANGED");
                let cursor:super::telegram_replay::Cursor=core.backup_db(|db|{let raw:String=db.query_row("SELECT cursor_json FROM telegram_observed_prefix WHERE space_id=?",[&request_target.scope.space_id],|r|r.get(0))?;Ok(serde_json::from_str(&raw)?)})?;
                let archived=expected["archived_events"].as_array().context("RECOVERY_PLAN_INVALID")?;
                anyhow::ensure!(archived.len()==2&&absent.len()==2,"RECOVERY_PLAN_INVALID");
                for event in archived {
                    let revision=event["revision"].as_i64().context("RECOVERY_PLAN_INVALID")?;
                    let bytes:Vec<u8>=serde_json::from_value(event["body"].clone())?;
                    let hash=format!("{:x}",Sha256::digest(&bytes));
                    anyhow::ensure!(cursor.evidence.get(&revision)==Some(&hash),"TELEGRAM_INDEX_CHANGED");
                    let decoded=super::telegram_replay::Event::decode(&request_target,&bytes)?;
                    let entry=CoreJournal(core).get(&decoded.change.operation_id,"event")?.context("TELEGRAM_SEND_EVIDENCE_INVALID")?;
                    anyhow::ensure!(entry.target==request_target&&entry.state==SendState::Confirmed&&entry.envelope.sha256==hash,"TELEGRAM_SEND_EVIDENCE_INVALID");
                    let original=entry.document.context("TELEGRAM_SEND_EVIDENCE_INVALID")?;
                    anyhow::ensure!(absent.contains(&original)&&i64::from(original.message_id)==revision,"TELEGRAM_SEND_EVIDENCE_INVALID");
                    match io.run(transport.inspect_document(original.message_id,original.document_id)).await {
                        Err(e) if e.to_string()=="TELEGRAM_MEDIA_MISSING"=>{},
                        Err(e)=>return Err(e),
                        Ok(_)=>bail!("RECOVERY_ORIGINAL_EVENT_STILL_PRESENT"),
                    }
                }
                let mut records=vec![];let mut high=snapshot.high;
                for index in &snapshot.indexes {
                    let envelope:super::telegram_transfer::Envelope=serde_json::from_str(&index.document.caption)?;
                    if envelope.kind=="event" {records.push((index.revision,index.verified_body()?.to_vec()));}
                    else if envelope.kind=="library" {
                        let library:super::telegram_provider::Library=serde_json::from_slice(index.verified_body()?)?;
                        library.append_archived_events_resolved(&request_target,index.document.message_id,&mut high,&mut records,&snapshot.media)?;
                    }
                }
                for event in archived {records.push((event["revision"].as_i64().context("RECOVERY_PLAN_INVALID")?,serde_json::from_value::<Vec<u8>>(event["body"].clone())?));}
                let mut unique=std::collections::BTreeMap::new();
                for (revision,bytes) in records {high=high.max(revision);if let Some(old)=unique.insert(revision,bytes.clone()) {anyhow::ensure!(old==bytes,"TELEGRAM_INDEX_CHANGED");}}
                super::telegram_replay::replay_decoded(&request_target,high,unique.into_iter().collect(),Some(&cursor),|bytes|super::telegram_replay::Event::decode_with_documents(&request_target,bytes,&snapshot.media))?;
            }
            Box::pin(super::telegram_provider::TelegramProvider::send_index(core,transport,io,&request_target,"library",&request_target.scope.library_id,&request_operation,&core.telegram_storage_device_id()?,body,super::delete_timing::IndexTiming::default(),ReceiptCheck::ReadBack)).await
        })).await?;
        let batch=provider.pull_changes(None).await?;
        let assets=batch.assets.iter().map(|a|(a.snapshot.asset_id.clone(),a.snapshot.clone())).collect::<std::collections::BTreeMap<_,_>>();
        let upgraded=core.backup_db(|db| {
            let tx=db.unchecked_transaction()?;let mut count=0;
            for (table,column,key) in [("asset_remote_records","record_json","asset_id"),("backup_space_assets","baseline_json","local_asset_id")] {
                let rows={let mut q=tx.prepare(&format!("SELECT {key},{column} FROM {table} WHERE space_id=?1 AND {column} IS NOT NULL"))?;let rows=q.query_map([&target.scope.space_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;rows};
                for (local,raw) in rows {
                    let mut raw:Value=serde_json::from_str(&raw)?;
                    let fresh=assets.get(raw["asset_id"].as_str().context("BACKUP_INVALID_RECORD")?).context("TELEGRAM_INDEX_CHANGED")?;
                    if !fresh.deleted&&raw.get("media").is_some_and(|m|!m.is_null()) {for field in ["sha256","bytes","container"] {anyhow::ensure!(raw["media"][field]==serde_json::to_value(fresh.media.as_ref())?[field],"TELEGRAM_MEDIA_CHANGED");}}
                    let changed=upgrade_cached_snapshot(&mut raw,fresh)?;count+=changed;
                    if changed>0 {tx.execute(&format!("UPDATE {table} SET {column}=?3 WHERE space_id=?1 AND {key}=?2"),params![target.scope.space_id,local,serde_json::to_string(&raw)?])?;}
                }
            }
            for asset in &batch.assets {sync_engine::import_asset(&tx,&target.scope,asset)?;}
            tx.execute("INSERT INTO backup_sync_checkpoints(space_id,checkpoint_json,last_error) VALUES(?1,?2,NULL) ON CONFLICT(space_id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json,last_error=NULL",params![target.scope.space_id,serde_json::to_string(&batch.checkpoint)?])?;
            tx.commit()?;Ok(count)
        })?;
        let summary=json!({"sealed_original_events":2,"new_archive_indexes":1,"video_uploads":0,"remote_deletes":0,"original_observed_evidence_retained":true,"upgraded_local_references":upgraded,"active_assets":assets.values().filter(|a|!a.deleted).count(),"deleted_assets":assets.values().filter(|a|a.deleted).count()});
        std::fs::write(work.join("telegram-recovery.redacted.json"),serde_json::to_vec_pretty(&summary)?)?;println!("{summary}");
        Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "explicit authorization to restore the original default after verified recovery"]
async fn authorized_finalize_recovery_and_sync() -> Result<()> {
    anyhow::ensure!(
        std::env::var("YINGQU_AUTHORIZED_BACKUP_RECOVERY").as_deref()
            == Ok("restore-verified-default-sync"),
        "EXPLICIT_RECOVERY_OPT_IN_REQUIRED"
    );
    let (path, work) = super::compatibility_online_audit::paths()?;
    let completed: Vec<Value> = serde_json::from_slice(&std::fs::read(
        work.join("r2-recovery-completed.private.json"),
    )?)?;
    let sealed: Value = serde_json::from_slice(&std::fs::read(
        work.join("telegram-recovery.redacted.json"),
    )?)?;
    anyhow::ensure!(
        completed.len() == 3
            && completed.iter().all(|v| v["sha256_verified"] == true)
            && sealed["original_observed_evidence_retained"] == true,
        "RECOVERY_NOT_VERIFIED"
    );
    let baseline = rusqlite::Connection::open_with_flags(
        work.join("before-recovery.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let selected: String = baseline.query_row(
        "SELECT space_id FROM backup_current_space WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let original_cursor: String = baseline.query_row(
        "SELECT cursor_json FROM telegram_observed_prefix WHERE space_id=?",
        [&selected],
        |r| r.get(0),
    )?;
    let original_cursor: super::telegram_replay::Cursor = serde_json::from_str(&original_cursor)?;
    let audit: Vec<Value> =
        serde_json::from_slice(&std::fs::read(work.join("r2-audit.private.json"))?)?;
    let core = crate::desktop_core::Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let result=async {
        anyhow::ensure!(super::spaces::current_id(&core)?.as_deref()==Some(selected.as_str()),"RECOVERY_SELECTION_CHANGED");
        anyhow::ensure!(!super::spaces::enabled(&core)?,"RECOVERY_EXPECTED_PAUSED_SYNC");
        let unfinished:i64=core.backup_db(|db|Ok(db.query_row("SELECT COUNT(*) FROM backup_operations WHERE state NOT IN ('completed','canceled')",[],|r|r.get(0))?))?;
        anyhow::ensure!(unfinished==0,"RECOVERY_UNFINISHED_OPERATIONS");
        let mut untouched=0;
        for space in core.backup_db(repository::spaces)? {
            if super::spaces::config(&core,&space.space_id)?.provider_type!=ProviderType::S3 {continue;}
            let config=super::spaces::config(&core,&space.space_id)?;
            let credentials:Credentials=serde_json::from_str(&SystemVault.read(space.credential_ref.as_deref().context("BACKUP_CREDENTIALS_MISSING")?)?)?;
            let store=super::s3_client::S3Store::configured(&space,&credentials.access_key_id,&credentials.secret_access_key,&config.region,config.path_style)?;
            let mut token=None;let mut found=std::collections::BTreeMap::new();
            loop {let page=store.list(&format!("{}records/",space.prefix),token).await?;for object in page.objects {found.insert(object.key,object.etag);}token=page.next;if token.is_none(){break;}}
            for record in audit[0]["records"].as_array().context("AUDIT_INVALID")? {anyhow::ensure!(found.get(record["key"].as_str().context("AUDIT_INVALID")?).is_some_and(|e|record["etag"]==*e),"RECOVERY_EXISTING_R2_RECORD_CHANGED");untouched+=1;}
        }
        let space=core.backup_db(repository::spaces)?.into_iter().find(|s|s.space_id==selected).context("BACKUP_SPACE_NOT_FOUND")?;
        let binding=super::telegram_spaces::binding(&core,&selected)?.context("BACKUP_INVALID_SPACE")?.0;
        let target=super::telegram_transfer::Target{scope:Scope{space_id:selected.clone(),library_id:space.library_id.clone(),provider_type:ProviderType::Telegram,protocol:2},account_id:binding.account_id,channel_id:binding.channel_id};
        let cfg=super::spaces::config(&core,&selected)?;
        let provider=super::telegram_provider::TelegramProvider::new(&core,target,cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?);
        provider.open_library(false).await?;let fresh=provider.pull_changes(None).await?;
        let cursor:super::telegram_replay::Cursor=serde_json::from_str(&fresh.checkpoint.value)?;cursor.covers(&original_cursor)?;
        core.backup_db(|db|{let check:String=db.query_row("PRAGMA integrity_check",[],|r|r.get(0))?;anyhow::ensure!(check=="ok","DATABASE_ERROR");let mut q=db.prepare("PRAGMA foreign_key_check")?;anyhow::ensure!(q.query([])?.next()?.is_none(),"DATABASE_ERROR");Ok(())})?;
        super::spaces::set_enabled(&core,true).await?;
        let summary=json!({"sync_enabled":true,"default_provider":"telegram","completed_r2_jobs":3,"unchanged_existing_r2_records":untouched,"original_telegram_cursor_evidence":original_cursor.evidence.len(),"current_telegram_cursor_evidence":cursor.evidence.len(),"pending_operations":unfinished,"integrity_check":"ok","foreign_key_errors":0});
        std::fs::write(work.join("final-recovery.redacted.json"),serde_json::to_vec_pretty(&summary)?)?;println!("{summary}");Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}

#[derive(Clone)]
struct RecoveryStore {
    inner: Arc<super::s3_client::S3Store>,
    prefix: String,
    assets: Vec<String>,
    writes: Arc<Mutex<Vec<Value>>>,
}
impl ObjectStore for RecoveryStore {
    fn get(&self, k: &str) -> StoreFuture<'_, Option<Object>> {
        self.inner.get(k)
    }
    fn head(&self, k: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
        self.inner.head(k)
    }
    fn list(&self, p: &str, t: Option<String>) -> StoreFuture<'_, Page> {
        self.inner.list(p, t)
    }
    fn stream(&self, k: &str, o: u64, e: Option<String>) -> StoreFuture<'_, ObjectStream> {
        self.inner.stream(k, o, e)
    }
    fn uploads(&self, k: &str) -> StoreFuture<'_, Vec<String>> {
        self.inner.uploads(k)
    }
    fn parts(&self, k: &str, u: &str) -> StoreFuture<'_, Vec<Part>> {
        self.inner.parts(k, u)
    }
    fn write(&self, operation: Write) -> StoreFuture<'_, String> {
        Box::pin(async move {
            let (key, kind) = match &operation {
                Write::PutFile { key, .. } => (key, "put_file"),
                Write::Put { key, .. } => (key, "put_record"),
                Write::CreateMultipart { key, .. } => (key, "create_multipart"),
                Write::UploadFilePart { key, .. } | Write::UploadPart { key, .. } => {
                    (key, "upload_part")
                }
                Write::CompleteMultipart { key, .. } => (key, "complete_multipart"),
                Write::Delete { .. }
                | Write::DeleteIfMatch { .. }
                | Write::AbortMultipart { .. } => bail!("RECOVERY_DELETE_OR_ABORT_FORBIDDEN"),
            };
            let allowed = self.assets.iter().any(|asset| {
                key == &format!("{}records/{asset}.json", self.prefix)
                    || key
                        .strip_prefix(&format!("{}media/{asset}/", self.prefix))
                        .is_some_and(|name| !name.contains('/') && !name.is_empty())
            });
            anyhow::ensure!(allowed, "RECOVERY_WRITE_OUTSIDE_AUDITED_ASSETS");
            self.writes
                .lock()
                .unwrap()
                .push(json!({"key":key,"kind":kind}));
            self.inner.write(operation).await
        })
    }
}

#[tokio::test]
#[ignore = "explicit authorization for the three existing audited R2 operations; app must be exited"]
async fn authorized_recover_three_r2_operations() -> Result<()> {
    anyhow::ensure!(
        std::env::var("YINGQU_AUTHORIZED_BACKUP_RECOVERY").as_deref()
            == Ok("three-audited-r2-operations"),
        "EXPLICIT_RECOVERY_OPT_IN_REQUIRED"
    );
    let (path, work) = super::compatibility_online_audit::paths()?;
    let audit: Vec<Value> =
        serde_json::from_slice(&std::fs::read(work.join("r2-audit.private.json"))?)?;
    let report = audit.first().context("AUDIT_INVALID")?;
    let jobs = report["blocked"].as_array().context("AUDIT_INVALID")?;
    anyhow::ensure!(
        audit.len() == 1 && jobs.len() == 3,
        "RECOVERY_EXPECTED_THREE_JOBS"
    );
    let core = crate::desktop_core::Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let previous = super::spaces::current_id(&core)?;
    let flags = core.backup_db(repository::spaces)?;
    let writes = Arc::new(Mutex::new(vec![]));
    let result=async {
        anyhow::ensure!(!super::spaces::enabled(&core)?,"RECOVERY_REQUIRES_PAUSED_SYNC");
        let id=report["space_id"].as_str().context("AUDIT_INVALID")?;
        let mut space=flags.iter().find(|s|s.space_id==id).context("BACKUP_SPACE_NOT_FOUND")?.clone();
        anyhow::ensure!(report["library_id"]==space.library_id,"BACKUP_LIBRARY_MISMATCH");
        let config=super::spaces::config(&core,id)?;
        let credentials:Credentials=serde_json::from_str(&SystemVault.read(space.credential_ref.as_deref().context("BACKUP_CREDENTIALS_MISSING")?)?)?;
        let inner=Arc::new(super::s3_client::S3Store::configured(&space,&credentials.access_key_id,&credentials.secret_access_key,&config.region,config.path_style)?);
        let assets=jobs.iter().map(|j|j["asset_id"].as_str().map(str::to_owned).context("AUDIT_INVALID")).collect::<Result<Vec<_>>>()?;
        let store=RecoveryStore{inner,prefix:space.prefix.clone(),assets,writes:writes.clone()};
        space.enabled=true;
        let provider=Arc::new(super::s3_provider::S3Provider::new(store.clone(),space.clone(),config.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?,core.backup_db(repository::device_id)?));
        provider.open_library(false).await?;
        // Recheck all three before mutating their state or uploading any byte.
        for job in jobs {
            let asset=job["asset_id"].as_str().context("AUDIT_INVALID")?;
            let operation=job["operation_id"].as_str().context("AUDIT_INVALID")?;
            let valid:bool=core.backup_db(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE operation_id=?1 AND space_id=?2 AND asset_id=?3 AND kind='upload' AND state='blocked' AND last_error='BACKUP_LEGACY_OUTCOME_UNKNOWN')",params![operation,id,asset],|r|r.get(0))?))?;
            anyhow::ensure!(valid,"RECOVERY_JOB_CHANGED");
            core.backup_db(|db| {
                let (remote,baseline,deleted,pending):(String,Option<String>,bool,String)=db.query_row("SELECT remote_asset_id,baseline_json,remote_deleted,pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![id,asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                let pending:sync_engine::PendingChanges=serde_json::from_str(&pending)?;
                let conflicts:i64=db.query_row("SELECT COUNT(*) FROM backup_conflicts WHERE space_id=?1 AND remote_asset_id=?2",params![id,asset],|r|r.get(0))?;
                anyhow::ensure!(remote==asset&&baseline.is_none()&&!deleted&&!pending.recreate&&pending.choices.is_empty()&&pending.retained_conflicts.is_empty()&&conflicts==0,"RECOVERY_BASELINE_CHANGED");
                Ok(())
            })?;
            anyhow::ensure!(provider.read_asset(asset).await?.is_none(),"RECOVERY_REMOTE_RECORD_NOW_EXISTS");
            for directory in ["media","transcripts"] {
                let page=store.list(&format!("{}{directory}/{asset}/",space.prefix),None).await?;
                anyhow::ensure!(page.objects.is_empty()&&page.next.is_none(),"RECOVERY_REQUIRES_OBJECT_RECONCILIATION");
            }
            if let Some(key)=job["payload"]["media_key"].as_str() { anyhow::ensure!(store.head(key).await?.is_none(),"RECOVERY_REQUIRES_OBJECT_RECONCILIATION"); }
            let source=core.backup_asset(asset)?.context("BACKUP_ASSET_NOT_FOUND")?;
            anyhow::ensure!(source.asset_state=="active","BACKUP_ASSET_DELETED");
            let (hash,_)=repository::fingerprint(source.path.as_deref().context("FILE_MISSING")?)?;
            anyhow::ensure!(job["source_sha256"]==hash && job["local_bytes"]==source.content_bytes,"BACKUP_SOURCE_CHANGED");
        }
        core.backup_db(|db| {let tx=db.unchecked_transaction()?;super::spaces::select_in_transaction(&tx,id,true)?;tx.commit()?;Ok(())})?;
        let engine=SyncEngine::new(core.clone(),provider.clone());
        let mut completed=vec![];
        for job in jobs {
            let asset=job["asset_id"].as_str().context("AUDIT_INVALID")?;
            let operation=job["operation_id"].as_str().context("AUDIT_INVALID")?;
            core.backup_db(|db| {
                let tx=db.unchecked_transaction()?;
                let local=repository::asset(&tx, asset)?.context("BACKUP_ASSET_NOT_FOUND")?;
                let (revision,baseline,deleted):(i64,Option<String>,bool)=tx.query_row("SELECT local_revision,baseline_json,remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![id,asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
                anyhow::ensure!(baseline.is_none()&&!deleted,"RECOVERY_BASELINE_CHANGED");
                let revision=u64::try_from(revision)?.checked_add(1).context("BACKUP_INVALID_OPERATION")?;
                let pending=sync_engine::PendingChanges{fields:sync_engine::fields(&local).into_iter().map(|(name,value)|(name,sync_engine::FieldEdit{value,revision,base:FieldBaseline::Missing})).collect(),media:Some(revision),cover:Some(revision),transcript:Some(revision),..Default::default()};
                tx.execute("UPDATE backup_space_assets SET local_revision=?3,pending_json=?4 WHERE space_id=?1 AND local_asset_id=?2",params![id,asset,i64::try_from(revision)?,serde_json::to_string(&pending)?])?;
                tx.execute("UPDATE backup_operations SET state='running',phase='queued',payload_json=?2,last_error=NULL,next_retry_at=NULL WHERE operation_id=?1 AND state='blocked'",params![operation,json!({"scope":provider.scope()}).to_string()])?;
                tx.commit()?;Ok(())
            })?;
            if let Err(error)=engine.execute_upload(operation).await {
                core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1",params![operation,error.to_string()])?;Ok(())})?;
                return Err(error);
            }
            let remote=provider.read_asset(asset).await?.context("BACKUP_PUBLICATION_UNCONFIRMED")?;
            anyhow::ensure!(remote.snapshot.operation_id==operation&&!remote.snapshot.deleted,"BACKUP_PUBLICATION_UNCONFIRMED");
            let media=remote.snapshot.media.as_ref().context("BACKUP_MEDIA_UNVERIFIED")?;
            let mut reader=provider.download_media(media.clone(),0).await?.reader;
            let mut hash=Sha256::new();let mut total=0;let mut buffer=vec![0;64*1024];
            loop {let count=reader.read(&mut buffer).await?;if count==0 {break;}hash.update(&buffer[..count]);total+=count as u64;}
            anyhow::ensure!(total==media.bytes && job["source_sha256"]==format!("{:x}",hash.finalize()),"BACKUP_CHECKSUM_MISMATCH");
            completed.push(json!({"operation_id":operation,"asset_id":asset,"sha256_verified":true,"bytes":total}));
            std::fs::write(work.join("r2-recovery-completed.private.json"),serde_json::to_vec_pretty(&completed)?)?;
            println!("verified recovered R2 job {} of 3",completed.len());
        }
        Ok::<_,anyhow::Error>(())
    }.await;
    // Restore selection/activation even on failure; global synchronization stays paused.
    let restored = core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        for space in &flags {
            tx.execute(
                "UPDATE backup_spaces SET enabled=?2 WHERE space_id=?1",
                params![space.space_id, space.enabled],
            )?;
        }
        if let Some(id) = previous {
            tx.execute(
                "UPDATE backup_current_space SET space_id=?1 WHERE id=1",
                [id],
            )?;
        } else {
            tx.execute("DELETE FROM backup_current_space WHERE id=1", [])?;
        }
        tx.commit()?;
        Ok(())
    });
    std::fs::write(
        work.join("r2-recovery-writes.private.json"),
        serde_json::to_vec_pretty(&*writes.lock().unwrap())?,
    )?;
    core.shutdown().await;
    restored?;
    result
}
