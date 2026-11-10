//! Portable original edits and deterministic channel-order projection.
use super::{provider::*, telegram_transfer::Target};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Event {
    pub schema_version: u32,
    pub library_id: String,
    pub change: AssetChange,
}
fn references(
    change: &mut AssetChange,
    mut apply: impl FnMut(&mut MediaRef) -> Result<()>,
) -> Result<()> {
    for value in change
        .media
        .iter_mut()
        .chain(match &mut change.cover {
            ReferenceChange::Set(r) => Some(r),
            _ => None,
        })
        .chain(match &mut change.transcript {
            ReferenceChange::Set(r) => Some(r),
            _ => None,
        })
    {
        apply(value)?;
    }
    if let Some(base) = &mut change.base {
        for value in base
            .media
            .iter_mut()
            .chain(base.cover.iter_mut())
            .chain(base.transcript.iter_mut())
        {
            apply(value)?;
        }
    }
    for conflict in change
        .conflicts
        .iter_mut()
        .chain(change.resolved_conflicts.iter_mut())
        .chain(change.base.iter_mut().flat_map(|b| b.conflicts.iter_mut()))
    {
        if matches!(conflict.field.as_str(), "cover" | "transcript") && !conflict.value.is_null() {
            let mut reference: MediaRef = serde_json::from_value(conflict.value.clone())?;
            apply(&mut reference)?;
            conflict.value = serde_json::to_value(reference)?;
        }
    }
    Ok(())
}
pub(super) fn media_references(change: &AssetChange) -> Result<Vec<MediaRef>> {
    let mut result = vec![];
    let mut copy = change.clone();
    references(&mut copy, |r| {
        result.push(r.clone());
        Ok(())
    })?;
    Ok(result)
}
/// Old edits cannot cross an explicit deletion/recreation boundary.
pub(super) fn validate_incarnation(
    change: &AssetChange,
    current: Option<&AssetSnapshot>,
    last_delete: Option<u64>,
) -> Result<()> {
    if change.recreate {
        let deleted = current
            .filter(|s| s.deleted)
            .context("BACKUP_REUPLOAD_STALE")?;
        if !change
            .base
            .as_ref()
            .is_some_and(|b| b.deleted && b.revision == deleted.revision)
            || change.media.is_none()
        {
            bail!("BACKUP_REUPLOAD_STALE");
        }
        for r in change
            .media
            .iter()
            .chain(match &change.cover {
                ReferenceChange::Set(r) => Some(r),
                _ => None,
            })
            .chain(match &change.transcript {
                ReferenceChange::Set(r) => Some(r),
                _ => None,
            })
        {
            let deleted_message = if deleted.revision <= i32::MAX as u64 {
                deleted.revision
            } else {
                deleted.revision >> 32
            };
            if !matches!(&r.location,MediaLocation::TelegramMessage{message_id,..} if *message_id>deleted_message as i64)
            {
                bail!("BACKUP_REUPLOAD_STALE");
            }
        }
    } else if let Some(deletion) = last_delete {
        if current.is_some_and(|s| s.deleted) {
            bail!("BACKUP_ASSET_DELETED");
        }
        if change.base.as_ref().is_none_or(|b| b.revision < deletion) {
            bail!("BACKUP_REUPLOAD_STALE");
        }
    }
    Ok(())
}
impl Event {
    /// Resolve the one historical reference shape without weakening current MediaRef parsing.
    /// Original event bytes remain the cursor evidence; only the decoded view is enriched.
    pub(super) fn decode_with_documents(
        target: &Target,
        body: &[u8],
        documents: &[super::telegram_transfer::RemoteDocument],
    ) -> Result<Self> {
        let original = Self::decode(target, body);
        if original.is_ok() {
            return original;
        }
        let mut raw: serde_json::Value =
            serde_json::from_slice(body).context("TELEGRAM_INDEX_INVALID")?;
        let asset = raw["change"]["asset_id"]
            .as_str()
            .context("TELEGRAM_INDEX_INVALID")?
            .to_owned();
        fn resolve(
            value: &mut serde_json::Value,
            target: &Target,
            asset: &str,
            documents: &[super::telegram_transfer::RemoteDocument],
            changed: &mut bool,
        ) -> Result<()> {
            match value {
                serde_json::Value::Object(object) => {
                    if object.get("location").is_some_and(|l| {
                        l["provider_type"] == "telegram_message" && l.get("document_id").is_none()
                    }) {
                        let location = &object["location"];
                        anyhow::ensure!(
                            location["account_id"] == target.account_id
                                && location["channel_id"] == target.channel_id,
                            "BACKUP_SCOPE_MISMATCH"
                        );
                        let mut matches = documents.iter().filter(|d| {
                            if Some(i64::from(d.message_id)) != location["message_id"].as_i64()
                                || Some(d.bytes) != object["bytes"].as_u64()
                            {
                                return false;
                            }
                            let Ok(e) = serde_json::from_str::<super::telegram_transfer::Envelope>(
                                &d.caption,
                            ) else {
                                return false;
                            };
                            e.yingqu_media == 2
                                && e.library_id == target.scope.library_id
                                && e.asset_id == asset
                                && object["sha256"] == e.sha256
                                && object["container"] == e.container
                        });
                        let document = matches.next().context("TELEGRAM_MEDIA_CHANGED")?;
                        anyhow::ensure!(matches.next().is_none(), "TELEGRAM_MEDIA_CHANGED");
                        object
                            .get_mut("location")
                            .and_then(serde_json::Value::as_object_mut)
                            .context("TELEGRAM_INDEX_INVALID")?
                            .insert("document_id".into(), document.document_id.into());
                        *changed = true;
                    }
                    for value in object.values_mut() {
                        resolve(value, target, asset, documents, changed)?;
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        resolve(value, target, asset, documents, changed)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut changed = false;
        resolve(&mut raw, target, &asset, documents, &mut changed)?;
        if !changed {
            return original;
        }
        Self::decode(target, &serde_json::to_vec(&raw)?)
    }
    pub(super) fn encode(target: &Target, change: &AssetChange) -> Result<Vec<u8>> {
        let mut event = Self {
            schema_version: 2,
            library_id: target.scope.library_id.clone(),
            change: change.clone(),
        };
        event.validate(target)?;
        references(&mut event.change, |r| {
            r.scope.space_id.clear();
            Ok(())
        })?;
        Ok(serde_json::to_vec(&event)?)
    }
    pub(super) fn decode(target: &Target, body: &[u8]) -> Result<Self> {
        let mut event: Self = serde_json::from_slice(body).context("TELEGRAM_INDEX_INVALID")?;
        references(&mut event.change, |r| {
            if !r.scope.space_id.is_empty() {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            r.scope.space_id.clone_from(&target.scope.space_id);
            Ok(())
        })?;
        event.validate(target)?;
        Ok(event)
    }
    fn validate(&self, target: &Target) -> Result<()> {
        if self.schema_version != 2 || self.library_id != target.scope.library_id {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        let change = &self.change;
        for id in [
            &self.library_id,
            &change.asset_id,
            &change.operation_id,
            &change.device_id,
        ] {
            uuid::Uuid::parse_str(id).context("TELEGRAM_INDEX_INVALID")?;
        }
        change.validate_scope(&target.scope)?;
        let mut checked = change.clone();
        references(&mut checked, |r| {
            target.message_id(r)?;
            if r.bytes == 0
                || r.bytes > i64::MAX as u64
                || r.sha256.len() != 64
                || !r.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || r.container.is_empty()
                || r.container.len() > 12
                || !r.container.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            Ok(())
        })?;
        for (field, value) in &change.fields {
            super::s3_provider::validate_field(field, value)?;
        }
        for conflict in change
            .conflicts
            .iter()
            .chain(change.resolved_conflicts.iter())
            .chain(change.base.iter().flat_map(|b| b.conflicts.iter()))
        {
            if matches!(conflict.field.as_str(), "cover" | "transcript") {
                continue;
            }
            super::s3_provider::validate_field(&conflict.field, &conflict.value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    pub high_water: i64,
    pub evidence: BTreeMap<i64, String>,
}
impl Cursor {
    pub(super) fn covers(&self, old: &Self) -> Result<()> {
        if old
            .evidence
            .iter()
            .any(|(id, hash)| self.evidence.get(id) != Some(hash))
        {
            bail!("TELEGRAM_INDEX_CHANGED");
        }
        Ok(())
    }
}
pub(super) struct Projection {
    pub assets: BTreeMap<String, AssetSnapshot>,
    pub operations: BTreeMap<String, String>,
    pub cursor: Cursor,
    pub last_deletions: BTreeMap<String, u64>,
    pub operation_generations: BTreeMap<String, Option<u64>>,
    pub references: BTreeMap<String, Vec<MediaRef>>,
}
/// Caller supplies a complete fixed-high-water prefix; no partial result becomes a checkpoint.
pub(super) fn replay(
    target: &Target,
    high_water: i64,
    events: Vec<(i64, Vec<u8>)>,
    previous: Option<&Cursor>,
) -> Result<Projection> {
    replay_decoded(target, high_water, events, previous, |body| {
        Event::decode(target, body)
    })
}
pub(super) fn replay_decoded(
    target: &Target,
    high_water: i64,
    mut events: Vec<(i64, Vec<u8>)>,
    previous: Option<&Cursor>,
    mut decode: impl FnMut(&[u8]) -> Result<Event>,
) -> Result<Projection> {
    let mut projection = Projection {
        assets: BTreeMap::new(),
        operations: BTreeMap::new(),
        cursor: Cursor {
            high_water,
            evidence: BTreeMap::new(),
        },
        last_deletions: BTreeMap::new(),
        operation_generations: BTreeMap::new(),
        references: BTreeMap::new(),
    };
    events.sort_by_key(|(id, _)| *id);
    for (id, body) in events {
        if id <= 0 || id > high_water || projection.cursor.evidence.contains_key(&id) {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        let hash = format!("{:x}", Sha256::digest(&body));
        projection.cursor.evidence.insert(id, hash.clone());
        let event = decode(&body)?;
        let change = &event.change;
        if let Some(old) = projection
            .operations
            .insert(change.operation_id.clone(), hash.clone())
        {
            if old != hash {
                bail!("TELEGRAM_OPERATION_MISMATCH");
            }
            continue;
        }
        // Early replaceable rich messages carried their preceding complete snapshot
        // in base. Only raw-message revisions belong to that historical format.
        // This does not fabricate evidence for a missing observed event: covers still
        // verifies every original byte digest at the end of replay.
        if id <= i64::from(i32::MAX) && !projection.assets.contains_key(&change.asset_id) {
            if let Some(base) = &change.base {
                if base.revision == 0
                    || base.revision >= id as u64
                    || !base.deleted && base.media.is_none()
                {
                    bail!("TELEGRAM_INDEX_INVALID");
                }
                for (field, value) in &base.fields {
                    super::s3_provider::validate_field(field, value)?;
                }
                if !base.deleted {
                    super::s3_provider::validate_field(
                        "original_filename",
                        base.fields
                            .get("original_filename")
                            .context("TELEGRAM_INDEX_INVALID")?,
                    )?;
                }
                if base.deleted {
                    projection
                        .last_deletions
                        .insert(change.asset_id.clone(), base.revision);
                }
                projection
                    .assets
                    .insert(change.asset_id.clone(), base.clone());
            }
        }
        let current = projection.assets.get(&change.asset_id);
        projection
            .references
            .entry(change.asset_id.clone())
            .or_default()
            .extend(media_references(change)?);
        let generation = projection.last_deletions.get(&change.asset_id).copied();
        if validate_incarnation(change, current, generation).is_err() {
            continue;
        }
        projection
            .operation_generations
            .insert(change.operation_id.clone(), generation);
        let merged = super::sync_engine::merge(change, current)?;
        let mut fields = current.map(|v| v.fields.clone()).unwrap_or_default();
        fields.extend(merged.fields);
        let reference = |update: ReferenceChange<MediaRef>, old: Option<MediaRef>| match update {
            ReferenceChange::Unchanged => old,
            ReferenceChange::Set(r) => Some(r),
            ReferenceChange::Clear => None,
        };
        let snapshot = AssetSnapshot {
            library_id: target.scope.library_id.clone(),
            asset_id: change.asset_id.clone(),
            revision: id as u64,
            deleted: change.delete,
            fields,
            media: merged
                .media
                .or_else(|| current.and_then(|v| v.media.clone())),
            cover: reference(merged.cover, current.and_then(|v| v.cover.clone())),
            transcript: reference(
                merged.transcript,
                current.and_then(|v| v.transcript.clone()),
            ),
            conflicts: merged.conflicts,
            device_id: change.device_id.clone(),
            operation_id: change.operation_id.clone(),
        };
        if !snapshot.deleted && snapshot.media.is_none() {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        if !snapshot.deleted {
            super::s3_provider::validate_field(
                "original_filename",
                snapshot
                    .fields
                    .get("original_filename")
                    .context("TELEGRAM_INDEX_INVALID")?,
            )?;
        }
        if snapshot.deleted {
            projection
                .last_deletions
                .insert(change.asset_id.clone(), id as u64);
        }
        projection.assets.insert(change.asset_id.clone(), snapshot);
    }
    if let Some(old) = previous {
        projection.cursor.covers(old)?;
    }
    Ok(projection)
}

pub(super) fn remote(scope: &Scope, snapshot: AssetSnapshot) -> RemoteAsset {
    RemoteAsset {
        version: OpaqueState {
            scope: scope.clone(),
            value: snapshot.revision.to_string(),
        },
        snapshot,
    }
}
#[cfg(test)]
#[path = "telegram_replay_tests.rs"]
mod tests;
