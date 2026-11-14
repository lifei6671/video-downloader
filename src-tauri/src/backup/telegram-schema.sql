CREATE TABLE IF NOT EXISTS telegram_account_bindings (
 purpose TEXT PRIMARY KEY CHECK(purpose IN ('download','backup')),session_ref TEXT NOT NULL);
-- Send evidence only. Scheduling remains owned by backup_operations / SyncEngine.
CREATE TABLE IF NOT EXISTS telegram_media_sends (
    operation_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('video','cover','transcript','library','event')),
    entry_json TEXT NOT NULL CHECK(json_valid(entry_json)),
    PRIMARY KEY(operation_id,kind)
);
CREATE TABLE IF NOT EXISTS telegram_transfer_cooldowns (
    account_id TEXT PRIMARY KEY,
    retry_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_extra_sends (
 operation_id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('database','asset_archive')),
 entry_json TEXT NOT NULL CHECK(json_valid(entry_json)));
CREATE TABLE IF NOT EXISTS telegram_spaces (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 account_id TEXT NOT NULL,channel_id TEXT NOT NULL,
 initializing INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id,channel_id)
);
CREATE TABLE IF NOT EXISTS telegram_observed_prefix (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),cursor_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_space_labels (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id),
 channel_title TEXT NOT NULL
);

-- The channel update position and the message rows it covers are committed together.
CREATE TABLE IF NOT EXISTS telegram_message_map (
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id) ON DELETE CASCADE,
 target_json TEXT NOT NULL, pts INTEGER NOT NULL, high_water INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS telegram_mapped_documents (
 space_id TEXT NOT NULL REFERENCES telegram_message_map(space_id) ON DELETE CASCADE,
 message_id INTEGER NOT NULL, document_json TEXT NOT NULL, revision INTEGER, index_body BLOB
);
CREATE INDEX IF NOT EXISTS telegram_mapped_message ON telegram_mapped_documents(space_id,message_id);
