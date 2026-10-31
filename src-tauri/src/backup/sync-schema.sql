CREATE TABLE IF NOT EXISTS backup_provider_config(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 provider_type TEXT NOT NULL CHECK(provider_type IN ('s3','telegram')),
 profile TEXT NOT NULL CHECK(profile IN ('r2','s3','telegram')),
 region TEXT NOT NULL DEFAULT 'auto',path_style INTEGER NOT NULL DEFAULT 1,
 capabilities_json TEXT,protocol INTEGER NOT NULL DEFAULT 1,
 migration_json TEXT);
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
