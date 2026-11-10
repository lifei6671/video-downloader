//! S3 wire format and adapter; object keys and conditional tokens stay in this module.
use super::{provider::*, record::ObjectRef};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRecord {
    pub schema_version: u32,
    pub library_id: String,
    pub asset_id: String,
    pub record_revision: u64,
    pub state: String,
    pub deleted_at: Option<u64>,
    pub fields: FieldPatch,
    pub media: Option<ObjectRef>,
    pub cover: Option<ObjectRef>,
    pub transcript: Option<ObjectRef>,
    pub conflicts: Vec<ConflictVersion>,
    pub device_id: String,
    pub operation_id: String,
}

fn legacy_object_kind<'a>(prefix: &str, key: &'a str, asset: &str) -> Option<&'a str> {
    let parts: Vec<_> = key.strip_prefix(prefix)?.split('/').collect();
    let (kind, _) = parts.last()?.split_once('.')?;
    let root = if kind == "transcript" {
        "transcripts"
    } else {
        "media"
    };
    let hash = |s: &str| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit());
    if !matches!(kind, "video" | "cover" | "transcript")
        || parts.get(1)? != &asset
        || parts.first()? != &root
    {
        return None;
    }
    match parts.as_slice() {
        [_, _, source, _] if kind == "video" && hash(source) => Some(kind),
        [_, _, source, content, _] if kind != "video" && hash(source) && hash(content) => {
            Some(kind)
        }
        [_, _, source, device, operation, _]
            if hash(source)
                && uuid::Uuid::parse_str(device).is_ok()
                && uuid::Uuid::parse_str(operation).is_ok() =>
        {
            Some(kind)
        }
        _ => None,
    }
}
fn object_generation(prefix: &str, key: &str, asset: &str) -> Result<Option<u64>> {
    if legacy_object_kind(prefix, key, asset).is_some() {
        return Ok(None);
    }
    let name = key
        .strip_prefix(&format!("{prefix}media/{asset}/"))
        .filter(|s| !s.contains('/'))
        .context("BACKUP_INVALID_KEY")?;
    let (_, tail) = name.split_once('-').context("BACKUP_INVALID_KEY")?;
    if let Some(tail) = tail.strip_prefix('r') {
        let (revision, _) = tail.split_once('-').context("BACKUP_INVALID_KEY")?;
        let revision = revision
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0 && *v <= i64::MAX as u64 && v.to_string() == revision)
            .context("BACKUP_INVALID_KEY")?;
        Ok(Some(revision))
    } else {
        Ok(None)
    }
}
fn validate_object(reference: &ObjectRef, prefix: &str, asset: &str, kind: &str) -> Result<()> {
    let valid = match kind {
        "video" => matches!(reference.container.as_str(), "mp4" | "mov" | "webm" | "mkv"),
        "cover" => matches!(
            reference.container.as_str(),
            "jpg" | "jpeg" | "png" | "webp"
        ),
        "transcript" => reference.container == "json",
        _ => false,
    };
    if !valid
        || reference.bytes == 0
        || reference.bytes > 5 * 1024_u64.pow(4)
        || reference.sha256.len() != 64
        || !reference.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("BACKUP_INVALID_OBJECT");
    }
    if kind != "video" && reference.bytes > 16 * 1024 * 1024 {
        bail!("BACKUP_METADATA_TOO_LARGE");
    }
    if legacy_object_kind(prefix, &reference.key, asset) == Some(kind) {
        let parts: Vec<_> = reference
            .key
            .strip_prefix(prefix)
            .context("BACKUP_INVALID_KEY")?
            .split('/')
            .collect();
        let named_hash = if kind == "video" {
            Some(parts[2])
        } else if parts.len() == 5 {
            Some(parts[3])
        } else {
            None
        };
        if named_hash.is_some_and(|hash| hash != reference.sha256)
            || parts.last().copied() != Some(format!("{kind}.{}", reference.container).as_str())
        {
            bail!("BACKUP_INVALID_KEY");
        }
        return Ok(());
    }
    let generation = object_generation(prefix, &reference.key, asset)?
        .map(|r| format!("r{r}-"))
        .unwrap_or_default();
    if reference.key
        != format!(
            "{prefix}media/{asset}/{kind}-{generation}{}.{}",
            reference.sha256, reference.container
        )
    {
        bail!("BACKUP_INVALID_KEY");
    }
    Ok(())
}
fn validate_source(value: &serde_json::Value) -> Result<()> {
    let source: super::record::Source =
        serde_json::from_value(value.clone()).context("BACKUP_INVALID_SOURCE")?;
    if !matches!(
        source.platform.as_str(),
        "douyin" | "xiaohongshu" | "tiktok" | "youtube" | "telegram"
    ) || source.video_id.is_empty()
        || source.video_id.len() > 128
    {
        bail!("BACKUP_INVALID_SOURCE");
    }
    if let Some(raw) = source.canonical_url {
        let url = url::Url::parse(&raw).context("BACKUP_INVALID_SOURCE")?;
        let valid = match source.platform.as_str() {
            "youtube" => matches!(
                url.host_str(),
                Some("www.youtube.com" | "youtube.com" | "youtu.be")
            ),
            "douyin" => matches!(url.host_str(), Some("www.douyin.com" | "douyin.com")),
            "xiaohongshu" => url.host_str() == Some("www.xiaohongshu.com"),
            "tiktok" => matches!(url.host_str(), Some("www.tiktok.com" | "tiktok.com")),
            "telegram" => url.host_str() == Some("t.me"),
            _ => false,
        };
        if !valid
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url
                .query_pairs()
                .any(|(k, v)| source.platform != "youtube" || k != "v" || v != source.video_id)
        {
            bail!("BACKUP_INVALID_SOURCE");
        }
    }
    Ok(())
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn historical_keys_are_readable_only_with_their_original_identity() {
        let asset = "00000000-0000-4000-8000-000000000001";
        let device = "00000000-0000-4000-8000-000000000002";
        let operation = "00000000-0000-4000-8000-000000000003";
        let hash = "a".repeat(64);
        let source = "b".repeat(64);
        for (kind, container, key) in [
            ("video", "mp4", format!("p/media/{asset}/{hash}/video.mp4")),
            (
                "cover",
                "png",
                format!("p/media/{asset}/{source}/{hash}/cover.png"),
            ),
            (
                "transcript",
                "json",
                format!("p/transcripts/{asset}/{source}/{hash}/transcript.json"),
            ),
            (
                "video",
                "mp4",
                format!("p/media/{asset}/{hash}/{device}/{operation}/video.mp4"),
            ),
            (
                "cover",
                "png",
                format!("p/media/{asset}/{source}/{device}/{operation}/cover.png"),
            ),
            (
                "transcript",
                "json",
                format!("p/transcripts/{asset}/{source}/{device}/{operation}/transcript.json"),
            ),
        ] {
            let reference = ObjectRef {
                key,
                bytes: 100,
                sha256: hash.clone(),
                container: container.into(),
            };
            validate_object(&reference, "p/", asset, kind).unwrap();
            assert_eq!(
                object_generation("p/", &reference.key, asset).unwrap(),
                None
            );
            assert!(validate_object(&reference, "other/", asset, kind).is_err());
            assert!(validate_object(&reference, "p/", device, kind).is_err());
            let mut changed = reference.clone();
            changed.container = "webm".into();
            assert!(validate_object(&changed, "p/", asset, kind).is_err());
            if kind == "video" || reference.key.split('/').count() == 6 {
                changed = reference.clone();
                changed.sha256 = source.clone();
                assert!(validate_object(&changed, "p/", asset, kind).is_err());
            }
            changed = reference.clone();
            changed.key = changed.key.replace(asset, "../escape");
            assert!(validate_object(&changed, "p/", asset, kind).is_err());
        }
    }
    #[test]
    fn pre_deletion_index_descriptor_remains_protocol_two() {
        let old = br#"{"schema_version":2,"library_id":"00000000-0000-4000-8000-000000000001","mode":"multi_writer","created_at":"2026-10-30"}"#;
        let library: crate::backup::library::LibraryMetadata = serde_json::from_slice(old).unwrap();
        library.validate().unwrap();
        assert!(!library.deletion_indexes);
    }
    #[test]
    fn only_known_historical_paths_allow_an_unattributed_writer() {
        let asset = "00000000-0000-4000-8000-000000000001";
        let hash = "a".repeat(64);
        let mut value = serde_json::json!({"schema_version":2,"library_id":asset,"asset_id":asset,"record_revision":1,"state":"active","deleted_at":null,"fields":{"source":{"platform":"youtube","video_id":"fixture","canonical_url":null},"original_filename":"fixture.mp4"},"media":{"key":format!("p/media/{asset}/{hash}/video.mp4"),"bytes":24,"sha256":hash,"container":"mp4"},"cover":null,"transcript":null,"conflicts":[],"device_id":"","operation_id":asset});
        let record =
            validate_wire(&serde_json::to_vec(&value).unwrap(), asset, asset, "p/").unwrap();
        assert!(record.device_id.is_empty());
        value["media"]["key"] =
            format!("p/media/{asset}/video-{}.mp4", record.media.unwrap().sha256).into();
        assert!(validate_wire(&serde_json::to_vec(&value).unwrap(), asset, asset, "p/").is_err());
    }
}

