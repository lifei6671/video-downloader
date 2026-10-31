//! Read-only, incremental discovery. Listing never reads video or text objects.
use super::{
    model::{Asset, AssetTranscript},
    record::{ObjectRef, Record},
    store::{Client, ObjectStore},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub asset: Asset,
    pub space_id: String,
    pub remote_state: String,
    pub has_cover: bool,
    pub has_transcript: bool,
    pub remote_deleted:bool,
    pub has_pending_local_changes:bool,
    pub conflict_count:usize,
    pub remote_revision:u64,
}
#[derive(Serialize, Default)]
pub struct Refresh {
    pub imported: usize,
    pub unchanged: usize,
    pub rejected: Vec<String>,
}

pub fn list(core: &Core, space_id: &str) -> Result<Vec<Entry>> {
    if super::spaces::config(core,space_id)?.protocol==2 {return super::sync_read::list(core,space_id);}
    let mut entries = Vec::new();
    for asset in core.backup_assets()? {
        let hidden: bool = core.backup_db(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_hidden_history WHERE asset_id=?)",
                [&asset.asset_id],
                |r| r.get(0),
            )?)
        })?;

        let remote:Option<(String,bool,bool)>=core.backup_db(|db|Ok(db.query_row("SELECT remote_state,cover_key IS NOT NULL,transcript_key IS NOT NULL FROM asset_remote_objects WHERE space_id=?1 AND asset_id=?2",params![space_id,asset.asset_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?))?;
        if let Some((remote_state, has_cover, has_transcript)) = remote {
            if hidden && remote_state != "pending_delete" {
                continue;
            }
            entries.push(Entry {
                asset,
                space_id: space_id.into(),
                remote_state,
                has_cover,
                has_transcript,
                remote_deleted:false,has_pending_local_changes:false,conflict_count:0,remote_revision:0,
            });
        }
    }
    Ok(entries)
}

pub async fn refresh<S: ObjectStore>(core: &Core, client: &Client<S>) -> Result<Refresh> {
    let library = client.connect().await?;
    let space = client.space();
    let prefix = format!("{}records/", space.prefix);
    let mut token = None;
    let mut seen = std::collections::HashSet::new();
    let mut result = Refresh::default();
    loop {
        let page = client.list(&prefix, token).await?;
        for object in page.objects {
            let Some(id) = object
                .key
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(".json"))
                .filter(|s| uuid::Uuid::parse_str(s).is_ok())
            else {
                result.rejected.push("BACKUP_INVALID_RECORD_KEY".into());
                continue;
            };
            let cached:Option<String>=core.backup_db(|db|Ok(db.query_row("SELECT record_etag FROM asset_remote_objects WHERE space_id=?1 AND asset_id=?2",params![space.space_id,id],|r|r.get(0)).optional()?.flatten()))?;
            if cached.as_deref() == Some(&object.etag) {
                result.unchanged += 1;
                continue;
            }
            let Some(value) = client.get(&object.key).await? else {
                continue;
            };
            let parsed = (|| {
                let record: Record =
                    serde_json::from_slice(&value.body).context("BACKUP_INVALID_RECORD")?;
                record.validate(&library, &space.prefix)?;
                if record.asset_id() != id {
                    bail!("BACKUP_INVALID_RECORD");
                }
                Ok(record)
            })();
            match parsed {
                Ok(record) => {
                    import(core, &space.space_id, &object.key, &value.etag, &record)?;
                    result.imported += 1;
                }
                Err(error) => result.rejected.push(format!("{id}: {error}")),
            }
        }
        token = page.next;
        if let Some(next) = &token {
            if !seen.insert(next.clone()) {
                bail!("BACKUP_INVALID_PAGE");
            }
        } else {
            break;
        }
    }
    Ok(result)
}

