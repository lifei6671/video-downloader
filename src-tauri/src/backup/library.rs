//! Canonical library metadata and resumable initialization during explicit storage connection.
use super::{
    model::Library,
    record::Record,
    store::{Condition, ObjectStore, Write},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Reconcile legacy outcomes before allowing any v2 scheduling. No media is read or uploaded.
pub async fn reconcile_local<S: ObjectStore>(
    core: &crate::desktop_core::Core,
    store: &S,
    space: &super::model::Space,
) -> Result<()> {
    use rusqlite::params;
    let library_key = format!("{}library.json", space.prefix);
    let object = store
        .get(&library_key)
        .await?
        .context("BACKUP_LIBRARY_NOT_FOUND")?;
    let library: LibraryMetadata = serde_json::from_slice(&object.body)?;
    library.validate()?;
    if library.library_id != space.library_id {
        bail!("BACKUP_LIBRARY_MISMATCH");
    }
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let mut q=tx.prepare("SELECT asset_id,record_key FROM asset_remote_objects WHERE space_id=?1 AND NOT EXISTS(SELECT 1 FROM backup_space_assets m WHERE m.space_id=?1 AND m.local_asset_id=asset_remote_objects.asset_id)")?;
        let rows=q.query_map([&space.space_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (local,key) in rows {
            let prefix=format!("{}records/",space.prefix);
            let remote=key.strip_prefix(&prefix).and_then(|v|v.strip_suffix(".json")).context("BACKUP_INVALID_RECORD_KEY")?;
            uuid::Uuid::parse_str(remote).context("BACKUP_INVALID_RECORD_KEY")?;
            tx.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id) VALUES(?1,?2,?3)",params![space.space_id,remote,local])?;
        }
        drop(q);tx.commit()?;Ok(())
    })?;
    let device = core.backup_db(super::repository::device_id)?;
    let operations = super::scheduler::operations(core)?;
    for op in operations.into_iter().filter(|op| {
        op.space_id == space.space_id && matches!(op.kind.as_str(), "upload" | "delete")
    }) {
        if op.payload["scope"]["protocol"] == 2 {
            continue;
        }
        let Some(local) = op.asset_id.as_deref() else {
            continue;
        };
        let remote:String=core.backup_db(|db|Ok(db.query_row("SELECT remote_asset_id FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space.space_id,local],|r|r.get(0))?))?;
        let key = format!("{}records/{remote}.json", space.prefix);
        let Some(object) = store.get(&key).await? else {
            // A missing record does not prove a write failed and never proves a deletion.
            core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_LEGACY_OUTCOME_UNKNOWN' WHERE operation_id=? AND state NOT IN ('completed','canceled')",[&op.operation_id])?;Ok(())})?;
            continue;
        };
        let record = super::s3_provider::validate_wire(
            &object.body,
            &space.library_id,
            &remote,
            &space.prefix,
        )?;
        let evidence = op
            .payload
            .get("record")
            .cloned()
            .and_then(|raw| serde_json::from_value::<Record>(raw).ok());
        if let Some(Record::Active(active)) = evidence {
            let original = super::s3_provider::normalize_record(
                Record::Active(active.clone()),
                &record.operation_id,
            )?;
            let matching = serde_json::from_slice::<serde_json::Value>(&original)?
                == serde_json::from_slice::<serde_json::Value>(&object.body)?;
            if matching
                && device == space.writer_device_id
                && space.role == super::model::Role::Primary
                && op.kind == "upload"
                && op.payload["media_key"] == active.media.key
                && op.payload["source_hash"] == active.media.sha256
            {
                core.backup_db(|db|{let tx=db.unchecked_transaction()?;
                    // v3 recorded publication but not whether HEAD reused another device's media.
                    // Confirm this outcome without inventing a local video-upload provenance.
                    tx.execute("UPDATE backup_operations SET state='completed',last_error=NULL WHERE operation_id=?",[&op.operation_id])?;tx.commit()?;Ok(())})?;
                continue;
            }
        }
        if op.state != "completed" && op.state != "canceled" {
            core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_LEGACY_REVIEW_REQUIRED' WHERE operation_id=?",[&op.operation_id])?;Ok(())})?;
        }
    }
    let members: Vec<String> = core.backup_db(|db| {
        let mut q =
            db.prepare("SELECT remote_asset_id FROM backup_space_assets WHERE space_id=?")?;
        let rows = q.query_map([&space.space_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    let scope = super::provider::Scope {
        space_id: space.space_id.clone(),
        library_id: space.library_id.clone(),
        provider_type: super::provider::ProviderType::S3,
        protocol: 2,
    };
    for remote in members {
        let key = format!("{}records/{remote}.json", space.prefix);
        let Some(object) = store.get(&key).await? else {
            continue;
        };
        let record = super::s3_provider::validate_wire(
            &object.body,
            &space.library_id,
            &remote,
            &space.prefix,
        )?;
        let snapshot = record.snapshot(&scope)?;
        let token = super::provider::OpaqueState {
            scope: scope.clone(),
            value: serde_json::json!({"asset_id":remote,"etag":object.etag}).to_string(),
        };
        core.backup_db(|db|{db.execute("UPDATE backup_space_assets SET baseline_json=?3,version_json=?4,remote_deleted=?5,cloud_visible=NOT ?5 WHERE space_id=?1 AND remote_asset_id=?2 AND baseline_json IS NULL",params![space.space_id,remote,serde_json::to_string(&snapshot)?,serde_json::to_string(&token)?,snapshot.deleted])?;Ok(())})?;
    }
    // The connection owner commits verified capabilities and the supplied/stored credentials
    // together after reconciliation, never during ordinary background discovery.
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Migration {
    pub operation_id: String,
    pub original: Library,
    pub next_page: Option<String>,
    pub converted: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryMetadata {
    #[serde(default, skip_serializing_if = "is_false")]
    pub deletion_indexes: bool,
    pub schema_version: u32,
    pub library_id: String,
    pub mode: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration: Option<Migration>,
}
impl LibraryMetadata {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 2 {
            bail!("BACKUP_UNSUPPORTED_SCHEMA");
        }
        uuid::Uuid::parse_str(&self.library_id).context("BACKUP_INVALID_LIBRARY")?;
        if self.mode == "migrating" {
            bail!("BACKUP_MIGRATION_IN_PROGRESS");
        }
        if self.mode != "multi_writer" || self.migration.is_some() {
            bail!("BACKUP_INVALID_LIBRARY");
        }
        Ok(())
    }
}

/// Normalize development data when a user explicitly connects storage.
/// Conditional metadata writes preserve library identity and allow interrupted initialization to resume.
pub async fn initialize<S: ObjectStore>(
    store: &S,
    prefix: &str,
    library_id: &str,
    operation_id: &str,
) -> Result<LibraryMetadata> {
    uuid::Uuid::parse_str(operation_id).context("BACKUP_INVALID_OPERATION")?;
    let key = format!("{prefix}library.json");
    let object = store.get(&key).await?.context("BACKUP_LIBRARY_NOT_FOUND")?;
    let value: serde_json::Value = serde_json::from_slice(&object.body)?;
    let mut library = if value["schema_version"] == 1 {
        let original: Library = serde_json::from_value(value)?;
        original.validate()?;
        if original.library_id != library_id {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        LibraryMetadata {
            deletion_indexes: false,
            schema_version: 2,
            library_id: library_id.into(),
            mode: "migrating".into(),
            created_at: original.created_at.clone(),
            migration: Some(Migration {
                operation_id: operation_id.into(),
                original,
                next_page: None,
                converted: 0,
            }),
        }
    } else {
        serde_json::from_value::<LibraryMetadata>(value)?
    };
    if library.library_id != library_id {
        bail!("BACKUP_LIBRARY_MISMATCH");
    }
    if library.mode == "multi_writer" {
        library.validate()?;
        return Ok(library);
    }
    if library.schema_version != 2 || library.mode != "migrating" {
        bail!("BACKUP_UNSUPPORTED_SCHEMA");
    }
    let migration = library
        .migration
        .as_ref()
        .context("BACKUP_INVALID_LIBRARY")?;
    if migration.operation_id != operation_id {
        bail!("BACKUP_MIGRATION_OWNED_BY_OTHER");
    }
    let mut etag = store
        .write(Write::Put {
            key: key.clone(),
            body: serde_json::to_vec(&library)?,
            condition: Condition::Match(object.etag),
        })
        .await?;
    let record_prefix = format!("{prefix}records/");
    let mut seen = std::collections::HashSet::new();
    loop {
        let migration = library
            .migration
            .as_ref()
            .context("BACKUP_INVALID_LIBRARY")?
            .clone();
        let page = store
            .list(&record_prefix, migration.next_page.clone())
            .await?;
        for info in page.objects {
            let id = info
                .key
                .strip_prefix(&record_prefix)
                .and_then(|v| v.strip_suffix(".json"))
                .context("BACKUP_INVALID_RECORD_KEY")?;
            uuid::Uuid::parse_str(id).context("BACKUP_INVALID_RECORD_KEY")?;
            let object = store
                .get(&info.key)
                .await?
                .context("BACKUP_OBJECT_NOT_FOUND")?;
            let raw: serde_json::Value = serde_json::from_slice(&object.body)?;
            if raw["schema_version"] == 2 {
                super::s3_provider::validate_wire(&object.body, library_id, id, prefix)?;
                continue;
            }
            let record: Record = serde_json::from_value(raw)?;
            record.validate(&migration.original, prefix)?;
            if record.asset_id() != id {
                bail!("BACKUP_INVALID_RECORD");
            }
            let body = super::s3_provider::normalize_record(record, operation_id)?;
            store
                .write(Write::Put {
                    key: info.key,
                    body,
                    condition: Condition::Match(object.etag),
                })
                .await?;
            library
                .migration
                .as_mut()
                .context("BACKUP_INVALID_LIBRARY")?
                .converted += 1;
        }
        if let Some(next) = &page.next {
            if !seen.insert(next.clone()) {
                bail!("BACKUP_INVALID_PAGE");
            }
        }
        library
            .migration
            .as_mut()
            .context("BACKUP_INVALID_LIBRARY")?
            .next_page = page.next.clone();
        if page.next.is_none() {
            library.mode = "multi_writer".into();
            library.migration = None;
        }
        etag = store
            .write(Write::Put {
                key: key.clone(),
                body: serde_json::to_vec(&library)?,
                condition: Condition::Match(etag),
            })
            .await?;
        if page.next.is_none() {
            break;
        }
    }
    library.validate()?;
    Ok(library)
}
