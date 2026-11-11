//! Immutable cover bytes shared by local cache, asset and backup owners.
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub(crate) enum Owner {
    Cache,
    Asset,
    Capture,
    InlineUpload,
    LegacyUpload,
    Library,
}

impl Owner {
    fn name(self) -> &'static str {
        match self {
            Self::Cache => "cache",
            Self::Asset => "asset",
            Self::Capture => "capture",
            Self::InlineUpload => "inline_upload",
            Self::LegacyUpload => "legacy_upload",
            Self::Library => "library",
        }
    }
}

pub(crate) fn init(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cover_blobs (
            digest TEXT PRIMARY KEY NOT NULL CHECK(length(digest)=64),
            body BLOB NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS cover_refs (
            owner TEXT NOT NULL CHECK(owner IN ('cache','asset','capture','inline_upload','legacy_upload','library')),
            owner_id TEXT NOT NULL,
            digest TEXT NOT NULL REFERENCES cover_blobs(digest),
            PRIMARY KEY(owner,owner_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS cover_refs_digest ON cover_refs(digest);
         CREATE TRIGGER IF NOT EXISTS cover_ref_deleted AFTER DELETE ON cover_refs BEGIN
            DELETE FROM cover_blobs WHERE digest=OLD.digest
             AND NOT EXISTS(SELECT 1 FROM cover_refs WHERE digest=OLD.digest);
         END;
         CREATE TRIGGER IF NOT EXISTS cover_ref_replaced AFTER UPDATE OF digest ON cover_refs
         WHEN OLD.digest<>NEW.digest BEGIN
            DELETE FROM cover_blobs WHERE digest=OLD.digest
             AND NOT EXISTS(SELECT 1 FROM cover_refs WHERE digest=OLD.digest);
         END;",
    )?;
    Ok(())
}

pub(crate) fn put(db: &Connection, owner: Owner, id: &str, body: &[u8]) -> Result<()> {
    let tx = db
        .is_autocommit()
        .then(|| db.unchecked_transaction())
        .transpose()?;
    let digest = format!("{:x}", Sha256::digest(body));
    db.execute(
        "INSERT INTO cover_blobs(digest,body) VALUES(?1,?2) ON CONFLICT(digest) DO NOTHING",
        params![digest, body],
    )?;
    db.execute(
        "INSERT INTO cover_refs(owner,owner_id,digest) VALUES(?1,?2,?3)
         ON CONFLICT(owner,owner_id) DO UPDATE SET digest=excluded.digest
         WHERE digest<>excluded.digest",
        params![owner.name(), id, digest],
    )?;
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(())
}

pub(crate) fn read(db: &Connection, owner: Owner, id: &str) -> Result<Option<Vec<u8>>> {
    Ok(db
        .query_row(
            "SELECT b.body FROM cover_refs r JOIN cover_blobs b USING(digest)
         WHERE r.owner=?1 AND r.owner_id=?2",
            params![owner.name(), id],
            |row| row.get(0),
        )
        .optional()?)
}

pub(crate) fn remove(db: &Connection, owner: Owner, id: &str) -> Result<()> {
    db.execute(
        "DELETE FROM cover_refs WHERE owner=?1 AND owner_id=?2",
        params![owner.name(), id],
    )?;
    Ok(())
}

pub(crate) fn register_asset(db: &Connection, id: &str, body: &[u8]) -> Result<()> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cover_refs WHERE owner='asset' AND owner_id=?)",
        [id],
        |row| row.get::<_, bool>(0),
    )? {
        put(db, Owner::Asset, id, body)?;
    }
    Ok(())
}

fn table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)",
        [name],
        |row| row.get(0),
    )?)
}

fn cache_has_old_cover(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('parse_cache') WHERE name='cover')",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) fn prepare(db: &Connection, already_backed_up: bool) -> Result<()> {
    if already_backed_up {
        return Ok(());
    }
    if cache_has_old_cover(db)? || table(db, "asset_covers")? || table(db, "asset_resources")? {
        crate::backup::schema_migration::snapshot(db, "before-cover-store-v1")?;
    }
    Ok(())
}

fn move_blobs(db: &Connection, query: &str, owner: Owner) -> Result<()> {
    let mut statement = db.prepare(query)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        put(
            db,
            owner,
            row.get_ref(0)?.as_str()?,
            row.get_ref(1)?.as_blob()?,
        )?;
    }
    Ok(())
}

pub(crate) fn migrate_cache(db: &Connection) -> Result<()> {
    if cache_has_old_cover(db)? {
        move_blobs(
            db,
            "SELECT cache_key,cover FROM parse_cache WHERE cover IS NOT NULL",
            Owner::Cache,
        )?;
        db.execute("ALTER TABLE parse_cache DROP COLUMN cover", [])?;
    }
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS cover_cache_deleted AFTER DELETE ON parse_cache BEGIN
            DELETE FROM cover_refs WHERE owner='cache' AND owner_id=OLD.cache_key;
         END;",
    )?;
    Ok(())
}

