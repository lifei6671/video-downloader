//! Durable single-slot backup scheduling, independent of platform download slots.
use super::{
    model::{Asset, Role},
    record::{Active, ObjectRef, Record},
    repository, settings,
    store::{Client, Condition, ObjectStore, Part, Write},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

const MULTIPART_THRESHOLD: u64 = 100 * 1024 * 1024;
const DEFAULT_PART_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ATTEMPTS: i64 = 5;

pub async fn cleanup_canceled<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    op: &Operation,
) -> Result<()> {
    if let Some(key) = op.payload["media_key"].as_str() {
        for id in client.uploads(key).await? {
            match client
                .write(Write::AbortMultipart {
                    key: key.into(),
                    upload_id: id,
                })
                .await
            {
                Ok(_) => {}
                Err(e) if e.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                Err(e) => return Err(e),
            }
        }
    }
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET phase='cancel_cleaned' WHERE operation_id=? AND state='canceled'",[&op.operation_id])?;Ok(())})
}

async fn probe_cleanup<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    op: &Operation,
) -> Result<()> {
    let key = op.payload["key"]
        .as_str()
        .context("BACKUP_INVALID_OPERATION")?;
    client.write(Write::Delete { key: key.into() }).await?;
    if client.head(key).await?.is_some() {
        bail!("BACKUP_CLEANUP_PENDING");
    }
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='completed',phase='cleaned',last_error=NULL WHERE operation_id=?",[&op.operation_id])?;Ok(())})
}

pub async fn test_connection<S: ObjectStore>(core: &Core, client: &Client<S>) -> Result<()> {
    client.connect().await?;
    if client.space().role == Role::Reader {
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("{}probes/{id}", client.space().prefix);
    let payload = json!({"key":key});
    core.backup_db(|db|{db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES(?1,?2,'probe','running','cleanup',?3)",params![id,client.space().space_id,payload.to_string()])?;Ok(())})?;
    let result = async {
        client
            .write(Write::Put {
                key: key.clone(),
                body: id.as_bytes().to_vec(),
                condition: Condition::Absent,
            })
            .await?;
        if client.get(&key).await?.context("BACKUP_PROBE_FAILED")?.body != id.as_bytes() {
            bail!("BACKUP_PROBE_FAILED");
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let op = operations(core)?
        .into_iter()
        .find(|o| o.operation_id == id)
        .context("BACKUP_INVALID_OPERATION")?;
    let cleanup = probe_cleanup(core, client, &op).await;
    if let Err(error) = cleanup {
        core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='retry_wait',last_error=?2 WHERE operation_id=?1",params![id,error.to_string()])?;Ok(())})?;
        return Err(error);
    }
    result
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub operation_id: String,
    pub space_id: String,
    pub asset_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub phase: String,
    pub payload: Value,
    pub attempt: i64,
    pub last_error: Option<String>,
}

pub fn operations(core: &Core) -> Result<Vec<Operation>> {
    core.backup_db(|db| {
        let mut q=db.prepare("SELECT operation_id,space_id,asset_id,kind,state,phase,payload_json,attempt,last_error FROM backup_operations ORDER BY created_at DESC,rowid DESC")?;
        let rows=q.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get::<_,String>(6)?,r.get(7)?,r.get(8)?)))?;
        rows.map(|r|{let(operation_id,space_id,asset_id,kind,state,phase,payload,attempt,last_error)=r?;Ok(Operation{operation_id,space_id,asset_id,kind,state,phase,payload:serde_json::from_str(&payload)?,attempt,last_error})}).collect()
    })
}

fn checkpoint(core: &Core, op: &Operation, phase: &str, payload: &Value) -> Result<()> {
    core.backup_db(|db| {
        let tx=db.unchecked_transaction()?;
        let changed=tx.execute("UPDATE backup_operations SET phase=?2,payload_json=?3,updated_at=unixepoch() WHERE operation_id=?1 AND state IN ('running','canceled')",params![op.operation_id,phase,payload.to_string()])?;
        if changed!=1 {bail!("CANCELED");}
        if let Some(asset)=&op.asset_id {
            for field in ["media_key","cover_key","transcript_key"] {
                if let Some(key)=payload[field].as_str() {
                    tx.execute("INSERT OR IGNORE INTO backup_owned_objects VALUES(?1,?2,?3)",params![op.space_id,asset,key])?;
                }
            }
        }
        tx.commit()?;Ok(())
    })
}

pub fn retry(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db| {
        let changed=db.execute("UPDATE backup_operations SET state='pending',next_retry_at=NULL,last_error=NULL,attempt=0 WHERE operation_id=? AND kind<>'restore' AND state IN ('blocked','retry_wait','canceled') AND (kind<>'upload' OR EXISTS(SELECT 1 FROM media_assets a WHERE a.asset_id=backup_operations.asset_id AND a.asset_state='active'))",[id])?;
        if changed!=1 {bail!("BACKUP_INVALID_OPERATION");}
        Ok(())
    })
}

/// User action joins the existing upload queue; it never starts a second worker.
pub fn request_upload(core: &Core, space_id: &str, asset_id: &str) -> Result<()> {
    let space = core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|space| space.space_id == space_id)
        .context("BACKUP_SPACE_NOT_FOUND")?;
    if !space.enabled {
        bail!("BACKUP_SPACE_INACTIVE");
    }
    if space.role != Role::Primary {
        bail!("BACKUP_READ_ONLY");
    }
    let asset = core
        .backup_assets()?
        .into_iter()
        .find(|asset| asset.asset_id == asset_id)
        .context("BACKUP_ASSET_NOT_FOUND")?;
    if asset.asset_state != "active" {
        bail!("BACKUP_ASSET_DELETED");
    }
    if !asset.path.as_ref().is_some_and(|path| path.is_file()) {
        bail!("BACKUP_LOCAL_RELEASED");
    }
    enqueue(core)?;
    if let Some(op) = operations(core)?.into_iter().find(|op| {
        op.space_id == space_id
            && op.asset_id.as_deref() == Some(asset_id)
            && op.kind == "upload"
            && op.state != "completed"
    }) {
        // Pending/running is already accepted, making repeated clicks idempotent.
        core.backup_db(|db| {
            db.execute("UPDATE backup_operations SET state='pending',next_retry_at=NULL,last_error=NULL,attempt=0 WHERE operation_id=? AND state IN ('blocked','retry_wait','canceled')",[op.operation_id])?;
            Ok(())
        })?;
    }
    Ok(())
}

