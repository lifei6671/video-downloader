CREATE TABLE IF NOT EXISTS backup_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
INSERT OR IGNORE INTO backup_schema VALUES(1,1);
CREATE TABLE IF NOT EXISTS backup_device(id INTEGER PRIMARY KEY CHECK(id=1),device_id TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS backup_spaces(
 space_id TEXT PRIMARY KEY,library_id TEXT NOT NULL,endpoint TEXT NOT NULL,bucket TEXT NOT NULL,prefix TEXT NOT NULL,
 role TEXT NOT NULL CHECK(role IN ('primary','reader')),writer_device_id TEXT NOT NULL,writer_epoch INTEGER NOT NULL CHECK(writer_epoch>0),credential_ref TEXT,enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),UNIQUE(endpoint,bucket,prefix));
CREATE TABLE IF NOT EXISTS backup_current_space(id INTEGER PRIMARY KEY CHECK(id=1),space_id TEXT NOT NULL REFERENCES backup_spaces(space_id));
CREATE TABLE IF NOT EXISTS media_assets(
 asset_id TEXT PRIMARY KEY,task_id TEXT UNIQUE,metadata_json TEXT NOT NULL,original_filename TEXT NOT NULL,
 content_sha256 TEXT,content_bytes INTEGER NOT NULL CHECK(content_bytes>0),asset_state TEXT NOT NULL CHECK(asset_state IN ('active','deleted')));
CREATE TABLE IF NOT EXISTS asset_local_files(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),path TEXT,bytes INTEGER NOT NULL CHECK(bytes>=0),sha256 TEXT,source_json TEXT,verified_at INTEGER,
 local_state TEXT NOT NULL CHECK(local_state IN ('present','missing','restoring','invalid')));
CREATE TABLE IF NOT EXISTS asset_remote_objects(
 space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_key TEXT NOT NULL,record_etag TEXT,record_revision INTEGER NOT NULL DEFAULT 0,
 media_key TEXT,cover_key TEXT,transcript_key TEXT,remote_state TEXT NOT NULL CHECK(remote_state IN ('not_backed_up','pending','uploading','backed_up','pending_delete','deleted','error')),PRIMARY KEY(space_id,asset_id));
CREATE TABLE IF NOT EXISTS asset_transcripts(
 asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),content_sha256 TEXT,text TEXT NOT NULL,model_revision TEXT NOT NULL,model_name TEXT,pipeline_revision TEXT NOT NULL,completed_at INTEGER NOT NULL,source_json TEXT);
CREATE TABLE IF NOT EXISTS asset_remote_records(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),record_json TEXT NOT NULL,PRIMARY KEY(space_id,asset_id));
CREATE TABLE IF NOT EXISTS backup_asset_versions(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),published_signature TEXT NOT NULL,PRIMARY KEY(space_id,asset_id));
/* Asset transcript body is device-local; the portable object excludes source_json. */
CREATE TABLE IF NOT EXISTS asset_covers(asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id),body BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS backup_operations(
 operation_id TEXT PRIMARY KEY,space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT REFERENCES media_assets(asset_id),kind TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','running','retry_wait','blocked','completed','canceled')),phase TEXT NOT NULL,payload_json TEXT NOT NULL,
 attempt INTEGER NOT NULL DEFAULT 0,next_retry_at INTEGER,last_error TEXT,created_at INTEGER NOT NULL DEFAULT(unixepoch()),updated_at INTEGER NOT NULL DEFAULT(unixepoch()));
CREATE UNIQUE INDEX IF NOT EXISTS backup_operation_active ON backup_operations(space_id,asset_id,kind) WHERE state NOT IN ('completed','canceled');

CREATE TABLE IF NOT EXISTS backup_credential_cleanup(reference TEXT PRIMARY KEY);

CREATE TABLE IF NOT EXISTS backup_owned_objects(space_id TEXT NOT NULL REFERENCES backup_spaces(space_id),asset_id TEXT NOT NULL REFERENCES media_assets(asset_id),object_key TEXT NOT NULL,PRIMARY KEY(space_id,asset_id,object_key));

CREATE TABLE IF NOT EXISTS asset_hidden_history(asset_id TEXT PRIMARY KEY REFERENCES media_assets(asset_id));
