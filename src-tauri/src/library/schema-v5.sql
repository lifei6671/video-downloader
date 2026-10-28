CREATE TABLE asset_clips (
    id TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES asset_files(id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK(length(trim(title)) > 0),
    reason TEXT NOT NULL,
    start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
    end_ms INTEGER NOT NULL CHECK(end_ms > start_ms),
    source_json TEXT NOT NULL,
    document_id TEXT REFERENCES asset_documents(id) ON DELETE RESTRICT,
    document_version INTEGER,
    quote TEXT,
    created_at INTEGER NOT NULL,
    CHECK((document_id IS NULL AND document_version IS NULL AND quote IS NULL) OR
          (document_id IS NOT NULL AND document_version IS NOT NULL AND document_version > 0 AND quote IS NOT NULL AND length(trim(quote)) > 0))
);
CREATE INDEX asset_clips_order ON asset_clips(asset_id, created_at DESC, id DESC);
UPDATE library_schema SET version=5 WHERE id=1;
