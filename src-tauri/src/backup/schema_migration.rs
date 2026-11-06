//! Local-only migrations. Portable records and send evidence are never rewritten here.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::{fs::OpenOptions, path::Path};

const VERSION: i64 = 9;
const SPACES: &[&str] = &[
    "space_id",
    "library_id",
    "endpoint",
    "bucket",
    "prefix",
    "credential_ref",
    "enabled",
];
const OBJECTS: &[&str] = &["space_id", "asset_id", "record_revision", "remote_state"];
const CONFIG: &[&str] = &[
    "space_id",
    "provider_type",
    "profile",
    "region",
    "path_style",
    "capabilities_json",
    "protocol",
];

pub(crate) struct Prepared {
    version: Option<i64>,
}

impl Prepared {
    pub(crate) fn is_upgrade(&self) -> bool {
        matches!(self.version, Some(1 | 5 | 6 | 7 | 8))
    }
}

fn version(db: &Connection) -> Result<Option<i64>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backup_schema' COLLATE NOCASE)",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        let partial: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name COLLATE NOCASE IN ('backup_spaces','media_assets'))",
            [], |row| row.get(0),
        )?;
        ensure!(!partial, "BACKUP_MIGRATION_INVALID_STRUCTURE");
        return Ok(None);
    }
    Ok(Some(
        db.query_row("SELECT version FROM backup_schema WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?
        .context("BACKUP_MIGRATION_INVALID_STRUCTURE")?,
    ))
}

fn columns(db: &Connection, table: &str, expected: &[&str]) -> Result<()> {
    // All table names and expected columns are compile-time migration definitions.
    let actual = db
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(actual == expected, "BACKUP_MIGRATION_INVALID_STRUCTURE");
    Ok(())
}

pub(crate) fn prepare(db: &Connection) -> Result<Prepared> {
    ensure!(db.is_autocommit(), "BACKUP_SCHEMA_REQUIRES_AUTOCOMMIT");
    let old = version(db)?;
    match old {
        None => return Ok(Prepared { version: old }),
        Some(1 | 6 | 7 | 8 | VERSION) => {
            // v1 was reused by the current-format rewrite. Do not mistake an older
            // unrelated v1 layout for that format merely because its number matches.
            columns(db, "backup_spaces", SPACES)?;
            columns(db, "asset_remote_objects", OBJECTS)?;
            columns(db, "backup_provider_config", CONFIG)?;
            if old == Some(VERSION) {
                return Ok(Prepared { version: old });
            }
        }
        Some(5) => {
            columns(
                db,
                "backup_spaces",
                &[
                    "space_id",
                    "library_id",
                    "endpoint",
                    "bucket",
                    "prefix",
                    "role",
                    "writer_device_id",
                    "writer_epoch",
                    "credential_ref",
                    "enabled",
                ],
            )?;
            columns(
                db,
                "asset_remote_objects",
                &[
                    "space_id",
                    "asset_id",
                    "record_key",
                    "record_etag",
                    "record_revision",
                    "media_key",
                    "cover_key",
                    "transcript_key",
                    "remote_state",
                ],
            )?;
            columns(
                db,
                "backup_provider_config",
                &[
                    "space_id",
                    "provider_type",
                    "profile",
                    "region",
                    "path_style",
                    "capabilities_json",
                    "protocol",
                    "migration_json",
                ],
            )?;
            let migrating: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM backup_provider_config WHERE migration_json IS NOT NULL)",
                [], |row| row.get(0),
            )?;
            ensure!(!migrating, "BACKUP_MIGRATION_REMOTE_UPGRADE_PENDING");
            // Maintenance probe cleanup runs independently of the sync toggles.
            // An unresolved legacy probe must not become an automatic remote write.
            let cleanup: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM backup_probe_cleanup)",
                [],
                |row| row.get(0),
            )?;
            ensure!(!cleanup, "BACKUP_MIGRATION_PROBE_CLEANUP_PENDING");
            // This is the only rebuilt table. Unknown indexes, triggers or
            // incoming foreign keys could otherwise be lost or cascade on DROP.
            let extra: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE tbl_name='backup_provider_config' COLLATE NOCASE
                 AND type IN ('index','trigger') AND NOT(type='index'
                 AND name='sqlite_autoindex_backup_provider_config_1' COLLATE NOCASE AND sql IS NULL))
                 OR EXISTS(SELECT 1 FROM sqlite_master m,pragma_foreign_key_list(m.name) f
                 WHERE m.type='table' AND f.\"table\"='backup_provider_config' COLLATE NOCASE)",
                [],
                |row| row.get(0),
            )?;
            ensure!(!extra, "BACKUP_MIGRATION_INVALID_STRUCTURE");
        }
        _ => bail!("BACKUP_UNSUPPORTED_SCHEMA"),
    }
    // A SQLite upgrade does not convert an incompatible remote protocol.
    let incompatible: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_provider_config WHERE protocol<>2)",
        [],
        |row| row.get(0),
    )?;
    ensure!(!incompatible, "BACKUP_MIGRATION_UNSUPPORTED_PROTOCOL");
    snapshot(db, old.expect("recognized migration version"))?;
    Ok(Prepared { version: old })
}