pub(crate) fn migrate_backup(db: &Connection) -> Result<()> {
    let old = table(db, "asset_covers")?;
    if old {
        move_blobs(db, "SELECT asset_id,body FROM asset_covers", Owner::Asset)?;
        db.execute("DROP TABLE asset_covers", [])?;
        migrate_captures(db)?;
    }
    db.execute_batch(
        "CREATE VIEW IF NOT EXISTS asset_covers AS
            SELECT r.owner_id AS asset_id,b.body
            FROM cover_refs r JOIN cover_blobs b USING(digest) WHERE r.owner='asset';
         CREATE TRIGGER IF NOT EXISTS cover_asset_deleted AFTER DELETE ON media_assets BEGIN
            DELETE FROM cover_refs WHERE owner='asset' AND owner_id=OLD.asset_id;
         END;
         CREATE TRIGGER IF NOT EXISTS cover_capture_deleted AFTER DELETE ON backup_operation_captures BEGIN
            DELETE FROM cover_refs WHERE owner='capture' AND owner_id=OLD.operation_id;
         END;
         CREATE TRIGGER IF NOT EXISTS cover_inline_deleted AFTER DELETE ON backup_operations BEGIN
            DELETE FROM cover_refs WHERE owner='inline_upload' AND owner_id=OLD.operation_id;
         END;
         CREATE TRIGGER IF NOT EXISTS cover_inline_released AFTER UPDATE OF payload_json,state ON backup_operations
         WHEN json_type(NEW.payload_json,'$.capture') IS NULL
              OR (NEW.kind='upload' AND NEW.state='completed') BEGIN
            DELETE FROM cover_refs WHERE owner='inline_upload' AND owner_id=OLD.operation_id;
         END;
         DELETE FROM cover_refs WHERE owner='inline_upload' AND owner_id IN
            (SELECT operation_id FROM backup_operations WHERE kind='upload' AND state='completed');",
    )?;
    if table(db, "asset_resources")? {
        move_blobs(
            db,
            "SELECT asset_id,body FROM asset_resources WHERE kind='cover'",
            Owner::Library,
        )?;
        db.execute_batch(
            "ALTER TABLE asset_resources RENAME TO legacy_asset_resources;
             DELETE FROM legacy_asset_resources WHERE kind='cover';
             CREATE VIEW asset_resources AS
                SELECT asset_id,kind,body FROM legacy_asset_resources
                UNION ALL
                SELECT r.owner_id,'cover',b.body
                FROM cover_refs r JOIN cover_blobs b USING(digest) WHERE r.owner='library';
             CREATE TRIGGER cover_library_deleted AFTER DELETE ON library_assets BEGIN
                DELETE FROM cover_refs WHERE owner='library' AND owner_id=OLD.id;
             END;",
        )?;
    }
    Ok(())
}

fn take_cover(capture: &mut Value) -> Result<Option<Vec<u8>>> {
    if !capture.get("cover").is_some_and(Value::is_array) {
        return Ok(None);
    }
    let bytes = capture
        .as_object_mut()
        .and_then(|object| object.remove("cover"));
    Ok(bytes.map(serde_json::from_value).transpose()?)
}

pub(crate) fn save_capture(db: &Connection, id: &str, mut capture: Value) -> Result<()> {
    let cover = take_cover(&mut capture)?;
    let tx = db
        .is_autocommit()
        .then(|| db.unchecked_transaction())
        .transpose()?;
    if db.execute(
        "INSERT INTO backup_operation_captures(operation_id,capture_json) VALUES(?1,?2)
         ON CONFLICT(operation_id) DO NOTHING",
        params![id, capture.to_string()],
    )? != 0
    {
        if let Some(body) = cover {
            put(db, Owner::Capture, id, &body)?;
        } else {
            db.execute(
                "INSERT INTO cover_refs(owner,owner_id,digest)
                 SELECT 'capture',owner_id,digest FROM cover_refs
                 WHERE owner='inline_upload' AND owner_id=?",
                [id],
            )?;
        }
    }
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(())
}

pub(crate) fn hydrate_capture(
    db: &Connection,
    id: &str,
    capture: &mut Value,
    separate: bool,
) -> Result<()> {
    let owner = if separate {
        Owner::Capture
    } else {
        Owner::InlineUpload
    };
    if let Some(body) = read(db, owner, id)? {
        capture["cover"] = serde_json::to_value(body)?;
    }
    Ok(())
}