impl WireRecord {
    pub(crate) fn snapshot(&self, scope: &Scope) -> Result<AssetSnapshot> {
        if scope.library_id != self.library_id
            || scope.provider_type != ProviderType::S3
            || scope.protocol != 2
        {
            bail!("BACKUP_SCOPE_MISMATCH");
        }
        let wrap = |reference: &ObjectRef| MediaRef {
            scope: scope.clone(),
            location: MediaLocation::S3Object {
                locator: reference.key.clone(),
            },
            bytes: reference.bytes,
            sha256: reference.sha256.clone(),
            container: reference.container.clone(),
        };
        Ok(AssetSnapshot {
            library_id: self.library_id.clone(),
            asset_id: self.asset_id.clone(),
            revision: self.record_revision,
            deleted: self.state == "deleted",
            fields: self.fields.clone(),
            media: self.media.as_ref().map(wrap),
            cover: self.cover.as_ref().map(wrap),
            transcript: self.transcript.as_ref().map(wrap),
            conflicts: self.conflicts.clone(),
            device_id: self.device_id.clone(),
            operation_id: self.operation_id.clone(),
        })
    }
}

pub(crate) fn validate_wire(
    body: &[u8],
    library: &str,
    asset: &str,
    prefix: &str,
) -> Result<WireRecord> {
    let record: WireRecord = serde_json::from_slice(body).context("BACKUP_INVALID_RECORD")?;
    if record.schema_version != 2
        || record.library_id != library
        || record.asset_id != asset
        || record.deleted_at.is_some_and(|time| time > i64::MAX as u64)
        || (record.state == "deleted" && record.deleted_at.is_none())
        || (record.state == "active" && record.deleted_at.is_some())
        || record.record_revision == 0
        || record.record_revision > i64::MAX as u64
        || !matches!(record.state.as_str(), "active" | "deleted")
    {
        bail!("BACKUP_INVALID_RECORD");
    }
    uuid::Uuid::parse_str(asset).context("BACKUP_INVALID_RECORD")?;
    uuid::Uuid::parse_str(&record.operation_id).context("BACKUP_INVALID_RECORD")?;
    if record.state == "active" && record.media.is_none() {
        bail!("BACKUP_INVALID_RECORD");
    }
    for (reference, kind) in record
        .media
        .iter()
        .map(|r| (r, "video"))
        .chain(record.cover.iter().map(|r| (r, "cover")))
        .chain(record.transcript.iter().map(|r| (r, "transcript")))
    {
        validate_object(reference, prefix, asset, kind)?;
    }
    for field in record.fields.keys() {
        if !matches!(
            field.as_str(),
            "source"
                | "title"
                | "author"
                | "duration_ms"
                | "width"
                | "height"
                | "original_filename"
                | "completed_at"
        ) {
            bail!("BACKUP_INVALID_RECORD");
        }
    }
    if record.state == "active" {
        let filename = record
            .fields
            .get("original_filename")
            .and_then(|v| v.as_str())
            .context("BACKUP_INVALID_FILENAME")?;
        if filename.is_empty()
            || filename.len() > 1024
            || filename.contains(['/', '\\', '\0'])
            || matches!(filename, "." | "..")
        {
            bail!("BACKUP_INVALID_FILENAME");
        }
        let source = record
            .fields
            .get("source")
            .context("BACKUP_INVALID_SOURCE")?;
        validate_source(source)?;
    }
    let legacy_unattributed = record.device_id.is_empty()
        && record
            .media
            .as_ref()
            .is_some_and(|r| legacy_object_kind(prefix, &r.key, asset).is_some());
    if !legacy_unattributed {
        uuid::Uuid::parse_str(&record.device_id).context("BACKUP_INVALID_RECORD")?;
    }
    if record.conflicts.len() > 1024 {
        bail!("BACKUP_INVALID_RECORD");
    }
    for conflict in &record.conflicts {
        uuid::Uuid::parse_str(&conflict.device_id).context("BACKUP_INVALID_RECORD")?;
        uuid::Uuid::parse_str(&conflict.operation_id).context("BACKUP_INVALID_RECORD")?;
        validate_field(&conflict.field, &conflict.value)?;
        let mut candidate = record.clone();
        candidate.conflicts.clear();
        match conflict.field.as_str() {
            "cover" => candidate.cover = serde_json::from_value(conflict.value.clone())?,
            "transcript" => candidate.transcript = serde_json::from_value(conflict.value.clone())?,
            field => {
                candidate
                    .fields
                    .insert(field.into(), conflict.value.clone());
            }
        }
        if candidate.state == "active" {
            validate_wire(&serde_json::to_vec(&candidate)?, library, asset, prefix)?;
        }
    }
    for (field, value) in &record.fields {
        validate_field(field, value)?;
    }
    Ok(record)
}