pub fn cancel(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db| {
        let changed=db.execute("UPDATE backup_operations SET state='canceled',last_error=NULL WHERE operation_id=? AND kind='upload' AND state NOT IN ('completed','canceled')",[id])?;
        if changed!=1 {bail!("BACKUP_INVALID_OPERATION");}
        Ok(())
    })
}

fn signature(core: &Core, asset: &Asset) -> Result<String> {
    let transcript = core.asset_transcript(&asset.asset_id)?;
    let cover: Option<Vec<u8>> = core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT body FROM asset_covers WHERE asset_id=?",
                [&asset.asset_id],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    let data = serde_json::to_vec(&json!([
        asset.metadata,
        asset.content_sha256,
        transcript,
        cover.map(|b| format!("{:x}", Sha256::digest(b)))
    ]))?;
    Ok(format!("{:x}", Sha256::digest(data)))
}

pub fn enqueue(core: &Core) -> Result<()> {
    let spaces = core.backup_db(repository::spaces)?;
    for space in spaces
        .iter()
        .filter(|s| s.enabled && s.role == Role::Primary)
    {
        for asset in core
            .backup_assets()?
            .into_iter()
            .filter(|a| a.asset_state == "active" && a.path.as_ref().is_some_and(|p| p.is_file()))
        {
            let sig = signature(core, &asset)?;
            core.backup_db(|db| {
                let published:Option<String>=db.query_row("SELECT published_signature FROM backup_asset_versions WHERE space_id=?1 AND asset_id=?2",params![space.space_id,asset.asset_id],|r|r.get(0)).optional()?;
                let canceled:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state='canceled')",params![space.space_id,asset.asset_id],|r|r.get(0))?;
                if published.as_deref()==Some(&sig)||canceled {return Ok(());}
                let payload=json!({"endpoint":space.endpoint,"bucket":space.bucket,"prefix":space.prefix,"signature":sig});
                let tx=db.unchecked_transaction()?;
                tx.execute("INSERT OR IGNORE INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'upload','pending','queued',?4)",params![uuid::Uuid::new_v4().to_string(),space.space_id,asset.asset_id,payload.to_string()])?;
                tx.execute("INSERT OR IGNORE INTO asset_remote_objects(space_id,asset_id,record_key,remote_state) VALUES(?1,?2,?3,'pending')",params![space.space_id,asset.asset_id,format!("{}records/{}.json",space.prefix,asset.asset_id)])?;
                tx.commit()?;
                Ok(())
            })?;
        }
    }
    Ok(())
}

