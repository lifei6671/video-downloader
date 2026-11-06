-- Final development schema at 844f31f, before consolidating the release migration.
CREATE TABLE IF NOT EXISTS backup_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
INSERT OR IGNORE INTO backup_schema VALUES(1,9);
CREATE TABLE IF NOT EXISTS backup_device(id INTEGER PRIMARY KEY CHECK(id=1),device_id TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS backup_spaces(
 space_id TEXT PRIMARY KEY,library_id TEXT NOT NULL,endpoint TEXT NOT NULL,bucket TEXT NOT NULL,prefix TEXT NOT NULL,
 credential_ref TEXT,enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));
CREATE UNIQUE INDEX IF NOT EXISTS backup_s3_target ON backup_spaces(endpoint,bucket,prefix) WHERE endpoint<>'';
CREATE TABLE IF NOT EXISTS backup_pull_runs(
 run_id INTEGER PRIMARY KEY AUTOINCREMENT,space_id TEXT NOT NULL REFERENCES backup_spaces(space_id) ON DELETE CASCADE,
 trigger TEXT NOT NULL,foreground_delete_busy INTEGER NOT NULL,started_at_ms INTEGER NOT NULL,ended_at_ms INTEGER,outcome TEXT);
CREATE TABLE IF NOT EXISTS backup_current_space(id INTEGER PRIMARY KEY CHECK(id=1),space_id TEXT NOT NULL REFERENCES backup_spaces(space_id));
CREATE TABLE IF NOT EXISTS media_assets(
 asset_id TEXT PRIMARY KEY,task_id TEXT UNIQUE,metadata_json TEXT NOT NULL,original_filename TEXT NOT NULL,
 content_sha256 TEXT,content_bytes INTEGER NOT NULL CHECK(content_bytes>0),asset_state TEXT NOT NULL CHECK(asset_state IN ('active','deleted')));
CREATE TABLE IF NOT EXISTS asset_local_files(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),path TEXT,bytes INTEGER NOT NULL CHECK(bytes>=0),sha256 TEXT,source_json TEXT,verified_at INTEGER,
 local_state TEXT NOT NULL CHECK(local_state IN ('present','missing','restoring','invalid')));
CREATE TABLE IF NOT EXISTS asset_remote_objects(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_revision INTEGER NOT NULL DEFAULT 0,
 remote_state TEXT NOT NULL CHECK(remote_state IN ('not_backed_up','pending','uploading','backed_up','pending_delete','deleted','error')),PRIMARY KEY(space_id,asset_id));
CREATE TABLE IF NOT EXISTS asset_transcripts(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),content_sha256 TEXT,text TEXT NOT NULL,model_revision TEXT NOT NULL,model_name TEXT,pipeline_revision TEXT NOT NULL,completed_at INTEGER NOT NULL,source_json TEXT);
CREATE TABLE IF NOT EXISTS asset_remote_records(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_json TEXT NOT NULL,PRIMARY KEY(space_id,asset_id));
/* Asset transcript body is device-local; the portable object excludes source_json. */
CREATE TABLE IF NOT EXISTS asset_covers(asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),body BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS backup_operations(
 operation_id TEXT PRIMARY KEY,space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT REFERENCES media_assets(asset_id),kind TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','running','retry_wait','blocked','completed','canceled')),phase TEXT NOT NULL,payload_json TEXT NOT NULL,
 attempt INTEGER NOT NULL DEFAULT 0,next_retry_at INTEGER,last_error TEXT,created_at INTEGER NOT NULL DEFAULT(unixepoch()),updated_at INTEGER NOT NULL DEFAULT(unixepoch()));
CREATE UNIQUE INDEX IF NOT EXISTS backup_operation_active ON backup_operations(space_id,asset_id,kind) WHERE state NOT IN ('completed','canceled');

CREATE TABLE IF NOT EXISTS backup_credential_cleanup(reference TEXT PRIMARY KEY);


CREATE TABLE IF NOT EXISTS asset_hidden_history(asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id));
CREATE TABLE IF NOT EXISTS asset_deletion_hidden_history(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),PRIMARY KEY(space_id,asset_id));

CREATE TABLE IF NOT EXISTS backup_provider_config(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 provider_type TEXT NOT NULL CHECK(provider_type IN ('s3','telegram')),
 profile TEXT NOT NULL CHECK(profile IN ('r2','s3','telegram')),
 region TEXT NOT NULL DEFAULT 'auto',path_style INTEGER NOT NULL DEFAULT 1,
 capabilities_json TEXT,protocol INTEGER NOT NULL DEFAULT 2 CHECK(protocol=2));
CREATE TABLE IF NOT EXISTS backup_space_names(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id) ON DELETE CASCADE,
 display_name TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS backup_sync_preference(
 id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS backup_removed_spaces(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id));
CREATE TABLE IF NOT EXISTS backup_space_assets(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),
 remote_asset_id TEXT NOT NULL,local_asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),
 remote_deleted INTEGER NOT NULL DEFAULT 0,cloud_visible INTEGER NOT NULL DEFAULT 1,
 local_revision INTEGER NOT NULL DEFAULT 0,ack_revision INTEGER NOT NULL DEFAULT 0,
 pending_json TEXT NOT NULL DEFAULT '{}',baseline_json TEXT,version_json TEXT,
 PRIMARY KEY(space_id,remote_asset_id),UNIQUE(space_id,local_asset_id));
