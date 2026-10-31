//! User choices are durable local revisions, published by the same conditional sync path.
use super::{
    provider::*,
    repository,
    sync_engine::{ConflictChoice, PendingChanges},
    sync_read,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct VersionView {
    pub id: String,
    pub field: String,
    pub value: String,
    pub image: Option<Vec<u8>>,
    pub device_id: String,
    pub can_adopt: bool,
}
#[derive(Serialize)]
pub struct LocalData {
    pub title: String,
    pub transcript: Option<String>,
    pub pending: bool,
    pub remote_deleted: bool,
    pub conflicts: Vec<VersionView>,
}
fn read(core: &Core, space: &str, local: &str, id: &str) -> Result<ConflictVersion> {
    core.backup_db(|db|{
        let raw:String=db.query_row("SELECT c.value_json FROM backup_conflicts c JOIN backup_space_assets m USING(space_id,remote_asset_id) WHERE c.space_id=?1 AND m.local_asset_id=?2 AND c.conflict_id=?3",params![space,local,id],|r|r.get(0))?;
        Ok(serde_json::from_str(&raw)?)
    })
}
pub fn local_data(core: &Core, space: &str, local: &str) -> Result<LocalData> {
    core.backup_db(|db|{
        let (raw,deleted):(String,bool)=db.query_row("SELECT pending_json,remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,local],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let pending:PendingChanges=serde_json::from_str(&raw)?;
        let asset=repository::list_assets(db)?.into_iter().find(|a|a.asset_id==local).context("BACKUP_ASSET_NOT_FOUND")?;
        let mut q=db.prepare("SELECT c.conflict_id,c.value_json FROM backup_conflicts c JOIN backup_space_assets m USING(space_id,remote_asset_id) WHERE c.space_id=?1 AND m.local_asset_id=?2 ORDER BY c.conflict_id")?;
        let rows=q.query_map(params![space,local],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let conflicts=rows.into_iter().map(|(id,raw)|->Result<_>{let c:ConflictVersion=serde_json::from_str(&raw)?;Ok(view(id,&c,deleted))}).collect::<Result<_>>()?;
        Ok(LocalData{title:asset.metadata["title"].as_str().unwrap_or(&asset.original_filename).into(),transcript:repository::transcript(db,local)?.map(|t|t.text),pending:!pending.is_empty(),remote_deleted:deleted,conflicts})
    })
}
/// Export the durable content, not the presentation model's preview placeholders.
pub fn export_data(core: &Core, space: &str, local: &str) -> Result<serde_json::Value> {
    core.backup_db(|db|{
        let(remote,revision,pending,deleted,library):(String,i64,String,bool,String)=db.query_row("SELECT m.remote_asset_id,m.local_revision,m.pending_json,m.remote_deleted,s.library_id FROM backup_space_assets m JOIN backup_spaces s USING(space_id) WHERE m.space_id=?1 AND m.local_asset_id=?2",params![space,local],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let mut q=db.prepare("SELECT value_json FROM backup_conflicts WHERE space_id=?1 AND remote_asset_id=?2")?;
        let raw=q.query_map(params![space,remote],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let conflicts=raw.iter().map(|v|serde_json::from_str::<ConflictVersion>(v)).collect::<std::result::Result<Vec<_>,_>>()?;
        let asset=repository::list_assets(db)?.into_iter().find(|a|a.asset_id==local).context("BACKUP_ASSET_NOT_FOUND")?;
        Ok(serde_json::json!({"schema_version":1,"space_id":space,"library_id":library,"local_asset_id":local,"remote_asset_id":remote,"local_revision":revision,"remote_deleted":deleted,"metadata":asset.metadata,"pending":serde_json::from_str::<PendingChanges>(&pending)?,"transcript":repository::transcript(db,local)?,"conflicts":conflicts}))
    })
}
fn view(id: String, c: &ConflictVersion, deleted: bool) -> VersionView {
    VersionView {
        id,
        image: None,
        field: c.field.clone(),
        value: if matches!(c.field.as_str(), "cover" | "transcript") && !c.value.is_null() {
            "点击读取版本".into()
        } else {
            c.value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| c.value.to_string())
        },
        device_id: c.device_id.clone(),
        can_adopt: !deleted,
    }
}
pub async fn detail(
    core: &Core,
    provider: &dyn BackupProvider,
    local: &str,
    id: &str,
) -> Result<VersionView> {
    let c = read(core, &provider.scope().space_id, local, id)?;
    let mut result = view(
        id.into(),
        &c,
        local_data(core, &provider.scope().space_id, local)?.remote_deleted,
    );
    if matches!(c.field.as_str(), "cover" | "transcript") && !c.value.is_null() {
        let r: MediaRef = serde_json::from_value(c.value)?;
        r.validate_scope(provider.scope())?;
        let body = match sync_read::small(core, provider, &r).await {
            Ok(body) => body,
            Err(error) if error.to_string() == "BACKUP_OBJECT_NOT_FOUND" => {
                result.value = "该冲突文件不存在，引用记录已保留，可导出".into();
                result.can_adopt = false;
                return Ok(result);
            }
            Err(error) => return Err(error),
        };
        if c.field == "transcript" {
            let t: super::model::AssetTranscript = serde_json::from_slice(&body)?;
            if t.asset_id != sync_read::remote_id(core, &provider.scope().space_id, local)? {
                bail!("BACKUP_INVALID_TRANSCRIPT");
            }
            result.value = t.text;
        } else {
            crate::files::image_extension(&body).context("INVALID_MEDIA_CONTENT")?;
            result.value = "封面版本".into();
            result.image = Some(body);
        }
    }
    Ok(result)
}


pub async fn resolve(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    local: &str,
    id: &str,
    adopt: bool,
) -> Result<()> {
    let scope = provider.scope();
    let conflict = read(core, &scope.space_id, local, id)?;
    let latest = sync_read::latest(core, provider, local).await?;
    if latest.snapshot.deleted {
        bail!("BACKUP_ASSET_DELETED");
    }
    if !latest.snapshot.conflicts.contains(&conflict) {
        bail!("BACKUP_CONFLICT_CHANGED");
    }
    // Reading a small version never fetches or restores the video.
    let selected = if adopt && conflict.field == "transcript" && !conflict.value.is_null() {
        let r: MediaRef = serde_json::from_value(conflict.value.clone())?;
        r.validate_scope(scope)?;
        let mut t: super::model::AssetTranscript =
            serde_json::from_slice(&sync_read::small(core, provider, &r).await?)?;
        if t.asset_id != latest.snapshot.asset_id
            || t.content_sha256 != latest.snapshot.media.as_ref().map(|m| m.sha256.clone())
        {
            bail!("BACKUP_INVALID_TRANSCRIPT");
        }
        t.asset_id = local.into();
        Some(t)
    } else {
        None
    };
    let selected_cover = if adopt && conflict.field == "cover" && !conflict.value.is_null() {
        let reference: MediaRef = serde_json::from_value(conflict.value.clone())?;
        reference.validate_scope(scope)?;
        let body = sync_read::small(core, provider, &reference).await?;
        crate::files::image_extension(&body).context("INVALID_MEDIA_CONTENT")?;
        Some(body)
    } else { None };
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let (raw,revision,deleted):(String,i64,bool)=tx.query_row("SELECT pending_json,local_revision,remote_deleted FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let mut pending:PendingChanges=serde_json::from_str(&raw)?;
        if deleted {bail!("BACKUP_ASSET_DELETED");}
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM backup_current_space c JOIN backup_spaces s USING(space_id) WHERE s.space_id=?1 AND s.enabled=1)",[&scope.space_id],|r|r.get(0))?;
        if !active {bail!("BACKUP_SPACE_INACTIVE");}
        if !pending.is_empty(){bail!("BACKUP_LOCAL_CHANGES_PENDING");}
        let inflight:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND asset_id=?2 AND kind='upload' AND state IN ('pending','running','retry_wait'))",params![scope.space_id,local],|r|r.get(0))?;
        if inflight {bail!("BACKUP_LOCAL_CHANGES_PENDING");}
        if let Some(t)=selected {pending.transcript=Some(u64::try_from(revision+1)?);tx.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,model_name,pipeline_revision,completed_at,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,NULL) ON CONFLICT(asset_id) DO UPDATE SET text=excluded.text,content_sha256=excluded.content_sha256,model_revision=excluded.model_revision,model_name=excluded.model_name,pipeline_revision=excluded.pipeline_revision,completed_at=excluded.completed_at,source_json=NULL",params![local,t.content_sha256,t.text,t.model_revision,t.model_name,t.pipeline_revision,i64::try_from(t.completed_at)?])?;}
        if let Some(body)=selected_cover {
            tx.execute("INSERT INTO asset_covers VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET body=excluded.body",params![local,body])?;
            pending.cover=Some(u64::try_from(revision+1)?);
        }
        // This explicit choice is based on the fresh record above, not a previous edit's base.
        pending.reference_bases.remove(&conflict.field);
        pending.choices.push(ConflictChoice{conflict,adopt,revision:u64::try_from(revision+1)?});
        tx.execute("UPDATE backup_space_assets SET pending_json=?3,local_revision=?4,baseline_json=?5 WHERE space_id=?1 AND local_asset_id=?2",params![scope.space_id,local,serde_json::to_string(&pending)?,revision+1,serde_json::to_string(&latest.snapshot)?])?;
        tx.commit()?;Ok(())
    })?;
    core.wake_backup();
    Ok(())
}
