//! Persisted controls for the current provider queue.
use crate::desktop_core::Core;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use rusqlite::OptionalExtension;

/// Delayed retries and background verification leave the foreground available for discovery.
pub(super) fn foreground_delete_busy(db: &rusqlite::Connection, space: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND kind='delete'
         AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)<>1
         AND (state='running' OR (state IN ('pending','retry_wait')
              AND (next_retry_at IS NULL OR next_retry_at<=unixepoch()))))",
        [space], |row| row.get(0),
    )?)
}

/// Select due work without promoting delayed retries or completed local cleanup.
pub(super) fn next_sync_operation(
    db: &rusqlite::Connection,
    space: &str,
) -> Result<Option<(String, bool)>> {
    Ok(db.query_row(
        "SELECT operation_id,kind='delete' AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)<>1 FROM backup_operations
         WHERE space_id=? AND kind IN ('upload','delete') AND state IN ('pending','retry_wait') AND (next_retry_at IS NULL OR next_retry_at<=unixepoch())
         ORDER BY CASE WHEN kind='delete' AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)<>1 THEN 0 WHEN kind='upload' THEN 1 ELSE 2 END,created_at,rowid LIMIT 1",
        [space], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?)
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

pub fn retry(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db| {
        let changed=db.execute("UPDATE backup_operations SET state='pending',next_retry_at=NULL,last_error=NULL,attempt=0 WHERE operation_id=? AND kind<>'restore' AND state IN ('blocked','retry_wait','canceled') AND (kind<>'upload' OR EXISTS(SELECT 1 FROM media_assets a WHERE a.asset_id=backup_operations.asset_id AND a.asset_state='active'))",[id])?;
        if changed!=1 {bail!("BACKUP_INVALID_OPERATION");}
        Ok(())
    })
}

pub fn cancel(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db| {
        let changed=db.execute("UPDATE backup_operations SET state='canceled',last_error=NULL WHERE operation_id=? AND kind='upload' AND state NOT IN ('completed','canceled')",[id])?;
        if changed!=1 {bail!("BACKUP_INVALID_OPERATION");}
        Ok(())
    })
}