CREATE TABLE IF NOT EXISTS backup_upload_proofs(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,device_id TEXT NOT NULL,
 operation_id TEXT NOT NULL,media_sha256 TEXT NOT NULL,
 PRIMARY KEY(space_id,remote_asset_id,device_id),
 FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));
CREATE TABLE IF NOT EXISTS backup_conflicts(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,conflict_id TEXT NOT NULL,
 value_json TEXT NOT NULL,PRIMARY KEY(space_id,remote_asset_id,conflict_id),
 FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));
CREATE TABLE IF NOT EXISTS backup_sync_checkpoints(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),checkpoint_json TEXT,
 last_success_at INTEGER,last_error TEXT);
CREATE TABLE IF NOT EXISTS backup_window_sync(
 space_id TEXT PRIMARY KEY REFERENCES backup_sync_checkpoints(space_id) ON DELETE CASCADE,
 last_success_ms INTEGER NOT NULL,last_skipped_ms INTEGER);
CREATE TABLE IF NOT EXISTS backup_probe_cleanup(
 operation_id TEXT PRIMARY KEY,target_json TEXT NOT NULL,credential_ref TEXT NOT NULL,last_error TEXT);
CREATE TABLE IF NOT EXISTS backup_tombstones(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,record_json TEXT NOT NULL,version_json TEXT NOT NULL,
 PRIMARY KEY(space_id,remote_asset_id),FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));

CREATE TABLE IF NOT EXISTS backup_record_retention(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 enabled INTEGER NOT NULL DEFAULT 0,
 days INTEGER NOT NULL DEFAULT 30 CHECK(days BETWEEN 1 AND 3650),
 next_check_at INTEGER NOT NULL DEFAULT 0,last_error TEXT,delete_verified INTEGER NOT NULL DEFAULT 0);

-- Reverse lookups used by history registration and local asset operations.
CREATE INDEX IF NOT EXISTS backup_space_assets_local ON backup_space_assets(local_asset_id);
CREATE INDEX IF NOT EXISTS backup_operations_asset ON backup_operations(asset_id);
CREATE TABLE IF NOT EXISTS backup_operation_captures (
 operation_id TEXT PRIMARY KEY REFERENCES backup_operations(operation_id) ON DELETE CASCADE,
 capture_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_import_receipts (
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,
 remote_json TEXT NOT NULL,version_json TEXT NOT NULL,local_revision INTEGER NOT NULL,pending_json TEXT NOT NULL,
 PRIMARY KEY(space_id,remote_asset_id),
 FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id) ON DELETE CASCADE
);

-- A local projection is shared by memberships, so another space can invalidate it.
CREATE TRIGGER IF NOT EXISTS backup_import_projection_changed AFTER UPDATE ON media_assets BEGIN
 DELETE FROM backup_import_receipts WHERE (space_id,remote_asset_id) IN
 (SELECT space_id,remote_asset_id FROM backup_space_assets WHERE local_asset_id=NEW.asset_id);
END;

-- Send evidence only. Scheduling remains owned by backup_operations / SyncEngine.
CREATE TABLE IF NOT EXISTS telegram_media_sends (
    operation_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('video','cover','transcript','library','event')),
    entry_json TEXT NOT NULL CHECK(json_valid(entry_json)),
    PRIMARY KEY(operation_id,kind)
);
CREATE TABLE IF NOT EXISTS telegram_transfer_cooldowns (
    account_id TEXT PRIMARY KEY,
    retry_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_spaces (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 account_id TEXT NOT NULL,channel_id TEXT NOT NULL,
 initializing INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id,channel_id)
);
CREATE TABLE IF NOT EXISTS telegram_observed_prefix (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),cursor_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_space_labels (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 channel_title TEXT NOT NULL
);

-- The channel update position and the message rows it covers are committed together.
CREATE TABLE IF NOT EXISTS telegram_message_map (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id) ON DELETE CASCADE,
 target_json TEXT NOT NULL, pts INTEGER NOT NULL, high_water INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_mapped_documents (
 space_id TEXT NOT NULL REFERENCES telegram_message_map(space_id) ON DELETE CASCADE,
 message_id INTEGER NOT NULL, document_json TEXT NOT NULL, revision INTEGER, index_body BLOB
);
CREATE INDEX IF NOT EXISTS telegram_mapped_message ON telegram_mapped_documents(space_id,message_id);

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

CREATE TABLE IF NOT EXISTS backup_history_generation(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL);
INSERT OR IGNORE INTO backup_history_generation VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS backup_history_insert AFTER INSERT ON download_tasks BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;
CREATE TRIGGER IF NOT EXISTS backup_history_delete AFTER DELETE ON download_tasks BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;
CREATE TRIGGER IF NOT EXISTS backup_history_update AFTER UPDATE OF snapshot ON download_tasks
WHEN json_extract(OLD.snapshot,'$.status') IS NOT json_extract(NEW.snapshot,'$.status')
 OR json_extract(OLD.snapshot,'$.path') IS NOT json_extract(NEW.snapshot,'$.path')
 OR json_extract(OLD.snapshot,'$.asset_id') IS NOT json_extract(NEW.snapshot,'$.asset_id')
BEGIN UPDATE backup_history_generation SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS backup_history_detach AFTER DELETE ON backup_space_assets BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;

CREATE TABLE IF NOT EXISTS telegram_account_bindings(purpose TEXT PRIMARY KEY CHECK(purpose IN ('download','backup')),session_ref TEXT NOT NULL);
