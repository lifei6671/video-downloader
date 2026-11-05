-- Legacy v5 table definitions only. No user records or credentials.
CREATE TABLE backup_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
CREATE TABLE backup_device(id INTEGER PRIMARY KEY CHECK(id=1),device_id TEXT NOT NULL UNIQUE);
CREATE TABLE "backup_spaces"(
            space_id TEXT PRIMARY KEY,library_id TEXT NOT NULL,endpoint TEXT NOT NULL,bucket TEXT NOT NULL,prefix TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('primary','reader','member')),writer_device_id TEXT NOT NULL,writer_epoch INTEGER NOT NULL CHECK(writer_epoch>0),credential_ref TEXT,enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));
CREATE TABLE media_assets(
 asset_id TEXT PRIMARY KEY,task_id TEXT UNIQUE,metadata_json TEXT NOT NULL,original_filename TEXT NOT NULL,
 content_sha256 TEXT,content_bytes INTEGER NOT NULL CHECK(content_bytes>0),asset_state TEXT NOT NULL CHECK(asset_state IN ('active','deleted')));
CREATE TABLE asset_local_files(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),path TEXT,bytes INTEGER NOT NULL CHECK(bytes>=0),sha256 TEXT,source_json TEXT,verified_at INTEGER,
 local_state TEXT NOT NULL CHECK(local_state IN ('present','missing','restoring','invalid')));
CREATE TABLE asset_remote_objects(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_key TEXT NOT NULL,record_etag TEXT,record_revision INTEGER NOT NULL DEFAULT 0,
 media_key TEXT,cover_key TEXT,transcript_key TEXT,remote_state TEXT NOT NULL CHECK(remote_state IN ('not_backed_up','pending','uploading','backed_up','pending_delete','deleted','error')),PRIMARY KEY(space_id,asset_id));
CREATE TABLE asset_remote_records(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_json TEXT NOT NULL,PRIMARY KEY(space_id,asset_id));
CREATE TABLE asset_transcripts(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),content_sha256 TEXT,text TEXT NOT NULL,model_revision TEXT NOT NULL,model_name TEXT,pipeline_revision TEXT NOT NULL,completed_at INTEGER NOT NULL,source_json TEXT);
CREATE TABLE asset_covers(asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),body BLOB NOT NULL);
CREATE TABLE backup_operations(
 operation_id TEXT PRIMARY KEY,space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT REFERENCES media_assets(asset_id),kind TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','running','retry_wait','blocked','completed','canceled')),phase TEXT NOT NULL,payload_json TEXT NOT NULL,
 attempt INTEGER NOT NULL DEFAULT 0,next_retry_at INTEGER,last_error TEXT,created_at INTEGER NOT NULL DEFAULT(unixepoch()),updated_at INTEGER NOT NULL DEFAULT(unixepoch()));
CREATE TABLE backup_provider_config(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 provider_type TEXT NOT NULL CHECK(provider_type IN ('s3','telegram')),
 profile TEXT NOT NULL CHECK(profile IN ('r2','s3','telegram')),
 region TEXT NOT NULL DEFAULT 'auto',path_style INTEGER NOT NULL DEFAULT 1,
 capabilities_json TEXT,protocol INTEGER NOT NULL DEFAULT 1,
 migration_json TEXT);
CREATE TABLE backup_sync_preference(
 id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL);
CREATE TABLE backup_owned_objects(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),object_key TEXT NOT NULL,PRIMARY KEY(space_id,asset_id,object_key));
CREATE TABLE backup_asset_versions(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),published_signature TEXT NOT NULL,PRIMARY KEY(space_id,asset_id));
CREATE TABLE telegram_media_sends (
    operation_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('video','cover','transcript','library','event')),
    entry_json TEXT NOT NULL CHECK(json_valid(entry_json)),
    PRIMARY KEY(operation_id,kind)
);
CREATE TABLE telegram_spaces (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 account_id TEXT NOT NULL,channel_id TEXT NOT NULL,
 initializing INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id,channel_id)
);
CREATE TABLE backup_sync_checkpoints(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),checkpoint_json TEXT,
 last_success_at INTEGER,last_error TEXT);
CREATE TABLE backup_space_assets(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),
 remote_asset_id TEXT NOT NULL,local_asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),
 remote_deleted INTEGER NOT NULL DEFAULT 0,cloud_visible INTEGER NOT NULL DEFAULT 1,
 local_revision INTEGER NOT NULL DEFAULT 0,ack_revision INTEGER NOT NULL DEFAULT 0,
 pending_json TEXT NOT NULL DEFAULT '{}',baseline_json TEXT,version_json TEXT,
 PRIMARY KEY(space_id,remote_asset_id),UNIQUE(space_id,local_asset_id));
CREATE TABLE backup_upload_proofs(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,device_id TEXT NOT NULL,
 operation_id TEXT NOT NULL,media_sha256 TEXT NOT NULL,
 PRIMARY KEY(space_id,remote_asset_id,device_id),
 FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));
CREATE TABLE backup_tombstones(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,record_json TEXT NOT NULL,version_json TEXT NOT NULL,
 PRIMARY KEY(space_id,remote_asset_id),FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));
CREATE TABLE backup_conflicts(
 space_id TEXT NOT NULL,remote_asset_id TEXT NOT NULL,conflict_id TEXT NOT NULL,
 value_json TEXT NOT NULL,PRIMARY KEY(space_id,remote_asset_id,conflict_id),
 FOREIGN KEY(space_id,remote_asset_id) REFERENCES backup_space_assets(space_id,remote_asset_id));
CREATE TABLE backup_probe_cleanup(
 operation_id TEXT PRIMARY KEY,target_json TEXT NOT NULL,credential_ref TEXT NOT NULL,last_error TEXT);
CREATE TABLE backup_current_space(id INTEGER PRIMARY KEY CHECK(id=1),space_id TEXT NOT NULL REFERENCES backup_spaces(space_id));
INSERT INTO backup_schema VALUES(1,5);