pub(crate) fn validate_field(field: &str, value: &serde_json::Value) -> Result<()> {
    let valid = match field {
        "title" => value.is_null() || value.as_str().is_some_and(|v| v.len() <= 65536),
        "author" => value.is_null() || value.as_str().is_some_and(|v| v.len() <= 8192),
        "duration_ms" => value.is_null() || value.as_u64().is_some_and(|v| v <= i64::MAX as u64),
        "width" | "height" => {
            value.is_null() || value.as_u64().is_some_and(|v| v <= u32::MAX as u64)
        }
        "completed_at" => {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
        }
        "original_filename" => value.as_str().is_some_and(|v| {
            !v.is_empty()
                && v.len() <= 1024
                && !v.contains(['/', '\\', '\0'])
                && !matches!(v, "." | "..")
        }),
        "source" => {
            validate_source(value).is_ok()
                && value.get("author_profile").is_none_or(|profile| {
                    profile.is_null()
                        || profile.as_object().is_some_and(|profile| {
                            profile.iter().all(|(key, value)| match key.as_str() {
                                "id" | "nickname" | "handle" => {
                                    value.is_null()
                                        || value.as_str().is_some_and(|s| s.len() <= 8192)
                                }
                                "kind" => value
                                    .as_str()
                                    .is_some_and(|s| matches!(s, "user" | "channel")),
                                _ => false,
                            })
                        })
                })
        }
        // Reference conflicts are serialized complete typed references by the adapter.
        "cover" | "transcript" => {
            value.is_null() || serde_json::from_value::<ObjectRef>(value.clone()).is_ok()
        }
        _ => false,
    };
    if !valid {
        bail!("BACKUP_INVALID_RECORD");
    }
    Ok(())
}

use super::{
    library::LibraryMetadata,
    model::Space,
    store::{Condition, ObjectStore, Write},
};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

#[path = "s3_deletion_index.rs"]
mod deletion_index;

pub struct S3Provider<S> {
    store: S,
    space: Space,
    scope: Scope,
    capabilities: ProviderCapabilities,
    device_id: String,
}
impl<S: ObjectStore> S3Provider<S> {
    pub fn new(
        store: S,
        space: Space,
        capabilities: ProviderCapabilities,
        device_id: String,
    ) -> Self {
        let scope = Scope {
            space_id: space.space_id.clone(),
            library_id: space.library_id.clone(),
            provider_type: ProviderType::S3,
            protocol: 2,
        };
        Self {
            store,
            space,
            scope,
            capabilities,
            device_id,
        }
    }
    pub fn space(&self) -> &Space {
        &self.space
    }
    fn key(&self, asset: &str) -> Result<String> {
        uuid::Uuid::parse_str(asset).context("BACKUP_INVALID_RECORD")?;
        Ok(format!("{}records/{asset}.json", self.space.prefix))
    }
    fn reference(&self, reference: &MediaRef) -> Result<ObjectRef> {
        reference.validate_scope(&self.scope)?;
        let MediaLocation::S3Object { locator } = &reference.location else {
            bail!("BACKUP_MEDIA_PROVIDER_MISMATCH");
        };
        let asset = self.media_asset(locator)?;
        let reference = ObjectRef {
            key: locator.clone(),
            bytes: reference.bytes,
            sha256: reference.sha256.clone(),
            container: reference.container.clone(),
        };
        let kind = locator
            .rsplit('/')
            .next()
            .and_then(|s| s.split(['-', '.']).next())
            .context("BACKUP_INVALID_KEY")?;
        validate_object(&reference, &self.space.prefix, asset, kind)?;
        Ok(reference)
    }
    fn object_generation(&self, key: &str, asset: &str) -> Result<Option<u64>> {
        object_generation(&self.space.prefix, key, asset)
    }
    fn upload_key(
        &self,
        asset: &str,
        kind: &str,
        sha256: &str,
        container: &str,
        generation: Option<u64>,
    ) -> String {
        let generation = generation
            .map(|revision| format!("r{revision}-"))
            .unwrap_or_default();
        format!(
            "{}media/{asset}/{kind}-{generation}{sha256}.{container}",
            self.space.prefix
        )
    }
    fn wrap(&self, reference: ObjectRef) -> MediaRef {
        MediaRef {
            scope: self.scope.clone(),
            location: MediaLocation::S3Object {
                locator: reference.key,
            },
            bytes: reference.bytes,
            sha256: reference.sha256,
            container: reference.container,
        }
    }
    fn media_asset<'a>(&self, key: &'a str) -> Result<&'a str> {
        let relative = key
            .strip_prefix(&self.space.prefix)
            .context("BACKUP_SCOPE_MISMATCH")?;
        let mut segments = relative.split('/');
        if let Some(asset) = relative.split('/').nth(1) {
            if legacy_object_kind(&self.space.prefix, key, asset).is_some() {
                uuid::Uuid::parse_str(asset).context("BACKUP_INVALID_KEY")?;
                return Ok(asset);
            }
        }
        if segments.next() != Some("media")
            || relative.contains('\\')
            || relative
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            bail!("BACKUP_INVALID_KEY");
        }
        let asset = segments.next().context("BACKUP_INVALID_KEY")?;
        uuid::Uuid::parse_str(asset).context("BACKUP_INVALID_KEY")?;
        let name = segments.next().context("BACKUP_INVALID_KEY")?;
        if segments.next().is_some()
            || !matches!(
                name.split('-').next(),
                Some("video" | "cover" | "transcript")
            )
        {
            bail!("BACKUP_INVALID_KEY");
        }
        Ok(asset)
    }
    fn remote(&self, wire: WireRecord, etag: String) -> Result<RemoteAsset> {
        let asset_id = wire.asset_id.clone();
        let mut snapshot = wire.snapshot(&self.scope)?;
        for conflict in &mut snapshot.conflicts {
            if matches!(conflict.field.as_str(), "cover" | "transcript")
                && !conflict.value.is_null()
            {
                let reference: ObjectRef = serde_json::from_value(conflict.value.clone())?;
                conflict.value = serde_json::to_value(self.wrap(reference))?;
            }
        }
        Ok(RemoteAsset {
            snapshot,
            version: OpaqueState {
                scope: self.scope.clone(),
                value: json!({"asset_id":asset_id,"etag":etag}).to_string(),
            },
        })
    }
    async fn library(&self) -> Result<LibraryMetadata> {
        if !self.space.enabled {
            bail!("BACKUP_SPACE_INACTIVE");
        }
        let object = self
            .store
            .get(&format!("{}library.json", self.space.prefix))
            .await?
            .context("BACKUP_LIBRARY_NOT_FOUND")?;
        let value: Value =
            serde_json::from_slice(&object.body).context("BACKUP_INVALID_LIBRARY")?;
        let library: LibraryMetadata =
            serde_json::from_value(value).context("BACKUP_INVALID_LIBRARY")?;
        library.validate()?;
        if library.library_id != self.scope.library_id {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        Ok(library)
    }
    async fn writable(&self) -> Result<()> {
        self.capabilities.require_multi_writer()?;
        self.library().await?;
        Ok(())
    }
    async fn read(&self, asset: &str) -> Result<Option<RemoteAsset>> {
        let library = self.library().await?;
        let key = self.key(asset)?;
        let Some(object) = self.store.get(&key).await? else {
            return if library.deletion_indexes {
                self.read_deleted_index(asset).await
            } else {
                Ok(None)
            };
        };
        let wire = validate_wire(
            &object.body,
            &self.scope.library_id,
            asset,
            &self.space.prefix,
        )?;
        Ok(Some(self.remote(wire, object.etag)?))
    }
    fn wire(&self, change: &AssetChange) -> Result<WireRecord> {
        uuid::Uuid::parse_str(&change.device_id).context("BACKUP_INVALID_RECORD")?;
        uuid::Uuid::parse_str(&change.operation_id).context("BACKUP_INVALID_OPERATION")?;
        if let Some(base) = &change.base {
            if base.library_id != self.scope.library_id || base.asset_id != change.asset_id {
                bail!("BACKUP_SCOPE_MISMATCH");
            }
            if base.deleted && !change.delete && !change.recreate {
                bail!("BACKUP_ASSET_DELETED");
            }
        }
        let mut fields = change
            .base
            .as_ref()
            .map(|b| b.fields.clone())
            .unwrap_or_default();
        fields.extend(change.fields.clone());
        let media = change
            .media
            .as_ref()
            .or_else(|| change.base.as_ref().and_then(|b| b.media.as_ref()))
            .map(|r| self.reference(r))
            .transpose()?;
        let reference =
            |update: &ReferenceChange<MediaRef>, previous: Option<&MediaRef>| match update {
                ReferenceChange::Unchanged => previous.map(|r| self.reference(r)).transpose(),
                ReferenceChange::Set(r) => self.reference(r).map(Some),
                ReferenceChange::Clear => Ok(None),
            };
        let cover = reference(
            &change.cover,
            change.base.as_ref().and_then(|b| b.cover.as_ref()),
        )?;
        let transcript = reference(
            &change.transcript,
            change.base.as_ref().and_then(|b| b.transcript.as_ref()),
        )?;
        if let (Some(old), Some(new)) = (
            change.base.as_ref().and_then(|b| b.media.as_ref()),
            media.as_ref(),
        ) {
            if old.sha256 != new.sha256 || old.bytes != new.bytes {
                bail!("BACKUP_IMMUTABLE_MEDIA_CHANGED");
            }
        }
        let mut conflicts = change.conflicts.clone();
        for conflict in &mut conflicts {
            if matches!(conflict.field.as_str(), "cover" | "transcript")
                && !conflict.value.is_null()
            {
                conflict.value = serde_json::to_value(
                    self.reference(&serde_json::from_value::<MediaRef>(conflict.value.clone())?)?,
                )?;
            }
        }
        if !change.delete {
            if let Some(base) = &change.base {
                let generation = if base.deleted && change.recreate {
                    Some(base.revision)
                } else {
                    base.media
                        .as_ref()
                        .map(|r| self.reference(r))
                        .transpose()?
                        .map(|r| self.object_generation(&r.key, &change.asset_id))
                        .transpose()?
                        .flatten()
                };
                for reference in media.iter().chain(&cover).chain(&transcript) {
                    if self.object_generation(&reference.key, &change.asset_id)? != generation {
                        bail!("BACKUP_REUPLOAD_STALE");
                    }
                }
            }
        }
        let mut wire = WireRecord {
            schema_version: 2,
            library_id: self.scope.library_id.clone(),
            asset_id: change.asset_id.clone(),
            record_revision: change.base.as_ref().map_or(1, |b| b.revision + 1),
            deleted_at: change
                .delete
                .then(|| chrono::Utc::now().timestamp().max(0) as u64),
            state: if change.delete { "deleted" } else { "active" }.into(),
            fields,
            media,
            cover,
            transcript,
            conflicts,
            device_id: change.device_id.clone(),
            operation_id: change.operation_id.clone(),
        };
        if change.delete {
            wire.fields.clear();
            wire.media = None;
            wire.cover = None;
            wire.transcript = None;
            wire.conflicts.clear();
        }
        validate_wire(
            &serde_json::to_vec(&wire)?,
            &self.scope.library_id,
            &change.asset_id,
            &self.space.prefix,
        )?;
        Ok(wire)
    }
}