fn still_running(core: &Core, op: &Operation) -> Result<()> {
    core.backup_db(|db| {
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations o JOIN backup_spaces s USING(space_id) LEFT JOIN media_assets a ON a.asset_id=o.asset_id WHERE o.operation_id=? AND o.state='running' AND s.enabled=1 AND s.role='primary' AND (a.asset_state='active' OR o.kind<>'upload'))",[&op.operation_id],|r|r.get(0))?;
        if !valid {bail!("CANCELED");}
        Ok(())
    })
}

async fn send<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    op: &Operation,
    mut write: Write,
    cancel: &mut watch::Receiver<bool>,
) -> Result<String> {
    still_running(core, op)?;
    while core.backup_restore_lock.try_lock().is_err() {
        if *cancel.borrow() {
            bail!("CANCELED");
        }
        still_running(core, op)?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let base = match &mut write {
        Write::PutFile { progress, .. } => {
            *progress = Some(observed.clone());
            Some(0)
        }
        Write::UploadFilePart {
            progress, offset, ..
        } => {
            *progress = Some(observed.clone());
            Some(*offset)
        }
        _ => None,
    };
    let request = client.write(write);
    tokio::pin!(request);
    loop {
        if *cancel.borrow() {
            bail!("CANCELED");
        }
        tokio::select! {
            result=&mut request=>return result,
            _=cancel.changed()=>bail!("CANCELED"),
            _=tokio::time::sleep(Duration::from_millis(250))=>{
                still_running(core,op)?;
                if let Some(base)=base {let completed=base+observed.load(std::sync::atomic::Ordering::Relaxed);core.backup_db(|db|{db.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.completed_bytes',?2) WHERE operation_id=?1 AND state='running'",params![op.operation_id,i64::try_from(completed)?])?;Ok(())})?;}
            },
        }
    }
}

pub async fn upload<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    op: &Operation,
    mut cancel: watch::Receiver<bool>,
) -> Result<()> {
    let _writer = core.backup_write_lock.lock().await;
    let work = upload_body(core, client, op, cancel.clone());
    tokio::pin!(work);
    loop {
        tokio::select! {
            result=&mut work=>return result,
            _=cancel.changed()=>bail!("CANCELED"),
            _=tokio::time::sleep(Duration::from_millis(200))=>still_running(core,op)?,
        }
    }
}

async fn upload_body<S: ObjectStore>(
    core: &Arc<Core>,
    client: &Client<S>,
    op: &Operation,
    mut cancel: watch::Receiver<bool>,
) -> Result<()> {
    let asset_id = op.asset_id.as_deref().context("BACKUP_INVALID_OPERATION")?;
    still_running(core, op)?;
    let library = client.connect().await?;
    let space = client.space();
    if op.payload["endpoint"] != space.endpoint
        || op.payload["bucket"] != space.bucket
        || op.payload["prefix"] != space.prefix
    {
        bail!("BACKUP_SPACE_CHANGED");
    }
    let key = format!("{}records/{asset_id}.json", space.prefix);
    let current = client.get(&key).await?;
    if let Some(current) = &current {
        let record: Record = serde_json::from_slice(&current.body)?;
        record.validate(&library, &space.prefix)?;
        if matches!(record, Record::Deleted(_)) {
            bail!("BACKUP_ASSET_DELETED");
        }
    }
    checkpoint(core, op, "hashing", &op.payload)?;
    fingerprint(core, op, asset_id).await?;
    let asset = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == asset_id)
        .context("BACKUP_ASSET_NOT_FOUND")?;
    let path = asset.path.clone().context("FILE_MISSING")?;
    let mut record = Active::from_asset(&asset, &library, &space.prefix)?;
    {
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(&path).await.context("IO_ERROR")?;
        let mut header = [0u8; 12];
        file.read_exact(&mut header)
            .await
            .context("INVALID_MEDIA_CONTENT")?;
        if !crate::files::video_header_valid(&record.media.container, &header) {
            bail!("INVALID_MEDIA_CONTENT");
        }
    }

    let mut payload = op.payload.clone();
    let hash = asset
        .content_sha256
        .as_deref()
        .context("BACKUP_HASH_PENDING")?;
    let old_hash = payload["source_hash"].as_str();
    if old_hash.is_some_and(|v| v != hash) {
        if let Some(id) = payload["upload_id"].as_str() {
            let old_key = payload["media_key"]
                .as_str()
                .context("BACKUP_INVALID_OPERATION")?;
            match send(
                core,
                client,
                op,
                Write::AbortMultipart {
                    key: old_key.into(),
                    upload_id: id.into(),
                },
                &mut cancel,
            )
            .await
            {
                Ok(_) => {}
                Err(e) if e.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                Err(e) => return Err(e),
            }
        }
        payload
            .as_object_mut()
            .context("BACKUP_INVALID_OPERATION")?
            .remove("upload_id");
    }
    payload["source_hash"] = hash.into();
    payload["media_key"] = record.media.key.clone().into();
    payload["signature"] = signature(core, &asset)?.into();
    checkpoint(core, op, "uploading_media", &payload)?;
    let head = client.head(&record.media.key).await?;
    let media_present = head
        .as_ref()
        .is_some_and(|h| h.bytes == record.media.bytes && h.sha256.as_deref() == Some(hash));
    if head.is_some() && !media_present {
        bail!("BACKUP_OBJECT_CONFLICT");
    }
    if !media_present {
        if asset.content_bytes < MULTIPART_THRESHOLD {
            send(
                core,
                client,
                op,
                Write::PutFile {
                    key: record.media.key.clone(),
                    path: path.clone(),
                    bytes: asset.content_bytes,
                    sha256: hash.into(),
                    progress: None,
                },
                &mut cancel,
            )
            .await?;
        } else {
            let part_bytes = DEFAULT_PART_BYTES
                .max(asset.content_bytes.div_ceil(10000).div_ceil(1024 * 1024) * (1024 * 1024));
            let mut upload_id = payload["upload_id"].as_str().map(str::to_owned);
            let mut parts: Vec<Part> = Vec::new();
            if let Some(id) = &upload_id {
                match client.parts(&record.media.key, id).await {
                    Ok(v) => parts = v,
                    Err(e) if e.to_string() == "BACKUP_UPLOAD_EXPIRED" => upload_id = None,
                    Err(e) => return Err(e),
                }
            }
            if upload_id.is_none() {
                let lost = client.uploads(&record.media.key).await?;
                // Only this asset/content key; clean an uncertain create before restarting.
                for id in lost {
                    send(
                        core,
                        client,
                        op,
                        Write::AbortMultipart {
                            key: record.media.key.clone(),
                            upload_id: id,
                        },
                        &mut cancel,
                    )
                    .await?;
                }
            }
            let id = match upload_id {
                Some(v) => v,
                None => {
                    send(
                        core,
                        client,
                        op,
                        Write::CreateMultipart {
                            key: record.media.key.clone(),
                            sha256: hash.into(),
                        },
                        &mut cancel,
                    )
                    .await?
                }
            };
            payload["upload_id"] = id.clone().into();
            payload["part_bytes"] = part_bytes.into();
            checkpoint(core, op, "uploading_parts", &payload)?;
            let count = asset.content_bytes.div_ceil(part_bytes);
            for number in 1..=count {
                let offset = (number - 1) * part_bytes;
                let bytes = (asset.content_bytes - offset).min(part_bytes);
                if parts
                    .iter()
                    .any(|p| p.number == number as u32 && p.bytes == bytes)
                {
                    continue;
                }
                let etag = send(
                    core,
                    client,
                    op,
                    Write::UploadFilePart {
                        key: record.media.key.clone(),
                        upload_id: id.clone(),
                        part: number as u32,
                        path: path.clone(),
                        offset,
                        bytes,
                        progress: None,
                    },
                    &mut cancel,
                )
                .await?;
                parts.retain(|p| p.number != number as u32);
                parts.push(Part {
                    number: number as u32,
                    etag,
                    bytes,
                });
                payload["parts"] = serde_json::to_value(&parts)?;
                payload["completed_bytes"] = parts.iter().map(|p| p.bytes).sum::<u64>().into();
                checkpoint(core, op, "uploading_parts", &payload)?;
            }
            parts.sort_by_key(|p| p.number);
            checkpoint(core, op, "completing_upload", &payload)?;
            send(
                core,
                client,
                op,
                Write::CompleteMultipart {
                    key: record.media.key.clone(),
                    upload_id: id,
                    parts: parts.into_iter().map(|p| (p.number, p.etag)).collect(),
                },
                &mut cancel,
            )
            .await?;
        }
    }
    payload["completed_bytes"] = asset.content_bytes.into();
    // A completed response is not sufficient if the source changed during transfer.
    fingerprint(core, op, asset_id).await?;
    let verified = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == asset_id)
        .context("BACKUP_ASSET_NOT_FOUND")?;
    if verified.content_sha256 != asset.content_sha256
        || verified.content_bytes != asset.content_bytes
    {
        bail!("BACKUP_SOURCE_CHANGED");
    }
    let cover: Option<Vec<u8>> = core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT body FROM asset_covers WHERE asset_id=?",
                [asset_id],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    if let Some(body) = cover {
        if let Some(ext) = crate::files::image_extension(&body) {
            let object = ObjectRef {
                key: format!(
                    "{}media/{asset_id}/{hash}/{:x}/cover.{ext}",
                    space.prefix,
                    Sha256::digest(&body)
                ),
                bytes: body.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&body)),
                container: ext.into(),
            };
            payload["cover_key"] = object.key.clone().into();
            checkpoint(core, op, "uploading_cover", &payload)?;
            put_small(core, client, op, &object.key, body, &mut cancel).await?;
            record.cover = Some(object);
        }
    }
    if let Some(transcript) = core
        .asset_transcript(asset_id)?
        .filter(|t| t.content_sha256.as_deref() == Some(hash))
    {
        let body = serde_json::to_vec(&transcript)?;
        let object = ObjectRef {
            key: format!(
                "{}transcripts/{asset_id}/{hash}/{:x}/transcript.json",
                space.prefix,
                Sha256::digest(&body)
            ),
            bytes: body.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&body)),
            container: "json".into(),
        };
        payload["transcript_key"] = object.key.clone().into();
        checkpoint(core, op, "uploading_transcript", &payload)?;
        put_small(core, client, op, &object.key, body, &mut cancel).await?;
        record.transcript = Some(object);
    }
    // Re-read after transfers: an interrupted prior publish may already be committed.
    let current = client.get(&key).await?;
    let mut condition = Condition::Absent;
    if let Some(current) = current {
        let old: Record = serde_json::from_slice(&current.body)?;
        old.validate(&library, &space.prefix)?;
        if matches!(old, Record::Deleted(_)) {
            bail!("BACKUP_ASSET_DELETED");
        }
        record.record_revision = old.revision();
        if serde_json::to_value(Record::Active(Box::new(record.clone())))?
            == serde_json::to_value(&old)?
        {
            return finish(core, op, &record, &current.etag, &payload);
        }
        record.record_revision = old.revision() + 1;
        condition = Condition::Match(current.etag);
    }
    let published = Record::Active(Box::new(record.clone()));
    published.validate(&library, &space.prefix)?;
    payload["record"] = serde_json::to_value(&published)?;
    checkpoint(core, op, "publishing_record", &payload)?;
    let etag = send(
        core,
        client,
        op,
        Write::Put {
            key,
            body: serde_json::to_vec(&published)?,
            condition,
        },
        &mut cancel,
    )
    .await?;
    finish(core, op, &record, &etag, &payload)
}

