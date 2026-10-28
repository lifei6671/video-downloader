CREATE TABLE asset_keyframes (
    id TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES asset_files(id) ON DELETE RESTRICT,
    clip_id TEXT REFERENCES asset_clips(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    source_json TEXT NOT NULL,
    body BLOB NOT NULL CHECK(length(body) > 0)
);
CREATE INDEX asset_keyframes_owner ON asset_keyframes(asset_id, clip_id);
UPDATE library_schema SET version=6 WHERE id=1;