impl<S: ObjectStore> BackupProvider for S3Provider<S> {
    fn scope(&self) -> &Scope {
        &self.scope
    }
    fn capabilities(&self) -> ProviderCapabilities {
        let mut capabilities = self.capabilities.clone();
        capabilities.resumable_upload = false;
        capabilities
    }
    fn open_library(&self, create: bool) -> ProviderFuture<'_, LibraryDescriptor> {
        Box::pin(async move {
            if create {
                self.capabilities.require_multi_writer()?;
                let library = LibraryMetadata {
                    deletion_indexes: false,
                    schema_version: 2,
                    library_id: self.scope.library_id.clone(),
                    mode: "multi_writer".into(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                match self
                    .store
                    .write(Write::Put {
                        key: format!("{}library.json", self.space.prefix),
                        body: serde_json::to_vec(&library)?,
                        condition: Condition::Absent,
                    })
                    .await
                {
                    Ok(_) => {}
                    Err(e) if e.to_string() == "BACKUP_PRECONDITION_FAILED" => {}
                    Err(e) => return Err(e),
                }
            }
            let library = self.library().await?;
            Ok(LibraryDescriptor {
                library_id: library.library_id,
                protocol: 2,
            })
        })
    }
    fn read_asset(&self, asset_id: &str) -> ProviderFuture<'_, Option<RemoteAsset>> {
        let asset = asset_id.to_owned();
        Box::pin(async move { self.read(&asset).await })
    }
    fn pull_changes(&self, checkpoint: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch> {
        Box::pin(async move {
            let library = self.library().await?;
            let known: std::collections::BTreeMap<String, String> = match checkpoint {
                Some(c) => {
                    self.scope.check(&c.scope)?;
                    serde_json::from_str(&c.value).context("BACKUP_INVALID_CHECKPOINT")?
                }
                None => Default::default(),
            };
            let mut cache = known.clone();
            let mut full_records = std::collections::BTreeMap::new();
            let mut assets = Vec::new();
            let mut token = None;
            let mut seen = std::collections::HashSet::new();
            let prefix = format!("{}records/", self.space.prefix);
            loop {
                let page = self.store.list(&prefix, token).await?;
                for info in page.objects {
                    let id = info
                        .key
                        .strip_prefix(&prefix)
                        .and_then(|v| v.strip_suffix(".json"))
                        .context("BACKUP_INVALID_RECORD_KEY")?;
                    uuid::Uuid::parse_str(id).context("BACKUP_INVALID_RECORD_KEY")?;
                    let revision_key = format!("@record/{id}");
                    let cached_revision = known.get(&revision_key).and_then(|value| {
                        let (revision, state) = value.split_once(':')?;
                        if !matches!(state, "active" | "deleted") {
                            return None;
                        }
                        Some((revision.parse::<u64>().ok()?, state == "deleted"))
                    });
                    if known.get(id) == Some(&info.etag) {
                        if let Some(revision) = cached_revision {
                            full_records.insert(id.to_owned(), revision);
                            continue;
                        }
                    }
                    let Some(object) = self.store.get(&info.key).await? else {
                        continue;
                    };
                    let wire = validate_wire(
                        &object.body,
                        &self.scope.library_id,
                        id,
                        &self.space.prefix,
                    )?;
                    cache.insert(
                        revision_key,
                        format!("{}:{}", wire.record_revision, wire.state),
                    );
                    full_records.insert(
                        id.to_owned(),
                        (wire.record_revision, wire.state == "deleted"),
                    );
                    cache.insert(id.into(), object.etag.clone());
                    assets.push(self.remote(wire, object.etag)?);
                }
                token = page.next;
                if let Some(next) = &token {
                    if !seen.insert(next.clone()) {
                        bail!("BACKUP_INVALID_PAGE");
                    }
                } else {
                    break;
                }
            }
            let (indexed, index_versions) = if library.deletion_indexes {
                self.read_all_deleted_indexes(&known).await?
            } else {
                (vec![], Default::default())
            };
            cache.extend(index_versions);
            for remote in indexed {
                let id = &remote.snapshot.asset_id;
                if full_records.get(id).is_some_and(|(revision, deleted)| {
                    *revision > remote.snapshot.revision
                        || *revision == remote.snapshot.revision && *deleted
                }) {
                    continue;
                }
                let version = remote.version.value.clone();
                if known.get(id) != Some(&version) {
                    assets.push(remote.clone());
                }
                cache.insert(
                    format!("@record/{id}"),
                    format!("{}:deleted", remote.snapshot.revision),
                );
                cache.insert(id.clone(), version);
            }
            Ok(ChangeBatch {
                assets,
                checkpoint: OpaqueState {
                    scope: self.scope.clone(),
                    value: serde_json::to_string(&cache)?,
                },
                complete: true,
            })
        })
    }
    fn publish_change(
        &self,
        change: AssetChange,
        base: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt> {
        Box::pin(async move {
            change.validate_scope(&self.scope)?;
            self.writable().await?;
            let wire = self.wire(&change)?;
            let condition = match base {
                Some(token) => {
                    self.scope.check(&token.scope)?;
                    let value: Value = serde_json::from_str(&token.value)?;
                    if value["asset_id"] != change.asset_id || change.base.is_none() {
                        bail!("BACKUP_SCOPE_MISMATCH");
                    }
                    if value["archived"] == true {
                        Condition::Absent
                    } else {
                        Condition::Match(
                            value["etag"]
                                .as_str()
                                .context("BACKUP_INVALID_VERSION_TOKEN")?
                                .into(),
                        )
                    }
                }
                None => {
                    if change.base.is_some() {
                        bail!("BACKUP_BASELINE_REQUIRED");
                    }
                    Condition::Absent
                }
            };
            let body = serde_json::to_vec(&wire)?;
            match self
                .store
                .write(Write::Put {
                    key: self.key(&change.asset_id)?,
                    body: body.clone(),
                    condition,
                })
                .await
            {
                Ok(etag) => Ok(PublishReceipt::Converged {
                    asset: Box::new(self.remote(wire, etag)?),
                }),
                Err(error) => {
                    if let Some(remote) = self.read(&change.asset_id).await? {
                        if remote.snapshot.operation_id == change.operation_id {
                            // Operation IDs are immutable identities: verify the full committed projection.
                            let committed = self
                                .store
                                .get(&self.key(&change.asset_id)?)
                                .await?
                                .context("BACKUP_OBJECT_NOT_FOUND")?;
                            if committed.body == body {
                                return Ok(PublishReceipt::Converged {
                                    asset: Box::new(remote),
                                });
                            }
                        }
                    }
                    if matches!(
                        error.to_string().as_str(),
                        "BACKUP_PRECONDITION_FAILED"
                            | "BACKUP_CONCURRENT_UPDATE"
                            | "BACKUP_NETWORK_ERROR"
                            | "BACKUP_TIMEOUT"
                    ) {
                        Ok(PublishReceipt::Retry {
                            reason: error.to_string(),
                        })
                    } else {
                        Err(error)
                    }
                }
            }
        })
    }
    fn upload_media(
        &self,
        source: MediaSource,
        resume: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt> {
        Box::pin(async move { self.upload(source, resume).await })
    }
    fn cancel_upload(&self, resume: ResumeState, operation_id: &str) -> ProviderFuture<'_, ()> {
        let operation_id = operation_id.to_owned();
        Box::pin(async move {
            self.scope.check(&resume.scope)?;
            let state: UploadState =
                serde_json::from_str(&resume.value).context("BACKUP_INVALID_RESUME_STATE")?;
            state.validate(self)?;
            if state.operation_id != operation_id || state.device_id != self.device_id {
                bail!("BACKUP_UPLOAD_NOT_OWNED");
            }
            self.discard_upload(&state).await?;
            Ok(())
        })
    }
    fn download_media(&self, reference: MediaRef, offset: u64) -> ProviderFuture<'_, MediaStream> {
        Box::pin(async move {
            self.library().await?;
            let object = self.reference(&reference)?;
            let head = self
                .store
                .head(&object.key)
                .await?
                .context("BACKUP_OBJECT_NOT_FOUND")?;
            if head.bytes != reference.bytes || offset > head.bytes {
                bail!("BACKUP_SIZE_MISMATCH");
            }
            let requested = if self.capabilities.range_read {
                offset
            } else {
                0
            };
            let mut stream = self
                .store
                .stream(&object.key, requested, Some(head.etag.clone()))
                .await?;
            let mut start = requested;
            if start > 0
                && stream.content_range.as_deref()
                    != Some(&format!("bytes {start}-{}/{}", head.bytes - 1, head.bytes))
            {
                start = 0;
                stream = self
                    .store
                    .stream(&object.key, 0, Some(head.etag.clone()))
                    .await?;
            }
            if stream.info.etag != head.etag || stream.info.bytes != head.bytes - start {
                bail!("BACKUP_SIZE_MISMATCH");
            }
            Ok(MediaStream {
                bytes: head.bytes,
                offset: start,
                reader: stream.reader,
            })
        })
    }
    fn verify_media(&self, reference: MediaRef) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            let object = self.reference(&reference)?;
            self.library().await?;
            let head = self
                .store
                .head(&object.key)
                .await?
                .context("BACKUP_OBJECT_NOT_FOUND")?;
            if head.bytes != reference.bytes || head.sha256.as_deref() != Some(&reference.sha256) {
                bail!("BACKUP_MEDIA_UNVERIFIED");
            }
            Ok(())
        })
    }
    fn delete_media(&self, reference: MediaRef, operation_id: &str) -> ProviderFuture<'_, ()> {
        let operation = operation_id.to_owned();
        Box::pin(async move {
            uuid::Uuid::parse_str(&operation).context("BACKUP_INVALID_OPERATION")?;
            self.writable().await?;
            let object = self.reference(&reference)?;
            if !self.capabilities.can_delete_owned_media {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            let asset = self.media_asset(&object.key)?;
            if legacy_object_kind(&self.space.prefix, &object.key, asset).is_some() {
                bail!("BACKUP_LEGACY_READ_ONLY");
            }
            if !self.read(asset).await?.is_some_and(|r| r.snapshot.deleted) {
                bail!("BACKUP_TOMBSTONE_REQUIRED");
            }
            self.store.write(Write::Delete { key: object.key }).await?;
            Ok(())
        })
    }
    fn compact_deleted_records(
        &self,
        candidates: Vec<RemoteAsset>,
        retention_days: u32,
        now: u64,
    ) -> ProviderFuture<'_, Vec<RemoteAsset>> {
        Box::pin(async move { self.compact_records(candidates, retention_days, now).await })
    }
    fn reconcile_deleted_asset(
        &self,
        asset_id: &str,
        operation_id: &str,
    ) -> ProviderFuture<'_, DeleteCleanup> {
        let asset = asset_id.to_owned();
        let operation = operation_id.to_owned();
        Box::pin(async move {
            uuid::Uuid::parse_str(&operation).context("BACKUP_INVALID_OPERATION")?;
            self.writable().await?;
            if !self.capabilities.can_delete_owned_media {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            let deletion = self
                .read(&asset)
                .await?
                .filter(|r| r.snapshot.deleted)
                .context("BACKUP_TOMBSTONE_REQUIRED")?;
            let revision = deletion.snapshot.revision;
            let old_generation = |key: &str| -> Result<bool> {
                if legacy_object_kind(&self.space.prefix, key, &asset).is_some() {
                    bail!("BACKUP_LEGACY_READ_ONLY");
                }
                Ok(self
                    .object_generation(key, &asset)?
                    .is_none_or(|generation| generation < revision))
            };
            {
                let prefix = format!("{}media/{asset}/", self.space.prefix);
                for session in self.store.multipart_sessions(&prefix).await? {
                    if !session.key.starts_with(&prefix) || self.media_asset(&session.key)? != asset
                    {
                        bail!("BACKUP_INVALID_KEY");
                    }
                    if !old_generation(&session.key)? {
                        continue;
                    }
                    self.writable().await?;
                    match self
                        .store
                        .write(Write::AbortMultipart {
                            key: session.key,
                            upload_id: session.upload_id,
                        })
                        .await
                    {
                        Ok(_) => {}
                        Err(e) if e.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                        Err(e) => return Err(e),
                    }
                }
                let mut token = None;
                let mut seen = std::collections::HashSet::new();
                let mut keys = Vec::new();
                loop {
                    let page = self.store.list(&prefix, token).await?;
                    for object in page.objects {
                        if !object.key.starts_with(&prefix) {
                            bail!("BACKUP_INVALID_KEY");
                        }
                        self.media_asset(&object.key)?;
                        if old_generation(&object.key)? {
                            keys.push(object.key);
                        }
                    }
                    token = page.next;
                    if let Some(next) = &token {
                        if !seen.insert(next.clone()) {
                            bail!("BACKUP_INVALID_PAGE");
                        }
                    } else {
                        break;
                    }
                }
                for key in keys {
                    self.writable().await?;
                    self.store.write(Write::Delete { key: key.clone() }).await?;
                    if self.store.head(&key).await?.is_some() {
                        bail!("BACKUP_CLEANUP_PENDING");
                    }
                }
            }
            Ok(DeleteCleanup::Complete)
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UploadState {
    completed_bytes: u64,
    asset_id: String,
    operation_id: String,
    device_id: String,
    key: String,
    sha256: String,
    bytes: u64,
    upload_id: Option<String>,
}
impl UploadState {
    fn validate<S: ObjectStore>(&self, provider: &S3Provider<S>) -> Result<()> {
        uuid::Uuid::parse_str(&self.operation_id).context("BACKUP_INVALID_OPERATION")?;
        uuid::Uuid::parse_str(&self.device_id).context("BACKUP_INVALID_OPERATION")?;
        let asset = provider.media_asset(&self.key)?;
        if asset != self.asset_id {
            bail!("BACKUP_INVALID_RESUME_STATE");
        }
        let prefix = format!("{}media/{asset}/", provider.space.prefix);
        let flat = self
            .key
            .strip_prefix(&prefix)
            .filter(|name| !name.contains('/'));
        {
            let name = flat.context("BACKUP_INVALID_RESUME_STATE")?;
            let (stem, container) = name
                .rsplit_once('.')
                .context("BACKUP_INVALID_RESUME_STATE")?;
            let kind = stem
                .split('-')
                .next()
                .context("BACKUP_INVALID_RESUME_STATE")?;
            if !matches!(
                (kind, container),
                ("video", "mp4" | "mov" | "mkv" | "webm")
                    | ("cover", "jpg" | "jpeg" | "png" | "webp")
                    | ("transcript", "json")
            ) || self.key
                != provider.upload_key(
                    asset,
                    kind,
                    &self.sha256,
                    container,
                    provider.object_generation(&self.key, asset)?,
                )
            {
                bail!("BACKUP_INVALID_RESUME_STATE");
            }
        }
        Ok(())
    }
}
/// Retry only the same file part; Create/Complete and conditional writes are excluded.
pub(super) async fn upload_part_with_retry(
    store: &impl ObjectStore,
    part: Write,
    check: &(impl Fn() -> Result<()> + Sync),
) -> Result<String> {
    let Write::UploadFilePart { progress, .. } = &part else {
        bail!("BACKUP_INVALID_PART");
    };
    for attempt in 0..3 {
        check()?;
        if let Some(progress) = progress {
            progress.store(0, Ordering::Relaxed);
        }
        match store.write(part.clone()).await {
            Ok(etag) => return Ok(etag),
            Err(error)
                if attempt < 2
                    && matches!(
                        error.to_string().as_str(),
                        "BACKUP_NETWORK_ERROR"
                            | "BACKUP_SERVICE_UNAVAILABLE"
                            | "BACKUP_RATE_LIMITED"
                    ) =>
            {
                for _ in 0..(10 * (attempt + 1)) {
                    check()?;
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("last part attempt returns its result")
}

impl<S: ObjectStore> S3Provider<S> {
    // Callers validate the persisted scope, operation and key before discarding it.
    async fn discard_upload(&self, state: &UploadState) -> Result<()> {
        self.writable().await?;
        let uploads = if let Some(id) = &state.upload_id {
            vec![id.clone()]
        } else {
            self.store.uploads(&state.key).await?
        };
        for upload_id in uploads {
            match self
                .store
                .write(Write::AbortMultipart {
                    key: state.key.clone(),
                    upload_id,
                })
                .await
            {
                Ok(_) => {}
                Err(error) if error.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    async fn upload(
        &self,
        source: MediaSource,
        resume: Option<ResumeState>,
    ) -> Result<UploadReceipt> {
        self.writable().await?;
        if source.bytes > self.capabilities.max_single_media_bytes {
            bail!("BACKUP_MEDIA_TOO_LARGE");
        }
        if source.device_id != self.device_id {
            bail!("BACKUP_UPLOAD_NOT_OWNED");
        }
        let generation = if let Some(revision) = source.recreate_revision {
            Some(revision)
        } else if let Some(current) = self.read(&source.asset_id).await? {
            if current.snapshot.deleted {
                bail!("BACKUP_ASSET_DELETED");
            }
            current
                .snapshot
                .media
                .as_ref()
                .map(|r| self.reference(r))
                .transpose()?
                .map(|r| self.object_generation(&r.key, &source.asset_id))
                .transpose()?
                .flatten()
        } else {
            None
        };
        uuid::Uuid::parse_str(&source.asset_id).context("BACKUP_INVALID_RECORD")?;
        uuid::Uuid::parse_str(&source.operation_id).context("BACKUP_INVALID_OPERATION")?;
        uuid::Uuid::parse_str(&source.device_id).context("BACKUP_INVALID_OPERATION")?;
        if !matches!(
            (source.kind.as_str(), source.container.as_str()),
            ("video", "mp4" | "mov" | "mkv" | "webm")
                | ("cover", "jpg" | "jpeg" | "png" | "webp")
                | ("transcript", "json")
        ) || source.bytes == 0
            || source.sha256.len() != 64
            || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("BACKUP_INVALID_OBJECT");
        }
        if source.kind != "video" && source.bytes > 16 * 1024 * 1024 {
            bail!("BACKUP_METADATA_TOO_LARGE");
        }
        let key = self.upload_key(
            &source.asset_id,
            &source.kind,
            &source.sha256,
            &source.container,
            generation,
        );
        let mut state = UploadState {
            completed_bytes: 0,
            asset_id: source.asset_id.clone(),
            operation_id: source.operation_id.clone(),
            device_id: source.device_id.clone(),
            key: key.clone(),
            sha256: source.sha256.clone(),
            bytes: source.bytes,
            upload_id: None,
        };
        if let Some(resume) = resume {
            self.scope.check(&resume.scope)?;
            let previous: UploadState = serde_json::from_str(&resume.value)?;
            previous.validate(self)?;
            if previous.operation_id != state.operation_id
                || previous.device_id != state.device_id
                || previous.asset_id != state.asset_id
            {
                bail!("BACKUP_INVALID_RESUME_STATE");
            }
            // Persisted state is cleanup evidence only. Each retry starts a fresh transfer.
            self.discard_upload(&previous).await?;
        }

        let check = || -> Result<()> {
            if let Some(control) = &source.control {
                (control.check)()?;
            }
            Ok(())
        };
        let checkpoint = |state: &UploadState| -> Result<()> {
            if let Some(control) = &source.control {
                (control.checkpoint)(OpaqueState {
                    scope: self.scope.clone(),
                    value: serde_json::to_string(state)?,
                })?;
            }
            Ok(())
        };
        check()?;
        checkpoint(&state)?;
        let matches = |h: &super::store::ObjectInfo| {
            h.bytes == source.bytes && h.sha256.as_deref() == Some(&source.sha256)
        };
        let present = self.store.head(&key).await?;
        if present.as_ref().is_some_and(|h| !matches(h)) {
            bail!("BACKUP_OBJECT_CONFLICT");
        }
        if present.is_none() {
            let progress = source.control.as_ref().map(|c| c.progress.clone());
            if source.bytes < 100 * 1024 * 1024 {
                check()?;
                let result = self
                    .store
                    .write(Write::PutFile {
                        key: key.clone(),
                        path: source.path.clone(),
                        bytes: source.bytes,
                        sha256: source.sha256.clone(),
                        progress,
                    })
                    .await;
                if let Err(error) = result {
                    if !self.store.head(&key).await?.as_ref().is_some_and(matches) {
                        return Err(error);
                    }
                }
            } else {
                let part_bytes = (16 * 1024 * 1024_u64)
                    .max(source.bytes.div_ceil(10000).div_ceil(1024 * 1024) * (1024 * 1024));
                // Single-writer backup: discard unfinished sessions at this exact destination,
                // including a previous CreateMultipart whose response was lost. Never adopt parts.
                for upload_id in self.store.uploads(&key).await? {
                    check()?;
                    match self
                        .store
                        .write(Write::AbortMultipart {
                            key: key.clone(),
                            upload_id,
                        })
                        .await
                    {
                        Ok(_) => {}
                        Err(error) if error.to_string() == "BACKUP_UPLOAD_EXPIRED" => {}
                        Err(error) => return Err(error),
                    }
                }
                check()?;
                state.upload_id = Some(
                    self.store
                        .write(Write::CreateMultipart {
                            key: key.clone(),
                            sha256: source.sha256.clone(),
                        })
                        .await?,
                );
                checkpoint(&state)?;
                let mut parts = Vec::<super::store::Part>::new();
                let upload_id = state
                    .upload_id
                    .clone()
                    .context("BACKUP_UPLOAD_ID_MISSING")?;
                for number in 1..=source.bytes.div_ceil(part_bytes) {
                    check()?;
                    let offset = (number - 1) * part_bytes;
                    let bytes = (source.bytes - offset).min(part_bytes);
                    if let Some(progress) = &progress {
                        progress.store(0, Ordering::Relaxed);
                    }
                    state.completed_bytes = parts.iter().map(|p| p.bytes).sum();
                    checkpoint(&state)?;
                    let etag = upload_part_with_retry(
                        &self.store,
                        Write::UploadFilePart {
                            key: key.clone(),
                            upload_id: upload_id.clone(),
                            part: number as u32,
                            path: source.path.clone(),
                            offset,
                            bytes,
                            progress: progress.clone(),
                        },
                        &check,
                    )
                    .await?;
                    parts.retain(|p| p.number != number as u32);
                    parts.push(super::store::Part {
                        number: number as u32,
                        etag,
                        bytes,
                    });
                    if let Some(progress) = &progress {
                        progress.store(0, Ordering::Relaxed);
                    }
                    state.completed_bytes = parts.iter().map(|p| p.bytes).sum();
                    checkpoint(&state)?;
                }
                parts.sort_by_key(|p| p.number);
                check()?;
                checkpoint(&state)?;
                let result = self
                    .store
                    .write(Write::CompleteMultipart {
                        key: key.clone(),
                        upload_id,
                        parts: parts.into_iter().map(|p| (p.number, p.etag)).collect(),
                    })
                    .await;
                if let Err(error) = result {
                    if !self.store.head(&key).await?.as_ref().is_some_and(matches) {
                        if error.to_string() == "BACKUP_CONCURRENT_UPDATE" {
                            if let Some(upload_id) = state.upload_id.take() {
                                self.store
                                    .write(Write::AbortMultipart {
                                        key: key.clone(),
                                        upload_id,
                                    })
                                    .await?;
                                checkpoint(&state)?;
                            }
                        }
                        return Err(error);
                    }
                }
            }
        }
        check()?;
        if !self.store.head(&key).await?.as_ref().is_some_and(matches) {
            bail!("BACKUP_SIZE_MISMATCH");
        }
        Ok(UploadReceipt {
            reference: self.wrap(ObjectRef {
                key,
                bytes: source.bytes,
                sha256: source.sha256,
                container: source.container,
            }),
            operation_id: source.operation_id,
            device_id: source.device_id,
        })
    }
}

/// Probe only operation-owned keys. The caller persists this operation before issuing writes.
pub async fn probe<S: ObjectStore>(
    store: &S,
    prefix: &str,
    operation: &str,
) -> Result<ProviderCapabilities> {
    probe_with_cleanup(store, prefix, operation, false).await
}
pub(super) async fn probe_with_cleanup<S: ObjectStore>(
    store: &S,
    prefix: &str,
    operation: &str,
    require_delete: bool,
) -> Result<ProviderCapabilities> {
    use tokio::io::AsyncReadExt;
    uuid::Uuid::parse_str(operation).context("BACKUP_INVALID_OPERATION")?;
    let key = format!("{prefix}probes/{operation}/conditional");
    let multipart = format!("{prefix}probes/{operation}/multipart");
    let body = b"sync-probe-v2-0123456789".to_vec();
    let mut stage = "conditional_create";
    let result=async {
        let mut etag=store.write(Write::Put {key:key.clone(),body:body.clone(),condition:Condition::Absent}).await?;
        for condition in [Condition::Absent,Condition::Match("invalid-version-token".into())] {
            match store.write(Write::Put {key:key.clone(),body:b"must-not-write".to_vec(),condition}).await {
                Err(e) if e.to_string()=="BACKUP_PRECONDITION_FAILED"=>{},_=>bail!("BACKUP_CONDITIONAL_WRITE_UNSUPPORTED"),
            }
        }
        stage="conditional_read";
        if store.get(&key).await?.context("BACKUP_PROBE_FAILED")?.body!=body {bail!("BACKUP_CONDITIONAL_WRITE_UNSUPPORTED");}
        if require_delete {
            stage="conditional_delete";
            match store.write(Write::DeleteIfMatch{key:key.clone(),etag:"\"invalid-delete-version\"".into()}).await {
                Err(e) if e.to_string()=="BACKUP_PRECONDITION_FAILED"=>{},
                Ok(_)=>bail!("BACKUP_CONDITIONAL_DELETE_UNSUPPORTED"),
                Err(e)=>return Err(e),
            }
            if store.get(&key).await?.context("BACKUP_CONDITIONAL_DELETE_UNSUPPORTED")?.body!=body {bail!("BACKUP_CONDITIONAL_DELETE_UNSUPPORTED");}
            store.write(Write::DeleteIfMatch{key:key.clone(),etag}).await?;
            if store.head(&key).await?.is_some() {bail!("BACKUP_CONDITIONAL_DELETE_UNSUPPORTED");}
            etag=store.write(Write::Put{key:key.clone(),body:body.clone(),condition:Condition::Absent}).await?;
        }
        let a=store.write(Write::Put {key:key.clone(),body:b"probe-writer-a".to_vec(),condition:Condition::Match(etag.clone())});
        let b=store.write(Write::Put {key:key.clone(),body:b"probe-writer-b".to_vec(),condition:Condition::Match(etag)});
        let (a,b)=tokio::join!(a,b);
        let valid=matches!((&a,&b),(Ok(_),Err(e))|(Err(e),Ok(_)) if e.to_string()=="BACKUP_PRECONDITION_FAILED");
        if !valid {bail!("BACKUP_CONDITIONAL_WRITE_UNSUPPORTED");}
        let expected=if a.is_ok() {b"probe-writer-a".as_slice()} else {b"probe-writer-b".as_slice()};
        stage="concurrent_read";
        let current=store.get(&key).await?.context("BACKUP_PROBE_FAILED")?;
        if current.body!=expected {bail!("BACKUP_PROBE_FAILED");}
        stage="range_content";
        let mut stream=store.stream(&key,1,Some(current.etag)).await?;
        if stream.content_range.as_deref()!=Some(&format!("bytes 1-{}/{}",expected.len()-1,expected.len())) {bail!("BACKUP_RANGE_UNSUPPORTED");}
        let mut bytes=Vec::new();stream.reader.read_to_end(&mut bytes).await?;
        if bytes!=expected[1..] {bail!("BACKUP_PROBE_FAILED");}
        stage="list_visibility";
        let mut token=None;let mut found=false;let mut seen=std::collections::HashSet::new();
        loop {
            let page=store.list(&format!("{prefix}probes/{operation}/"),token).await?;
            found|=page.objects.iter().any(|v|v.key==key);
            token=page.next;if let Some(next)=&token {if !seen.insert(next.clone()) {bail!("BACKUP_INVALID_PAGE");}} else {break;}
        }
        if !found {bail!("BACKUP_PROBE_FAILED");}
        use sha2::{Digest,Sha256};
        let hash=format!("{:x}",Sha256::digest(&body));
        stage="multipart_session_visibility";
        let id=store.write(Write::CreateMultipart {key:multipart.clone(),sha256:hash}).await?;
        stage="multipart_part_visibility";
        let etag=store.write(Write::UploadPart {key:multipart.clone(),upload_id:id.clone(),part:1,body:body.clone()}).await?;
        if !store.parts(&multipart,&id).await?.iter().any(|p|p.number==1&&p.bytes==body.len() as u64) {bail!("BACKUP_PROBE_FAILED");}
        stage="multipart_session_visibility";
        let sessions=store.multipart_sessions(&format!("{prefix}probes/{operation}/")).await?;
        // Upload IDs are opaque service handles. Verify a listed handle actually addresses the
        // same operation-owned bytes instead of assuming Create/List return identical strings.
        let own:Vec<_>=sessions.iter().filter(|s|s.key==multipart).collect();
        if own.len()!=1 {bail!("BACKUP_PROBE_FAILED");}
        if !store.parts(&multipart,&own[0].upload_id).await?.iter().any(|p|p.number==1&&p.bytes==body.len() as u64&&p.etag==etag) {bail!("BACKUP_PROBE_FAILED");}
        stage="multipart_content";
        store.write(Write::CompleteMultipart {key:multipart.clone(),upload_id:own[0].upload_id.clone(),parts:vec![(1,etag)]}).await?;
        if store.get(&multipart).await?.context("BACKUP_PROBE_FAILED")?.body!=body {bail!("BACKUP_PROBE_FAILED");}
        Ok(ProviderCapabilities {change_coordination:ChangeCoordination::ConditionalWrite,incremental_discovery:true,range_read:true,resumable_upload:false,max_single_media_bytes:5*1024_u64.pow(4),can_delete_owned_media:true})
    }.await;
    let cleanup = async {
        for key in [&key, &multipart] {
            for upload_id in store.uploads(key).await? {
                store
                    .write(Write::AbortMultipart {
                        key: key.clone(),
                        upload_id,
                    })
                    .await?;
            }
            store.write(Write::Delete { key: key.clone() }).await?;
            if store.head(key).await?.is_some() {
                bail!("BACKUP_CLEANUP_PENDING");
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    cleanup.context("BACKUP_PROBE_CLEANUP_FAILED")?;
    result.map_err(|e| {
        if e.to_string() == "BACKUP_PROBE_FAILED" {
            e.context(format!("BACKUP_PROBE_FAILED: {stage}"))
        } else {
            e
        }
    })
}
