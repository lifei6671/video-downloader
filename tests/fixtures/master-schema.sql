-- master 2d32959a7d8367aff57c6a1d962ea048afbc4ea0; desktop initialization SQL.
PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS commit_intents(task_id TEXT PRIMARY KEY,source TEXT NOT NULL,
                target TEXT NOT NULL,size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);
ALTER TABLE commit_intents ADD COLUMN manifest TEXT;
CREATE TABLE IF NOT EXISTS parse_cache (
           cache_key TEXT PRIMARY KEY, platform TEXT NOT NULL, video_id TEXT NOT NULL,
           resolver TEXT NOT NULL, kind TEXT NOT NULL, metadata TEXT NOT NULL,
           saved_at INTEGER NOT NULL CHECK(saved_at>=0), revision TEXT NOT NULL, cover BLOB);
         CREATE TABLE IF NOT EXISTS parse_cache_images (
           cache_key TEXT NOT NULL, image_index INTEGER NOT NULL CHECK(image_index BETWEEN 0 AND 63),
           body BLOB NOT NULL CHECK(length(body) BETWEEN 1 AND 33554432),
           PRIMARY KEY(cache_key,image_index));
         CREATE TABLE IF NOT EXISTS parse_cache_aliases (
           url TEXT PRIMARY KEY, platform TEXT NOT NULL, video_id TEXT NOT NULL);
ALTER TABLE parse_cache ADD COLUMN author_avatar BLOB;
CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS platform_settings (
           platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL, proxy TEXT,
           cookie TEXT NOT NULL, credential_ref TEXT);
         CREATE TABLE IF NOT EXISTS tdl_settings (id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL, path TEXT);
ALTER TABLE platform_settings ADD COLUMN http_chunk_size INTEGER;
ALTER TABLE tdl_settings ADD COLUMN session_directory TEXT;
ALTER TABLE app_settings ADD COLUMN max_downloads INTEGER NOT NULL DEFAULT 2 CHECK(max_downloads BETWEEN 1 AND 3);
ALTER TABLE app_settings ADD COLUMN filename_template TEXT NOT NULL DEFAULT '{platform}/{user_name}-{content}';
CREATE TABLE IF NOT EXISTS video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
