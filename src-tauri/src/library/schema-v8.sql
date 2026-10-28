CREATE TABLE note_comparisons (
    note_id TEXT PRIMARY KEY REFERENCES asset_notes(id) ON DELETE CASCADE,
    data TEXT NOT NULL
);
UPDATE library_schema SET version=8 WHERE id=1;
