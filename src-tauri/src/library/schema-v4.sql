CREATE TABLE transcript_timings (
    document_id TEXT PRIMARY KEY REFERENCES asset_documents(id) ON DELETE CASCADE,
    frames INTEGER NOT NULL CHECK(frames > 0),
    vad_revision TEXT NOT NULL,
    operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 128)
);
CREATE TABLE transcript_segments (
    document_id TEXT NOT NULL REFERENCES transcript_timings(document_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK(position >= 0),
    start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
    end_ms INTEGER NOT NULL CHECK(end_ms > start_ms),
    text TEXT NOT NULL CHECK(length(trim(text)) > 0),
    PRIMARY KEY(document_id, position)
);
UPDATE library_schema SET version=4 WHERE id=1;