async fn put_small<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    op: &Operation,
    key: &str,
    body: Vec<u8>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<()> {
    let old = client.get(key).await?;
    if old.as_ref().is_some_and(|o| o.body == body) {
        return Ok(());
    }
    if old.is_some() {
        bail!("BACKUP_OBJECT_CONFLICT");
    }
    let condition = Condition::Absent;
    send(
        core,
        client,
        op,
        Write::Put {
            key: key.into(),
            body,
            condition,
        },
        cancel,
    )
    .await?;
    Ok(())
}

fn finish(core: &Core, op: &Operation, record: &Active, etag: &str, payload: &Value) -> Result<()> {
    core.backup_db(|db| {
        let tx=db.unchecked_transaction()?;
        let changed=tx.execute("UPDATE backup_operations SET state='completed',phase='completed',payload_json=?2,last_error=NULL,updated_at=unixepoch() WHERE operation_id=?1 AND state='running'",params![op.operation_id,payload.to_string()])?;
        if changed!=1 {bail!("CANCELED");}
        tx.execute("UPDATE asset_remote_objects SET record_etag=?3,record_revision=?4,media_key=?5,cover_key=?6,transcript_key=?7,remote_state='backed_up' WHERE space_id=?1 AND asset_id=?2",params![op.space_id,record.asset_id,etag,i64::try_from(record.record_revision)?,record.media.key,record.cover.as_ref().map(|o|&o.key),record.transcript.as_ref().map(|o|&o.key)])?;
        tx.execute("INSERT INTO asset_remote_records VALUES(?1,?2,?3) ON CONFLICT(space_id,asset_id) DO UPDATE SET record_json=excluded.record_json",params![op.space_id,record.asset_id,serde_json::to_string(&Record::Active(Box::new(record.clone())))?])?;
        tx.execute("INSERT INTO backup_asset_versions VALUES(?1,?2,?3) ON CONFLICT(space_id,asset_id) DO UPDATE SET published_signature=excluded.published_signature",params![op.space_id,record.asset_id,payload["signature"].as_str().context("BACKUP_INVALID_OPERATION")?])?;
        tx.commit()?;Ok(())
    })
}

