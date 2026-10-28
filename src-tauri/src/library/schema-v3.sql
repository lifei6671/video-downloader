CREATE TABLE asset_visits (
    asset_id TEXT PRIMARY KEY REFERENCES library_assets(id) ON DELETE CASCADE,
    visited_at INTEGER NOT NULL CHECK(visited_at>=0),
    visit_order INTEGER NOT NULL
);
CREATE INDEX asset_visits_order ON asset_visits(visit_order DESC,asset_id DESC);
UPDATE library_schema SET version=3 WHERE id=1;