fn migrate_captures(db: &Connection) -> Result<()> {
    let mut q = db.prepare(
        "SELECT operation_id FROM backup_operation_captures
         WHERE json_type(capture_json,'$.cover')='array'",
    )?;
    // Collect keys, so rewriting these rows never interferes with the scan.
    let keys = q
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in keys {
        let raw: String = db.query_row(
            "SELECT capture_json FROM backup_operation_captures WHERE operation_id=?",
            [&id],
            |row| row.get(0),
        )?;
        let mut value: Value = serde_json::from_str(&raw)?;
        if let Some(body) = take_cover(&mut value)? {
            put(db, Owner::Capture, &id, &body)?;
            db.execute(
                "UPDATE backup_operation_captures SET capture_json=?2 WHERE operation_id=?1",
                params![id, value.to_string()],
            )?;
        }
    }
    migrate_inline(db, "backup_operations", Owner::InlineUpload)?;
    if table(db, "backup_legacy_v5_operations")? {
        migrate_inline(db, "backup_legacy_v5_operations", Owner::LegacyUpload)?;
        db.execute_batch(
            "CREATE TRIGGER cover_legacy_upload_deleted AFTER DELETE ON backup_legacy_v5_operations BEGIN
                DELETE FROM cover_refs WHERE owner='legacy_upload' AND owner_id=OLD.operation_id;
             END;",
        )?;
    }
    Ok(())
}

fn migrate_inline(db: &Connection, table: &str, owner: Owner) -> Result<()> {
    let mut q = db.prepare(&format!(
        "SELECT operation_id FROM {table} WHERE json_type(payload_json,'$.capture.cover')='array'",
    ))?;
    let keys = q
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut load = db.prepare(&format!(
        "SELECT payload_json FROM {table} WHERE operation_id=?"
    ))?;
    let mut update = db.prepare(&format!(
        "UPDATE {table} SET payload_json=json_remove(payload_json,'$.capture.cover') WHERE operation_id=?",
    ))?;
    for id in keys {
        let raw: String = load.query_row([&id], |row| row.get(0))?;
        let mut value: Value = serde_json::from_str(&raw)?;
        if let Some(body) = take_cover(&mut value["capture"])? {
            put(db, owner, &id, &body)?;
            update.execute([&id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cover_refs_share_bytes_and_keep_the_captured_version_until_released() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch(
            "CREATE TABLE media_assets(asset_id TEXT PRIMARY KEY);
             CREATE TABLE asset_covers(asset_id TEXT PRIMARY KEY,body BLOB);
             CREATE TABLE backup_operations(operation_id TEXT PRIMARY KEY,payload_json TEXT,kind TEXT,state TEXT);
             CREATE TABLE backup_operation_captures(operation_id TEXT PRIMARY KEY,capture_json TEXT);
             INSERT INTO media_assets VALUES('asset');
             INSERT INTO asset_covers VALUES('asset',X'0102');
             INSERT INTO backup_operations VALUES('upload','{\"capture\":{\"cover\":[1,2],\"remote\":\"asset\"}}','upload','blocked');
             INSERT INTO backup_operations VALUES('cleanup','{\"capture\":{\"cover\":[5]}}','upload','canceled');
             INSERT INTO backup_operations VALUES('completed','{\"capture\":{\"cover\":[6]}}','upload','completed');",
        )?;
        let tx = db.transaction()?;
        init(&tx)?;
        migrate_backup(&tx)?;
        tx.commit()?;

        let original = [1u8, 2];
        let new = [3u8, 4];
        put(&db, Owner::Cache, "preview", &original)?;
        let blobs = || {
            db.query_row("SELECT count(*) FROM cover_blobs", [], |r| {
                r.get::<_, i64>(0)
            })
        };
        assert_eq!(blobs()?, 2);
        db.execute(
            "UPDATE backup_operations SET state='completed' WHERE operation_id='cleanup'",
            [],
        )?;
        assert_eq!(blobs()?, 1);
        assert_eq!(
            read(&db, Owner::InlineUpload, "upload")?.as_deref(),
            Some(original.as_slice())
        );
        let payload: String = db.query_row(
            "SELECT payload_json FROM backup_operations WHERE operation_id='upload'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(
            serde_json::from_str::<Value>(&payload)?,
            json!({"capture":{"remote":"asset"}})
        );

        save_capture(&db, "upload", json!({"remote":"asset"}))?;
        db.execute(
            "UPDATE backup_operations SET payload_json='{}' WHERE operation_id='upload'",
            [],
        )?;
        put(&db, Owner::Asset, "asset", &new)?;
        remove(&db, Owner::Cache, "preview")?;
        assert_eq!(blobs()?, 2);
        let mut capture = json!({"remote":"asset"});
        hydrate_capture(&db, "upload", &mut capture, true)?;
        assert_eq!(capture["cover"], json!(original));
        assert_eq!(
            read(&db, Owner::Asset, "asset")?.as_deref(),
            Some(new.as_slice())
        );

        db.execute(
            "DELETE FROM backup_operation_captures WHERE operation_id='upload'",
            [],
        )?;
        assert_eq!(blobs()?, 1);
        db.execute("DELETE FROM media_assets WHERE asset_id='asset'", [])?;
        assert_eq!(blobs()?, 0);
        Ok(())
    }
}