pub async fn run(core: Arc<Core>, mut cancel: watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            break;
        }
        let result=async {
            settings::cleanup_credentials(&core,&settings::SystemVault)?;
            enqueue(&core)?;
            for op in cleanup_candidates(&core)? {
                let result=match settings::client(&core,&op.space_id) {
                    Ok(client)=>tokio::select!{_=cancel.changed()=>Err(anyhow::anyhow!("CANCELED")),result=cleanup_canceled(&core,&client,&op)=>result},
                    Err(error)=>Err(error),
                };
                if let Err(error)=result {
                    core.backup_db(|db|{db.execute("UPDATE backup_operations SET attempt=attempt+1,last_error=?2,next_retry_at=unixepoch()+30 WHERE operation_id=?1",params![op.operation_id,error.to_string()])?;Ok(())})?;
                }
            }
            let due:Option<String>=core.backup_db(|db|Ok(db.query_row("SELECT o.operation_id FROM backup_operations o JOIN backup_spaces s USING(space_id) WHERE s.enabled=1 AND s.role='primary' AND o.kind IN ('upload','probe','delete') AND o.state IN ('pending','retry_wait') AND (o.next_retry_at IS NULL OR o.next_retry_at<=unixepoch()) ORDER BY o.created_at,o.rowid LIMIT 1",[],|r|r.get(0)).optional()?))?;
            let selected=operations(&core)?.into_iter().find(|o|Some(&o.operation_id)==due.as_ref());
            if let Some(op)=selected {
                let claimed=core.backup_db(|db|Ok(db.execute("UPDATE backup_operations SET state='running',attempt=attempt+1 WHERE operation_id=? AND state IN ('pending','retry_wait') AND (next_retry_at IS NULL OR next_retry_at<=unixepoch())",[&op.operation_id])?==1))?;
                if !claimed {return Ok(());}
                if op.kind=="delete" {
                    if let Err(error)=super::lifecycle::prepare_local(&core,&op).await {
                        core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1",params![op.operation_id,error.to_string()])?;Ok(())})?;
                        return Ok(());
                    }
                }
                let job_cancel=cancel.clone();
                let result=match settings::client(&core,&op.space_id) {Ok(client)=> {
                    tokio::select! {
                        _=cancel.changed()=>Err(anyhow::anyhow!("CANCELED")),
                        result=async {if op.kind=="probe" {probe_cleanup(&core,&client,&op).await} else if op.kind=="delete" {super::lifecycle::remote(&core,&client,&op).await} else {upload(&core,&client,&op,job_cancel).await}}=>result,
                    }
                },Err(e)=>Err(e)};
                if let Err(error)=result {
                    record_failure(&core,&op,&error.to_string())?;
                }
            }
            Ok::<_,anyhow::Error>(())
        }.await;
        if let Err(error) = result {
            eprintln!("backup scheduler: {error}");
        }
        tokio::select! {_=cancel.changed()=>{},_=tokio::time::sleep(Duration::from_secs(2))=>{}}
    }
}

