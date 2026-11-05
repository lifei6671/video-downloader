-- Small, transactionally maintained UI projection. Recovery snapshots remain
-- in backup_operations; polling and byte progress never read/rewrite them.
CREATE TABLE IF NOT EXISTS backup_operation_display(
 operation_id TEXT PRIMARY KEY REFERENCES backup_operations(operation_id) ON DELETE CASCADE,
 space_id TEXT NOT NULL,asset_id TEXT,kind TEXT NOT NULL,state TEXT NOT NULL,phase TEXT NOT NULL,
 summary_json TEXT NOT NULL,attempt INTEGER NOT NULL,last_error TEXT,created_at INTEGER NOT NULL,
 sequence INTEGER NOT NULL,progress_bytes INTEGER,progress_started INTEGER);
CREATE INDEX IF NOT EXISTS backup_operation_display_space ON backup_operation_display(space_id,created_at DESC,sequence DESC);
CREATE INDEX IF NOT EXISTS backup_restore_lookup ON backup_operations(space_id,asset_id,kind,state);
INSERT OR IGNORE INTO backup_operation_display(operation_id,space_id,asset_id,kind,state,phase,summary_json,attempt,last_error,created_at,sequence)
 SELECT operation_id,space_id,asset_id,kind,state,phase,
 json_extract(payload_json,'$.completed_bytes','$.transfer_started','$.local_cleanup_done','$.archive_next','$.cleanup_dependency','$.cleanup_error','$.cleanup_blocked'),attempt,last_error,created_at,rowid
 FROM backup_operations WHERE operation_id NOT IN (SELECT operation_id FROM backup_operation_display);
CREATE TRIGGER IF NOT EXISTS backup_display_insert AFTER INSERT ON backup_operations BEGIN
 INSERT INTO backup_operation_display(operation_id,space_id,asset_id,kind,state,phase,summary_json,attempt,last_error,created_at,sequence)
 VALUES(NEW.operation_id,NEW.space_id,NEW.asset_id,NEW.kind,NEW.state,NEW.phase,
 json_extract(NEW.payload_json,'$.completed_bytes','$.transfer_started','$.local_cleanup_done','$.archive_next','$.cleanup_dependency','$.cleanup_error','$.cleanup_blocked'),NEW.attempt,NEW.last_error,NEW.created_at,NEW.rowid);
END;
CREATE TRIGGER IF NOT EXISTS backup_display_state AFTER UPDATE OF space_id,asset_id,kind,state,phase,attempt,last_error,created_at ON backup_operations BEGIN
 UPDATE backup_operation_display SET space_id=NEW.space_id,asset_id=NEW.asset_id,kind=NEW.kind,state=NEW.state,phase=NEW.phase,attempt=NEW.attempt,last_error=NEW.last_error,created_at=NEW.created_at WHERE operation_id=NEW.operation_id;
END;
CREATE TRIGGER IF NOT EXISTS backup_display_payload AFTER UPDATE OF payload_json ON backup_operations BEGIN
 UPDATE backup_operation_display SET summary_json=json_extract(NEW.payload_json,'$.completed_bytes','$.transfer_started','$.local_cleanup_done','$.archive_next','$.cleanup_dependency','$.cleanup_error','$.cleanup_blocked') WHERE operation_id=NEW.operation_id;
END;
CREATE TRIGGER IF NOT EXISTS backup_display_delete AFTER DELETE ON backup_operations BEGIN
 DELETE FROM backup_operation_display WHERE operation_id=OLD.operation_id;
END;
