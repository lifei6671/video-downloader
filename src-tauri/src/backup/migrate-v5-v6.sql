-- These immutable local archives preserve every removed field and original task state.
-- They are never read as active configuration or scheduled work.
CREATE TABLE backup_legacy_v5_spaces AS SELECT * FROM backup_spaces;
CREATE TABLE backup_legacy_v5_remote_objects AS SELECT * FROM asset_remote_objects;
CREATE TABLE backup_legacy_v5_provider_config AS SELECT * FROM backup_provider_config;
CREATE TABLE backup_legacy_v5_operations AS SELECT * FROM backup_operations;
CREATE TABLE backup_legacy_v5_sync_preference AS SELECT * FROM backup_sync_preference;

ALTER TABLE backup_spaces DROP COLUMN role;
ALTER TABLE backup_spaces DROP COLUMN writer_device_id;
ALTER TABLE backup_spaces DROP COLUMN writer_epoch;
ALTER TABLE asset_remote_objects DROP COLUMN record_key;
ALTER TABLE asset_remote_objects DROP COLUMN record_etag;
ALTER TABLE asset_remote_objects DROP COLUMN media_key;
ALTER TABLE asset_remote_objects DROP COLUMN cover_key;
ALTER TABLE asset_remote_objects DROP COLUMN transcript_key;

-- No table references provider_config; its rows reference the unchanged space IDs.
CREATE TABLE backup_provider_config_v6(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 provider_type TEXT NOT NULL CHECK(provider_type IN ('s3','telegram')),
 profile TEXT NOT NULL CHECK(profile IN ('r2','s3','telegram')),
 region TEXT NOT NULL DEFAULT 'auto',path_style INTEGER NOT NULL DEFAULT 1,
 capabilities_json TEXT,protocol INTEGER NOT NULL DEFAULT 2 CHECK(protocol=2));
INSERT INTO backup_provider_config_v6
 SELECT space_id,provider_type,profile,region,path_style,capabilities_json,protocol FROM backup_provider_config;
DROP TABLE backup_provider_config;
ALTER TABLE backup_provider_config_v6 RENAME TO backup_provider_config;
