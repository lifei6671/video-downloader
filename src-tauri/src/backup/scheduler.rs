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

pub(super) fn upload_running(db: &rusqlite::Connection, space: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_operations WHERE space_id=?1 AND kind='upload' AND state='running')",
        [space], |row| row.get(0),
    )?)
}

/// Select due work without promoting delayed retries or completed local cleanup.
pub(super) fn next_sync_operation(
    db: &rusqlite::Connection,
    space: &str,
) -> Result<Option<(String, bool)>> {
    select_sync_operation(db, space, true)
}

pub(super) fn next_upload_operation(
    db: &rusqlite::Connection,
    space: &str,
) -> Result<Option<(String, bool)>> {
    select_sync_operation(db, space, false)
}

fn select_sync_operation(
    db: &rusqlite::Connection,
    space: &str,
    include_deletes: bool,
) -> Result<Option<(String, bool)>> {
    let uploads_allowed = !foreground_delete_busy(db, space)?;
    Ok(db.query_row(
        "SELECT operation_id,kind='delete' AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)<>1 FROM backup_operations
         WHERE space_id=?1 AND kind IN ('upload','delete') AND state IN ('pending','retry_wait') AND (next_retry_at IS NULL OR next_retry_at<=unixepoch())
         AND (kind='upload' OR ?2) AND (kind<>'upload' OR ?3)
         AND (kind<>'upload' OR NOT EXISTS(SELECT 1 FROM backup_operations cleanup
             WHERE cleanup.space_id=backup_operations.space_id AND cleanup.asset_id=backup_operations.asset_id
             AND cleanup.kind='upload' AND cleanup.phase<>'cancel_cleaned'
             AND (cleanup.state='canceled' OR (cleanup.state='blocked' AND cleanup.last_error='BACKUP_ASSET_DELETED'))))
         ORDER BY CASE WHEN kind='delete' AND COALESCE(json_extract(payload_json,'$.local_cleanup_done'),0)<>1 THEN 0 WHEN kind='upload' THEN 1 ELSE 2 END,
         CASE WHEN EXISTS (SELECT 1 FROM backup_operations parked WHERE parked.space_id=backup_operations.space_id
             AND parked.phase='archive_ready' AND parked.state='pending'
             AND json_extract(parked.payload_json,'$.archive_next')=backup_operations.operation_id) THEN 0 ELSE 1 END,
         created_at,rowid LIMIT 1",
        rusqlite::params![space,include_deletes,uploads_allowed], |row| Ok((row.get(0)?,row.get(1)?)),
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
    select_operations(core, "ORDER BY created_at DESC,rowid DESC", [])
}

/// UI state only. Keep captured media and recovery evidence in the worker payload.
pub fn operations_for_display(core: &Core) -> Result<Vec<Operation>> {
    core.backup_db(|db| {
        let mut query = db.prepare("SELECT operation_id,space_id,asset_id,kind,state,phase,
            json_extract(payload_json,'$.completed_bytes','$.transfer_started','$.local_cleanup_done',
                '$.archive_next','$.cleanup_dependency','$.cleanup_error','$.cleanup_blocked'),attempt,last_error
            FROM backup_operations ORDER BY created_at DESC,rowid DESC")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get::<_,String>(6)?,row.get(7)?,row.get(8)?)))?;
        rows.map(|row| {
            let (operation_id,space_id,asset_id,kind,state,phase,projection,attempt,last_error) = row?;
            let values: [Value; 7] = serde_json::from_str(&projection)?;
            let payload = ["completed_bytes","transfer_started","local_cleanup_done","archive_next","cleanup_dependency","cleanup_error","cleanup_blocked"]
                .into_iter().zip(values).filter(|(_,value)| !value.is_null()).map(|(key,value)| (key.to_owned(),value)).collect();
            Ok(Operation { operation_id,space_id,asset_id,kind,state,phase,payload:Value::Object(payload),attempt,last_error })
        }).collect()
    })
}

pub(super) fn operation(core: &Core, id: &str) -> Result<Operation> {
    select_operations(core, "WHERE operation_id=?1", [id])?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("BACKUP_INVALID_OPERATION"))
}

pub(super) fn uploads_to_clean(core: &Core, space: &str) -> Result<Vec<Operation>> {
    select_operations(core, "WHERE space_id=?1 AND kind='upload' AND phase<>'cancel_cleaned'
        AND (state='canceled' OR (state='blocked' AND last_error='BACKUP_ASSET_DELETED'))
        AND COALESCE(json_extract(payload_json,'$.cleanup_blocked'),0)=0
        AND (next_retry_at IS NULL OR next_retry_at<=unixepoch())
        ORDER BY created_at DESC,rowid DESC", [space])
}

pub(super) fn park_upload_cleanup_dependencies(core: &Core, space: &str) -> Result<()> {
    core.backup_db(|db| {
        db.execute("UPDATE backup_operations AS waiting SET state='retry_wait',last_error='BACKUP_CLEANUP_PENDING',
            payload_json=json_set(payload_json,'$.cleanup_dependency',(SELECT old.operation_id FROM backup_operations old
                WHERE old.space_id=waiting.space_id AND old.asset_id=waiting.asset_id AND old.kind='upload'
                AND old.phase<>'cancel_cleaned' AND (old.state='canceled' OR (old.state='blocked' AND old.last_error='BACKUP_ASSET_DELETED'))
                ORDER BY old.created_at DESC,old.rowid DESC LIMIT 1)),updated_at=unixepoch()
            WHERE waiting.space_id=?1 AND waiting.kind='upload' AND waiting.state='pending' AND waiting.last_error IS NULL
            AND EXISTS(SELECT 1 FROM media_assets a WHERE a.asset_id=waiting.asset_id AND a.asset_state='active')
            AND NOT EXISTS(SELECT 1 FROM backup_operations d WHERE d.space_id=waiting.space_id AND d.asset_id=waiting.asset_id
                AND d.kind='delete' AND d.state NOT IN ('completed','canceled'))
            AND EXISTS(SELECT 1 FROM backup_operations old WHERE old.space_id=waiting.space_id AND old.asset_id=waiting.asset_id
                AND old.kind='upload' AND old.phase<>'cancel_cleaned'
                AND (old.state='canceled' OR (old.state='blocked' AND old.last_error='BACKUP_ASSET_DELETED')))", [space])?;
        Ok(())
    })
}

pub(super) fn finish_upload_cleanup(core: &Core, id: &str) -> Result<()> {
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let changed = tx.execute("UPDATE backup_operations SET phase='cancel_cleaned',state=CASE WHEN phase='superseded' THEN 'completed' ELSE state END,
            last_error=CASE WHEN state='canceled' THEN NULL ELSE last_error END,next_retry_at=NULL,
            payload_json=json_remove(payload_json,'$.cleanup_blocked','$.cleanup_error','$.cleanup_attempt'),updated_at=unixepoch()
            WHERE operation_id=?1 AND (state='canceled' OR (state='blocked' AND last_error='BACKUP_ASSET_DELETED'))", [id])?;
        if changed == 0 { return Ok(()); }
        tx.execute("UPDATE backup_operations SET state='pending',last_error=NULL,next_retry_at=NULL,
            payload_json=json_remove(payload_json,'$.cleanup_dependency'),updated_at=unixepoch()
            WHERE kind='upload' AND state IN ('retry_wait','blocked') AND json_extract(payload_json,'$.cleanup_dependency')=?1
            AND last_error IN ('BACKUP_CLEANUP_PENDING','TELEGRAM_SEND_UNCONFIRMED')
            AND EXISTS(SELECT 1 FROM media_assets a WHERE a.asset_id=backup_operations.asset_id AND a.asset_state='active')
            AND NOT EXISTS(SELECT 1 FROM backup_operations d WHERE d.space_id=backup_operations.space_id
                AND d.asset_id=backup_operations.asset_id AND d.kind='delete' AND d.state NOT IN ('completed','canceled'))", [id])?;
        tx.commit()?;
        Ok(())
    })
}

pub(super) fn defer_upload_cleanup(core: &Core, op: &Operation, reason: &str) -> Result<()> {
    if reason == "TELEGRAM_NETWORK_CHANGED" {
        return core.backup_db(|db| {
            db.execute("UPDATE backup_operations SET next_retry_at=unixepoch()+2 WHERE operation_id=?1
                AND phase<>'cancel_cleaned' AND (state='canceled' OR (state='blocked' AND last_error='BACKUP_ASSET_DELETED'))", [&op.operation_id])?;
            Ok(())
        });
    }
    let attempt = op.payload["cleanup_attempt"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(1);
    let blocked = attempt >= 5
        || matches!(
            reason,
            "BACKUP_SCOPE_MISMATCH"
                | "BACKUP_UPLOAD_NOT_OWNED"
                | "TELEGRAM_AUTH_REQUIRED"
                | "TELEGRAM_ACCOUNT_MISMATCH"
                | "TELEGRAM_CHANNEL_FORBIDDEN"
                | "TELEGRAM_MEDIA_CHANGED"
                | "TELEGRAM_DUPLICATE_OPERATION"
                | "BACKUP_INVALID_RESUME_STATE"
        );
    let delay = reason
        .strip_prefix("TELEGRAM_FLOOD_WAIT:")
        .and_then(|v| v.parse::<u32>().ok())
        .map(i64::from)
        .unwrap_or_else(|| (30_i64 * 2_i64.pow(attempt.min(4) as u32)).min(300));
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let changed = tx.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.cleanup_attempt',?2,'$.cleanup_error',?3,'$.cleanup_blocked',?4),
            next_retry_at=unixepoch()+?5,updated_at=unixepoch()
            WHERE operation_id=?1 AND (state='canceled' OR (state='blocked' AND last_error='BACKUP_ASSET_DELETED'))",
            rusqlite::params![op.operation_id,i64::try_from(attempt)?,reason,blocked,delay])?;
        if changed == 0 { return Ok(()); }
        tx.execute("UPDATE backup_operations SET state=?4,last_error=?5,next_retry_at=unixepoch()+?6,
            payload_json=json_set(payload_json,'$.cleanup_dependency',?1),updated_at=unixepoch()
            WHERE space_id=?2 AND asset_id=?3 AND kind='upload' AND operation_id<>?1
            AND state IN ('pending','retry_wait','blocked')
            AND ((state='pending' AND last_error IS NULL) OR (json_extract(payload_json,'$.cleanup_dependency')=?1
                AND last_error IN ('BACKUP_CLEANUP_PENDING','TELEGRAM_SEND_UNCONFIRMED')))
            AND NOT EXISTS(SELECT 1 FROM backup_operations d WHERE d.space_id=backup_operations.space_id
                AND d.asset_id=backup_operations.asset_id AND d.kind='delete' AND d.state NOT IN ('completed','canceled'))",
            rusqlite::params![op.operation_id,op.space_id,op.asset_id,if blocked {"blocked"} else {"retry_wait"},
                if reason=="TELEGRAM_SEND_UNCONFIRMED" {reason} else {"BACKUP_CLEANUP_PENDING"},delay])?;
        tx.commit()?;
        Ok(())
    })
}

pub(super) fn unfinished_restores(core: &Core, asset: &str) -> Result<Vec<Operation>> {
    select_operations(core, "WHERE asset_id=?1 AND kind='restore' AND state<>'completed'
        ORDER BY created_at DESC,rowid DESC", [asset])
}

fn select_operations(core: &Core, clause: &str, params: impl rusqlite::Params) -> Result<Vec<Operation>> {
    core.backup_db(|db| {
        let mut q=db.prepare(&format!("SELECT operation_id,space_id,asset_id,kind,state,phase,payload_json,attempt,last_error FROM backup_operations {clause}"))?;
        let rows=q.query_map(params,|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get::<_,String>(6)?,r.get(7)?,r.get(8)?)))?;
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

#[cfg(test)]
mod display_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn display_projection_keeps_ui_state_and_leaves_recovery_payload_intact() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(&root.path().join("app.sqlite"),root.path().join("downloads"),"unused".into(),vec![], |_| {}).unwrap();
        let state = json!({"completed_bytes":0,"transfer_started":false,"local_cleanup_done":true,
            "archive_next":"peer","cleanup_dependency":"previous","cleanup_error":"TELEGRAM_SEND_UNCONFIRMED","cleanup_blocked":false});
        let mut payload = state.clone();
        payload["captured"] = json!({"cover":vec![255_u8;256*1024],"text":"worker recovery data"});
        core.backup_db(|db| {
            db.execute("INSERT INTO backup_spaces VALUES('space','library','','','','',0)", [])?;
            db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json,attempt,last_error,created_at)
                VALUES('old','space','upload','blocked','captured',?1,3,'BACKUP_NETWORK_ERROR',1),
                      ('new','space','delete','completed','completed','{}',0,NULL,1)", [payload.to_string()])?;
            Ok(())
        }).unwrap();
        let display = operations_for_display(&core).unwrap();
        assert_eq!(display.iter().map(|op| op.operation_id.as_str()).collect::<Vec<_>>(),vec!["new","old"]);
        assert_eq!(display[0].payload,json!({}));
        assert_eq!(display[1].payload,state);
        assert_eq!(display[1].attempt,3);
        assert_eq!(display[1].last_error.as_deref(),Some("BACKUP_NETWORK_ERROR"));
        assert!(serde_json::to_vec(&display).unwrap().len()<1024);
        assert_eq!(operation(&core,"old").unwrap().payload,payload);
    }
}