pub fn import(core: &Core, space_id: &str, key: &str, etag: &str, record: &Record) -> Result<()> {
    core.backup_db(|db| {
        let tx=db.unchecked_transaction()?;let id=record.asset_id();
        let previous:Option<(i64,String)>=tx.query_row("SELECT record_revision,remote_state FROM asset_remote_objects WHERE space_id=?1 AND asset_id=?2",params![space_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if previous.as_ref().is_some_and(|(revision,state)|*revision as u64>record.revision()||(state=="deleted"&&matches!(record,Record::Active(_)))) {bail!("BACKUP_RECORD_ROLLBACK");}
        match record {
            Record::Active(v)=>{
                let metadata=serde_json::json!({"platform":v.source.platform,"video_id":v.source.video_id,"canonical_url":v.source.canonical_url,"title":v.title,"author":v.author,"duration_ms":v.duration_ms,"width":v.width,"height":v.height,"completed_at":v.completed_at});
                // A local deletion intent cannot be reversed by an old remote listing.
                tx.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,?3,?4,?5,'active') ON CONFLICT(asset_id) DO UPDATE SET metadata_json=excluded.metadata_json,original_filename=excluded.original_filename,content_sha256=excluded.content_sha256,content_bytes=excluded.content_bytes WHERE media_assets.asset_state='active' AND (SELECT role FROM backup_spaces WHERE space_id=?6)='reader'",params![id,metadata.to_string(),v.original_filename,v.media.sha256,i64::try_from(v.media.bytes)?,space_id])?;
                tx.execute("INSERT OR IGNORE INTO asset_local_files(asset_id,bytes,local_state) VALUES(?1,0,'missing')",[id])?;
                tx.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_key,record_etag,record_revision,media_key,cover_key,transcript_key,remote_state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'backed_up') ON CONFLICT(space_id,asset_id) DO UPDATE SET record_key=excluded.record_key,record_etag=excluded.record_etag,record_revision=excluded.record_revision,media_key=excluded.media_key,cover_key=excluded.cover_key,transcript_key=excluded.transcript_key,remote_state=CASE WHEN asset_remote_objects.remote_state='pending_delete' THEN 'pending_delete' ELSE 'backed_up' END",params![space_id,id,key,etag,i64::try_from(v.record_revision)?,v.media.key,v.cover.as_ref().map(|o|&o.key),v.transcript.as_ref().map(|o|&o.key)])?;
            }
            Record::Deleted(v)=>{
                // Retain a downloaded copy on this device, never delete files on refresh.
                tx.execute("INSERT INTO media_assets VALUES(?1,NULL,'{}','',NULL,1,'deleted') ON CONFLICT(asset_id) DO UPDATE SET asset_state='deleted'",[id])?;
                tx.execute("INSERT OR IGNORE INTO asset_local_files(asset_id,bytes,local_state) VALUES(?1,0,'missing')",[id])?;
                tx.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_key,record_etag,record_revision,remote_state) VALUES(?1,?2,?3,?4,?5,'deleted') ON CONFLICT(space_id,asset_id) DO UPDATE SET record_etag=excluded.record_etag,record_revision=excluded.record_revision,remote_state=CASE WHEN asset_remote_objects.remote_state='pending_delete' THEN 'pending_delete' ELSE 'deleted' END,media_key=NULL,cover_key=NULL,transcript_key=NULL",params![space_id,id,key,etag,i64::try_from(v.record_revision)?])?;
            }
        }
        tx.execute("INSERT INTO asset_remote_records VALUES(?1,?2,?3) ON CONFLICT(space_id,asset_id) DO UPDATE SET record_json=excluded.record_json",params![space_id,id,serde_json::to_string(record)?])?;
        tx.commit()?;Ok(())
    })
}

