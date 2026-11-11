//! Local-only migrations. Portable records and send evidence are never rewritten here.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::{fs::OpenOptions, path::Path};

const VERSION: i64 = 1;
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
    pub(crate) snapshot_created: bool,
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
    let mut snapshot_created = false;
    match old {
        None => {
            // master has the desktop tables but no backup schema. Back it up before
            // the single release migration changes settings or adds any tables.
            let existing: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
                [], |row| row.get(0),
            )?;
            if existing {
                snapshot(db, "before-backup-v0")?;
                snapshot_created = true;
            }
        }
        Some(VERSION | 9) => {
            columns(db, "backup_spaces", SPACES)?;
            columns(db, "asset_remote_objects", OBJECTS)?;
            columns(db, "backup_provider_config", CONFIG)?;
            // The development branch previously reused v1. Its incomplete layout
            // must not be mistaken for the consolidated release schema.
            columns(
                db,
                "backup_operation_captures",
                &["operation_id", "capture_json"],
            )?;
            columns(
                db,
                "backup_import_receipts",
                &[
                    "space_id",
                    "remote_asset_id",
                    "remote_json",
                    "version_json",
                    "local_revision",
                    "pending_json",
                ],
            )?;
            columns(
                db,
                "asset_deletion_hidden_history",
                &["space_id", "asset_id"],
            )?;
            columns(
                db,
                "backup_operation_display",
                &[
                    "operation_id",
                    "space_id",
                    "asset_id",
                    "kind",
                    "state",
                    "phase",
                    "summary_json",
                    "attempt",
                    "last_error",
                    "created_at",
                    "sequence",
                    "progress_bytes",
                    "progress_started",
                ],
            )?;
            if old == Some(9) {
                // Already-final development databases need only the consolidated
                // version marker; never replay the removed intermediate migrations.
                snapshot(db, "before-backup-v9")?;
                snapshot_created = true;
            }
        }
        _ => bail!("BACKUP_UNSUPPORTED_SCHEMA"),
    }
    Ok(Prepared {
        version: old,
        snapshot_created,
    })
}

pub(crate) fn snapshot(db: &Connection, label: &str) -> Result<()> {
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
        .persist_noclobber(parent.join(format!("{name}.{label}-{}.sqlite", uuid::Uuid::new_v4(),)))
        .map_err(|error| anyhow::anyhow!(error))
        .context("BACKUP_MIGRATION_BACKUP_FAILED")?;
    Ok(())
}

pub(crate) fn check_unchanged(db: &Connection, prepared: &Prepared) -> Result<()> {
    ensure!(
        version(db)? == prepared.version,
        "BACKUP_MIGRATION_DATABASE_CHANGED"
    );
    Ok(())
}

pub(crate) fn finish(db: &Connection, prepared: &Prepared) -> Result<()> {
    if prepared.version != Some(VERSION) {
        db.execute("INSERT INTO backup_schema(id,version) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET version=excluded.version", [VERSION])?;
    }
    Ok(())
}
