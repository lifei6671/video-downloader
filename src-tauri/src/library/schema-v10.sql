CREATE TABLE library_ideas (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    viewpoint TEXT NOT NULL,
    body TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
-- References retain their source identity and names when a source disappears.
CREATE TABLE idea_references (
    idea_id TEXT NOT NULL REFERENCES library_ideas(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('video','clip','topic')),
    asset_id TEXT,
    resource_id TEXT NOT NULL,
    label TEXT NOT NULL,
    asset_title TEXT,
    position INTEGER NOT NULL CHECK(position >= 0),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(idea_id,kind,resource_id)
);
CREATE INDEX idea_references_order ON idea_references(idea_id,position);
UPDATE library_schema SET version=10 WHERE id=1;