pub fn cached_record(core: &Core, space_id: &str, asset_id: &str) -> Result<Record> {
    let raw: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT record_json FROM asset_remote_records WHERE space_id=?1 AND asset_id=?2",
            params![space_id, asset_id],
            |r| r.get(0),
        )?)
    })?;
    Ok(serde_json::from_str(&raw)?)
}
pub async fn small_object<S: ObjectStore>(
    client: &Client<S>,
    reference: &ObjectRef,
) -> Result<Vec<u8>> {
    let object = client
        .get(&reference.key)
        .await?
        .context("BACKUP_OBJECT_NOT_FOUND")?;
    if object.body.len() as u64 != reference.bytes
        || format!("{:x}", Sha256::digest(&object.body)) != reference.sha256
    {
        bail!("BACKUP_CHECKSUM_MISMATCH");
    }
    Ok(object.body)
}
pub fn cached_transcript(
    core: &Core,
    space_id: &str,
    asset_id: &str,
) -> Result<Option<AssetTranscript>> {
    let Some(value) = core.asset_transcript(asset_id)? else {
        return Ok(None);
    };
    let local:bool=core.backup_db(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM asset_transcripts t JOIN backup_spaces s ON s.space_id=?2 WHERE t.asset_id=?1 AND (t.source_json IS NOT NULL OR s.role='primary'))",params![asset_id,space_id],|r|r.get(0))?))?;
    if local {
        return Ok(Some(value));
    }
    let record = cached_record(core, space_id, asset_id)?;
    let Record::Active(record) = record else {
        return Ok(Some(value));
    };
    if value.content_sha256.as_deref() == Some(&record.media.sha256)
        && (local
            || record.transcript.as_ref().is_none_or(|r| {
                serde_json::to_vec(&value)
                    .is_ok_and(|b| format!("{:x}", Sha256::digest(b)) == r.sha256)
            }))
    {
        return Ok(Some(value));
    }
    Ok(None)
}
pub async fn transcript<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    asset_id: &str,
) -> Result<Option<AssetTranscript>> {
    if let Some(value) = cached_transcript(core, &client.space().space_id, asset_id)? {
        return Ok(Some(value));
    }
    let record = cached_record(core, &client.space().space_id, asset_id)?;
    let Record::Active(record) = record else {
        return core.asset_transcript(asset_id);
    };
    record_check(client, &Record::Active(record.clone())).await?;
    let Some(reference) = record.transcript.as_ref() else {
        return core.asset_transcript(asset_id);
    };
    let value: AssetTranscript = serde_json::from_slice(&small_object(client, reference).await?)
        .context("BACKUP_INVALID_TRANSCRIPT")?;
    if value.asset_id != asset_id
        || value.content_sha256.as_deref() != Some(&record.media.sha256)
        || value.text.len() > 16 * 1024 * 1024
    {
        bail!("BACKUP_INVALID_TRANSCRIPT");
    }
    core.backup_db(|db|{db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,model_name,pipeline_revision,completed_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(asset_id) DO UPDATE SET content_sha256=excluded.content_sha256,text=excluded.text,model_revision=excluded.model_revision,model_name=excluded.model_name,pipeline_revision=excluded.pipeline_revision,completed_at=excluded.completed_at,source_json=NULL WHERE asset_transcripts.source_json IS NULL AND (SELECT role FROM backup_spaces WHERE space_id=?8)='reader'",params![asset_id,value.content_sha256,value.text,value.model_revision,value.model_name,value.pipeline_revision,i64::try_from(value.completed_at)?,client.space().space_id])?;Ok(())})?;
    core.asset_transcript(asset_id)
}
async fn record_check<S: ObjectStore>(client: &Client<S>, record: &Record) -> Result<()> {
    record.validate(&client.connect().await?, &client.space().prefix)
}
pub async fn cover<S: ObjectStore>(
    core: &Core,
    client: &Client<S>,
    asset_id: &str,
) -> Result<Vec<u8>> {
    let record = cached_record(core, &client.space().space_id, asset_id)?;
    let Record::Active(record) = record else {
        bail!("BACKUP_ASSET_DELETED");
    };
    let reference = record.cover.clone().context("COVER_UNAVAILABLE")?;
    let cached: Option<Vec<u8>> = core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT body FROM asset_covers WHERE asset_id=?",
                [asset_id],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    if let Some(body) = cached.filter(|b| format!("{:x}", Sha256::digest(b)) == reference.sha256) {
        return Ok(body);
    }
    record_check(client, &Record::Active(record)).await?;
    let body = small_object(client, &reference).await?;
    if crate::files::image_extension(&body).is_none() {
        bail!("INVALID_MEDIA_CONTENT");
    }
    core.backup_db(|db|{db.execute("INSERT INTO asset_covers VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET body=excluded.body",params![asset_id,body])?;Ok(())})?;
    Ok(body)
}
