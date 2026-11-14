CREATE TABLE IF NOT EXISTS backup_databases(
 space_id TEXT PRIMARY KEY REFERENCES backup_spaces(space_id) ON DELETE CASCADE,
 key_ref TEXT NOT NULL,enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
 generated_password INTEGER NOT NULL DEFAULT 0 CHECK(generated_password IN (0,1)),
 interval_hours INTEGER NOT NULL DEFAULT 24 CHECK(interval_hours IN (1,3,6,24)),
 last_success_at INTEGER,last_error TEXT,next_attempt_at INTEGER NOT NULL DEFAULT 0,
 last_telegram_message_id INTEGER CHECK(last_telegram_message_id > 0),
 last_speed_bps INTEGER CHECK(last_speed_bps >= 0),
 last_canceled INTEGER NOT NULL DEFAULT 0 CHECK(last_canceled IN (0,1)));
