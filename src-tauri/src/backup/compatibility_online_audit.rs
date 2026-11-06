//! Explicitly authorized, read-only audit of an existing application profile.
//! No probe, upload, publish, abort, delete, or synchronization worker is started.
use super::{
    repository,
    settings::{Credentials, SystemVault, Vault},
    store::ObjectStore,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) fn paths() -> Result<(PathBuf, PathBuf)> {
    anyhow::ensure!(
        std::env::var("YINGQU_AUTHORIZED_BACKUP_AUDIT").as_deref()
            == Ok("existing-cloud-read-only"),
        "EXPLICIT_AUDIT_OPT_IN_REQUIRED"
    );
    let work = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("repository")?
        .join(".local/backup-compatibility-recovery");
    anyhow::ensure!(
        work.join("before-recovery.sqlite").is_file(),
        "PRE_RECOVERY_SNAPSHOT_REQUIRED"
    );
    let data = PathBuf::from(std::env::var("APPDATA")?).join("me.disign.yingqu");
    Ok((data.join("yingqu.sqlite"), work))
}

#[tokio::test]
#[ignore = "explicit existing-profile authorization and protected local snapshot required"]
async fn authorized_read_only_r2_audit() -> Result<()> {
    let (path, work) = paths()?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut spaces = vec![];
    for space in repository::spaces(&db)? {
        let cfg: (String, String, bool) = db.query_row(
            "SELECT provider_type,region,path_style FROM backup_provider_config WHERE space_id=?",
            [&space.space_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if cfg.0 != "s3" {
            continue;
        }
        let credentials: Credentials = serde_json::from_str(
            &SystemVault.read(
                space
                    .credential_ref
                    .as_deref()
                    .context("BACKUP_CREDENTIALS_MISSING")?,
            )?,
        )?;
        let store = super::s3_client::S3Store::configured(
            &space,
            &credentials.access_key_id,
            &credentials.secret_access_key,
            &cfg.1,
            cfg.2,
        )?;
        let library = store
            .get(&format!("{}library.json", space.prefix))
            .await?
            .context("BACKUP_LIBRARY_NOT_FOUND")?;
        let raw: Value = serde_json::from_slice(&library.body)?;
        let library_validation =
            serde_json::from_slice::<super::library::LibraryMetadata>(&library.body)
                .map_err(anyhow::Error::from)
                .and_then(|v| {
                    v.validate()?;
                    anyhow::ensure!(v.library_id == space.library_id, "BACKUP_LIBRARY_MISMATCH");
                    Ok(())
                });
        let mut records = vec![];
        let mut token = None;
        loop {
            let page = store
                .list(&format!("{}records/", space.prefix), token)
                .await?;
            for info in page.objects {
                anyhow::ensure!(records.len() < 10000, "AUDIT_RECORD_LIMIT");
                let body = store
                    .get(&info.key)
                    .await?
                    .context("BACKUP_OBJECT_NOT_FOUND")?;
                let value: Value = serde_json::from_slice(&body.body)?;
                let asset = value["asset_id"]
                    .as_str()
                    .context("BACKUP_INVALID_RECORD")?;
                let validation = super::s3_provider::validate_wire(
                    &body.body,
                    &space.library_id,
                    asset,
                    &space.prefix,
                );
                records.push(json!({"key":info.key,"etag":body.etag,"current_valid":validation.is_ok(),"validation_error":validation.err().map(|e|e.to_string()),"record":value}));
            }
            token = page.next;
            if token.is_none() {
                break;
            }
        }
        let blocked = {
            let mut q = db.prepare("SELECT operation_id,asset_id,payload_json FROM backup_operations WHERE space_id=? AND state='blocked' AND kind='upload' ORDER BY created_at,rowid")?;
            let rows = q.query_map([&space.space_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut operations = vec![];
        for (operation, asset, payload) in blocked {
            let payload: Value = serde_json::from_str(&payload)?;
            let mut objects = vec![];
            let mut keys = std::collections::BTreeSet::new();
            for key in [
                payload["media_key"].as_str(),
                payload["cover_key"].as_str(),
                payload["transcript_key"].as_str(),
            ]
            .into_iter()
            .flatten()
            {
                keys.insert(key.to_owned());
            }
            let mut token = None;
            loop {
                let page = store
                    .list(&format!("{}media/{asset}/", space.prefix), token)
                    .await?;
                for info in page.objects {
                    keys.insert(info.key);
                    anyhow::ensure!(keys.len() <= 100, "AUDIT_OBJECT_LIMIT");
                }
                token = page.next;
                if token.is_none() {
                    break;
                }
            }
            for key in keys {
                anyhow::ensure!(key.starts_with(&space.prefix), "BACKUP_SCOPE_MISMATCH");
                let info = store.head(&key).await?;
                objects.push(json!({"key":key,"exists":info.is_some(),"bytes":info.as_ref().map(|i|i.bytes),"sha256":info.and_then(|i|i.sha256)}));
            }
            let local:(Option<String>,i64,Option<String>)=db.query_row("SELECT a.content_sha256,a.content_bytes,f.path FROM media_assets a LEFT JOIN asset_local_files f USING(asset_id) WHERE a.asset_id=?",[&asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            let source = local
                .2
                .as_ref()
                .filter(|p| std::path::Path::new(p).is_file())
                .map(|p| repository::fingerprint(std::path::Path::new(p)))
                .transpose()?;
            let remote = records.iter().find(|r| r["record"]["asset_id"] == asset);
            operations.push(json!({"operation_id":operation,"asset_id":asset,"remote_record":remote,"objects":objects,"local_sha256":local.0,"local_bytes":local.1,"source_sha256":source.map(|s|s.0),"payload":payload}));
        }
        spaces.push(json!({"space_id":space.space_id,"library_id":space.library_id,"prefix":space.prefix,"library":raw,"library_current_valid":library_validation.is_ok(),"library_error":library_validation.err().map(|e|e.to_string()),"records":records,"blocked":operations}));
    }
    let summary:Vec<_>=spaces.iter().map(|s|json!({"library_current_valid":s["library_current_valid"],"library_error":s["library_error"],"records":s["records"].as_array().map(Vec::len),"invalid_records":s["records"].as_array().map(|v|v.iter().filter(|r|r["current_valid"]!=true).count()),"blocked":s["blocked"].as_array().map(|v|v.iter().map(|o|json!({"remote_record_exists":!o["remote_record"].is_null(),"existing_objects":o["objects"].as_array().map(|v|v.iter().filter(|i|i["exists"]==true).count()),"local_hash_matches":o["local_sha256"]==o["source_sha256"]})).collect::<Vec<_>>())})).collect();
    std::fs::write(
        work.join("r2-audit.private.json"),
        serde_json::to_vec_pretty(&spaces)?,
    )?;
    std::fs::write(
        work.join("r2-audit.redacted.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{}", serde_json::to_string(&summary)?);
    Ok(())
}

#[test]
#[ignore = "explicit authorization to read protected, previously downloaded metadata snapshot"]
fn authorized_validate_r2_snapshot() -> Result<()> {
    let (_, work) = paths()?;
    let reports: Vec<Value> =
        serde_json::from_slice(&std::fs::read(work.join("r2-audit.private.json"))?)?;
    let mut count = 0;
    for report in reports {
        let library: super::library::LibraryMetadata =
            serde_json::from_value(report["library"].clone())?;
        library.validate()?;
        anyhow::ensure!(
            report["library_id"] == library.library_id,
            "BACKUP_LIBRARY_MISMATCH"
        );
        for record in report["records"].as_array().context("AUDIT_INVALID")? {
            let body = serde_json::to_vec(&record["record"])?;
            super::s3_provider::validate_wire(
                &body,
                &library.library_id,
                record["record"]["asset_id"]
                    .as_str()
                    .context("AUDIT_INVALID")?,
                report["prefix"].as_str().context("AUDIT_INVALID")?,
            )?;
            count += 1;
        }
    }
    std::fs::write(
        work.join("r2-compatibility.redacted.json"),
        serde_json::to_vec_pretty(&json!({"validated_records":count,"remote_writes":0}))?,
    )?;
    println!("validated {count} existing R2 records; remote writes=0");
    Ok(())
}

#[test]
#[ignore = "explicit authorization to inspect protected local Telegram index cache"]
fn authorized_validate_telegram_snapshot() -> Result<()> {
    let (path, work) = paths()?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut reports = vec![];
    for space in repository::spaces(&db)? {
        let binding: Option<(String, String)> = db
            .query_row(
                "SELECT account_id,channel_id FROM telegram_spaces WHERE space_id=?",
                [&space.space_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((account_id, channel_id)) = binding else {
            continue;
        };
        let target = super::telegram_transfer::Target {
            scope: super::provider::Scope {
                space_id: space.space_id.clone(),
                library_id: space.library_id.clone(),
                provider_type: super::provider::ProviderType::Telegram,
                protocol: 2,
            },
            account_id,
            channel_id,
        };
        let docs = {
            let mut q =
                db.prepare("SELECT document_json FROM telegram_mapped_documents WHERE space_id=?")?;
            let rows = q
                .query_map([&space.space_id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|r| serde_json::from_str::<super::telegram_transfer::RemoteDocument>(&r))
                .collect::<serde_json::Result<Vec<_>>>()?
        };
        let mut q=db.prepare("SELECT index_body FROM telegram_mapped_documents WHERE space_id=? AND revision IS NOT NULL")?;
        for body in q.query_map([&space.space_id], |r| r.get::<_, Vec<u8>>(0))? {
            let body = body?;
            let raw: Value = serde_json::from_slice(&body)?;
            if raw.get("change").is_none() {
                if let Some(archived) = raw["archived_events"].as_array() {
                    let mut errors = std::collections::BTreeMap::<String, usize>::new();
                    for event in archived {
                        let bytes: Vec<u8> = serde_json::from_value(event["body"].clone())?;
                        if let Err(error) = super::telegram_replay::Event::decode_with_documents(
                            &target, &bytes, &docs,
                        ) {
                            *errors.entry(format!("{error:#}")).or_default() += 1;
                        }
                    }
                    reports.push(
                        json!({"archived_events":archived.len(),"archive_decode_errors":errors}),
                    );
                }
                continue;
            }
            let decoded =
                super::telegram_replay::Event::decode_with_documents(&target, &body, &docs);
            reports.push(json!({"decode_error":decoded.err().map(|e|format!("{e:#}"))}));
        }
    }
    std::fs::write(
        work.join("telegram-index-validation.redacted.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("{}", serde_json::to_string(&reports)?);
    Ok(())
}

#[tokio::test]
#[ignore = "explicit existing-profile authorization; application must be fully exited"]
async fn authorized_read_only_telegram_audit() -> Result<()> {
    use super::provider::BackupProvider;
    let (path, work) = paths()?;
    let core = crate::desktop_core::Core::open(
        &path,
        work.join("unused-downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let result = async {
        let mut reports = vec![];
        for mut space in core.backup_db(repository::spaces)? {
            let cfg = super::spaces::config(&core, &space.space_id)?;
            if cfg.provider_type != super::provider::ProviderType::Telegram { continue; }
            space.enabled = true; // Only an in-memory reader target; never persist activation.
            let binding = super::telegram_spaces::binding(&core, &space.space_id)?.context("BACKUP_INVALID_SPACE")?.0;
            let target = super::telegram_transfer::Target {
                scope:super::provider::Scope { space_id:space.space_id.clone(),library_id:space.library_id.clone(),provider_type:super::provider::ProviderType::Telegram,protocol:2 },
                account_id:binding.account_id, channel_id:binding.channel_id,
            };
            let provider = super::telegram_provider::TelegramProvider::new(&core,target,cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?);
            let history = provider.request("compatibility_audit", None, |_, transport, io| Box::pin(async move { transport.audit_existing_formats(io).await })).await?;
            let library = provider.open_library(false).await;
            let changes = if library.is_ok() { Some(provider.pull_changes(None).await) } else { None };
            let cached:i64=core.backup_db(|db|Ok(db.query_row("SELECT COUNT(*) FROM asset_remote_records WHERE space_id=?",[&space.space_id],|r|r.get(0))?))?;
            reports.push(json!({"space_id":space.space_id,"cached_assets":cached,"library_current_valid":library.is_ok(),"library_error":library.err().map(|e|e.to_string()),"pull_error":changes.as_ref().and_then(|v|v.as_ref().err()).map(|e|e.to_string()),"current_assets":changes.as_ref().and_then(|v|v.as_ref().ok()).map(|v|v.assets.len()),"history":history}));
        }
        std::fs::write(work.join("telegram-audit.private.json"),serde_json::to_vec_pretty(&reports)?)?;
        let summary:Vec<_>=reports.iter().map(|v|json!({"cached_assets":v["cached_assets"],"library_current_valid":v["library_current_valid"],"library_error":v["library_error"],"pull_error":v["pull_error"],"current_assets":v["current_assets"],"messages":v["history"].as_array().map(Vec::len),"rejected_rich_messages":v["history"].as_array().map(|h|h.iter().filter(|m|!m["current_error"].is_null()).count())})).collect();
        std::fs::write(work.join("telegram-audit.redacted.json"),serde_json::to_vec_pretty(&summary)?)?;
        println!("{}",serde_json::to_string(&summary)?);
        Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}
