CREATE TABLE note_analyses (
    note_id TEXT PRIMARY KEY REFERENCES asset_notes(id) ON DELETE CASCADE,
    data TEXT NOT NULL CHECK(json_valid(data))
);
UPDATE library_schema SET version=7 WHERE id=1;