pub(crate) fn cleanup_candidates(core: &Core) -> Result<Vec<Operation>> {
    let ids:Vec<String>=core.backup_db(|db|{
        let mut q=db.prepare("SELECT o.operation_id FROM backup_operations o JOIN backup_spaces s USING(space_id) WHERE s.enabled=1 AND s.role='primary' AND o.kind='upload' AND o.state='canceled' AND o.phase<>'cancel_cleaned' AND o.attempt<5 AND (o.next_retry_at IS NULL OR o.next_retry_at<=unixepoch()) ORDER BY o.created_at LIMIT 1")?;
        let rows=q.query_map([],|r|r.get(0))?;Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    Ok(operations(core)?
        .into_iter()
        .filter(|o| ids.contains(&o.operation_id))
        .collect())
}

async fn fingerprint(core: &Core, op: &Operation, asset_id: &str) -> Result<()> {
    let (stop, cancel) = watch::channel(false);
    let work = core.verify_backup_asset_cancelable(asset_id, cancel);
    tokio::pin!(work);
    loop {
        tokio::select! {
            result=&mut work=>return result,
            _=tokio::time::sleep(Duration::from_millis(250))=> {
                if still_running(core,op).is_err() {stop.send_replace(true);let _=work.await;bail!("CANCELED");}
            }
        }
    }
}

pub(crate) fn record_failure(core: &Core, op: &Operation, code: &str) -> Result<()> {
    let retryable = matches!(
        code,
        "BACKUP_NETWORK_ERROR"
            | "BACKUP_SERVICE_UNAVAILABLE"
            | "BACKUP_RATE_LIMITED"
            | "BACKUP_PRECONDITION_FAILED"
            | "BACKUP_UPLOAD_EXPIRED"
            | "BACKUP_SOURCE_CHANGED"
            | "CANCELED"
    );
    let state = if retryable && op.attempt + 1 < MAX_ATTEMPTS {
        "retry_wait"
    } else {
        "blocked"
    };
    let delay =
        2_i64.pow((op.attempt as u32).min(6)) + i64::from(uuid::Uuid::new_v4().as_bytes()[0] % 3);
    core.backup_db(|db|{db.execute("UPDATE backup_operations SET state=?2,last_error=?3,next_retry_at=unixepoch()+?4,updated_at=unixepoch() WHERE operation_id=?1 AND state='running'",params![op.operation_id,state,code,delay])?;db.execute("UPDATE asset_remote_objects SET remote_state='error' WHERE space_id=?1 AND asset_id=?2 AND remote_state<>'pending_delete'",params![op.space_id,op.asset_id])?;Ok(())})
}
