CREATE TABLE research_topics (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
-- Source IDs intentionally have no cascading foreign key: missing references stay visible.
CREATE TABLE research_topic_members (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL REFERENCES research_topics(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('video','clip','note')),
    asset_id TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    label TEXT NOT NULL,
    asset_title TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    created_at INTEGER NOT NULL,
    UNIQUE(topic_id,kind,resource_id)
);
CREATE INDEX research_topic_members_order ON research_topic_members(topic_id,position,id);
UPDATE library_schema SET version=9 WHERE id=1;
