CREATE TABLE asset_transcriptions (
    file_id TEXT PRIMARY KEY REFERENCES asset_files(id) ON DELETE CASCADE,
    snapshot TEXT NOT NULL
);
UPDATE library_schema SET version=2 WHERE id=1;