fn snapshot(db: &Connection, old: i64) -> Result<()> {
    let Some(path) = db.path().filter(|path| !path.is_empty()) else {
        return Ok(()); // In-memory fixture; no persistent user database exists.
    };
    let source = Path::new(path);
    let parent = source.parent().context("BACKUP_MIGRATION_BACKUP_FAILED")?;
    let name = source
        .file_name()
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?
        .to_string_lossy();
    let temporary = tempfile::Builder::new()
        .prefix(".backup-schema-")
        .tempfile_in(parent)
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?
        .into_temp_path();
    // SQLite's native backup includes committed WAL pages and preserves rowids.
    db.backup(rusqlite::MAIN_DB, &temporary, None)
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?;
    OpenOptions::new()
        .write(true)
        .open(&temporary)
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?
        .sync_all()
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?;
    temporary
        .persist_noclobber(parent.join(format!(
            "{name}.before-backup-v{old}-{}.sqlite",
            uuid::Uuid::new_v4(),
        )))
        .map_err(|error| anyhow::anyhow!(error))
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?;
    Ok(())
}

pub(crate) fn apply(db: &Connection, prepared: &Prepared) -> Result<()> {
    ensure!(
        version(db)? == prepared.version,
        "BACKUP_MIGRATION_DATABASE_CHANGED"
    );
    if prepared.version != Some(5) {
        return Ok(());
    }
    db.execute_batch(include_str!("migrate-v5-v6.sql"))?;
    columns(db, "backup_spaces", SPACES)?;
    columns(db, "asset_remote_objects", OBJECTS)?;
    columns(db, "backup_provider_config", CONFIG)?;
    Ok(())
}

pub(crate) fn finish(db: &Connection, prepared: &Prepared) -> Result<()> {
    if prepared.version == Some(5) {
        // Pause every automatic discovery/retention/upload path. Saved targets,
        // credential references and all blocked/send evidence remain in place.
        db.execute("UPDATE backup_spaces SET enabled=0", [])?;
        db.execute("INSERT INTO backup_sync_preference VALUES(1,0) ON CONFLICT(id) DO UPDATE SET enabled=0", [])?;
        db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_MIGRATION_REVIEW_REQUIRED' WHERE state IN ('pending','running','retry_wait')", [])?;
    }
    if matches!(prepared.version, Some(1 | 5 | 6)) {
        // Completed local deletion is durable evidence for the old global hide.
        // Preserve explicit hides without that evidence; never infer deletion
        // merely from a missing file or a remote tombstone.
        db.execute("INSERT OR IGNORE INTO asset_deletion_hidden_history(space_id,asset_id) SELECT o.space_id,o.asset_id FROM backup_operations o JOIN asset_hidden_history h ON h.asset_id=o.asset_id WHERE o.kind='delete' AND json_extract(o.payload_json,'$.local_cleanup_done')=1", [])?;
        db.execute("DELETE FROM asset_hidden_history WHERE asset_id IN (SELECT asset_id FROM asset_deletion_hidden_history)", [])?;
    }
    if prepared.version != Some(VERSION) {
        db.execute("INSERT OR IGNORE INTO backup_operation_captures SELECT operation_id,json_extract(payload_json,'$.capture') FROM backup_operations WHERE kind='upload' AND json_type(payload_json,'$.capture')='object' AND state<>'completed'", [])?;
        db.execute("UPDATE backup_operations SET payload_json=CASE WHEN state='completed' THEN json_remove(payload_json,'$.capture.cover','$.capture.transcript') ELSE json_remove(payload_json,'$.capture') END WHERE kind='upload' AND json_type(payload_json,'$.capture')='object'", [])?;
    }
    if prepared.version.is_some_and(|version| version < VERSION) {
        db.execute("UPDATE backup_schema SET version=? WHERE id=1", [VERSION])?;
    }
    Ok(())
}
