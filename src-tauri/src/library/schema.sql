CREATE TABLE library_schema (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    version INTEGER NOT NULL
);
INSERT INTO library_schema VALUES(1, 1);

CREATE TABLE library_assets (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    author TEXT,
    platform TEXT,
    video_id TEXT,
    canonical_url TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived_at INTEGER,
    CHECK((platform IS NULL) = (video_id IS NULL)),
    UNIQUE(platform, video_id)
);
CREATE INDEX library_assets_order ON library_assets(created_at DESC, id DESC);

CREATE TABLE asset_files (
    id TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    size INTEGER NOT NULL CHECK(size >= 0),
    mtime_ns TEXT,
    duration_ms INTEGER,
    width INTEGER,
    height INTEGER,
    UNIQUE(asset_id, path)
);
CREATE TABLE asset_task_links (
    task_id TEXT PRIMARY KEY REFERENCES download_tasks(id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    file_id TEXT REFERENCES asset_files(id) ON DELETE SET NULL
);
CREATE INDEX asset_task_links_asset ON asset_task_links(asset_id);
CREATE TABLE asset_resources (
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind = 'cover'),
    body BLOB NOT NULL,
    PRIMARY KEY(asset_id, kind)
);
CREATE TABLE library_categories (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES library_categories(id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK(length(trim(name)) > 0),
    position INTEGER NOT NULL DEFAULT 0,
    CHECK(id != parent_id)
);
CREATE UNIQUE INDEX library_categories_sibling_name ON library_categories(COALESCE(parent_id, ''), name);
CREATE TABLE asset_categories (
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    category_id TEXT NOT NULL REFERENCES library_categories(id) ON DELETE CASCADE,
    PRIMARY KEY(asset_id, category_id)
);
CREATE INDEX asset_categories_category ON asset_categories(category_id, asset_id);
CREATE TABLE library_tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE CHECK(length(trim(name)) > 0)
);
CREATE TABLE asset_tags (
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES library_tags(id) ON DELETE CASCADE,
    PRIMARY KEY(asset_id, tag_id)
);
CREATE INDEX asset_tags_tag ON asset_tags(tag_id, asset_id);
CREATE TABLE asset_documents (
    id TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    file_id TEXT REFERENCES asset_files(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('recognized', 'edited')),
    text TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version > 0),
    revision INTEGER NOT NULL CHECK(revision > 0),
    source_document_id TEXT REFERENCES asset_documents(id) ON DELETE RESTRICT,
    recognition_key TEXT UNIQUE,
    source_json TEXT,
    model_revision TEXT,
    model_name TEXT,
    pipeline_revision TEXT,
    backend TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(asset_id, kind, version)
);
CREATE INDEX asset_documents_asset ON asset_documents(asset_id, kind, version DESC);
CREATE TABLE asset_notes (
    id TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    origin TEXT NOT NULL CHECK(origin IN ('human', 'ai')),
    model_name TEXT,
    citations_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX asset_notes_asset ON asset_notes(asset_id, created_at, id);
