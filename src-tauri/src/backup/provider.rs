//! Provider-neutral synchronization protocol. Transport receipts are not convergence acknowledgements.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, future::Future, path::PathBuf, pin::Pin};

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    S3,
    Telegram,
}

impl ProviderType {
    pub fn require_implemented(self) -> Result<()> {
        if self == Self::Telegram {
            bail!("BACKUP_PROVIDER_NOT_IMPLEMENTED");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub space_id: String,
    pub library_id: String,
    pub provider_type: ProviderType,
    pub protocol: u32,
}
impl Scope {
    pub fn check(&self, other: &Self) -> Result<()> {
        self.provider_type.require_implemented()?;
        if self != other || self.protocol != 2 {
            bail!("BACKUP_SCOPE_MISMATCH");
        }
        Ok(())
    }
}

/// The payload is interpreted exclusively by the owning provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueState {
    pub scope: Scope,
    pub value: String,
}
pub type VersionToken = OpaqueState;
pub type Checkpoint = OpaqueState;
pub type ResumeState = OpaqueState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider_type", rename_all = "snake_case")]
pub enum MediaLocation {
    S3Object {
        locator: String,
    },
    TelegramMessage {
        account_id: String,
        channel_id: String,
        message_id: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub scope: Scope,
    pub location: MediaLocation,
    pub bytes: u64,
    pub sha256: String,
    pub container: String,
}
impl MediaRef {
    pub fn validate_scope(&self, scope: &Scope) -> Result<()> {
        scope.check(&self.scope)?;
        if !matches!(
            (&self.location, scope.provider_type),
            (MediaLocation::S3Object { .. }, ProviderType::S3)
        ) {
            bail!("BACKUP_MEDIA_PROVIDER_MISMATCH");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeCoordination {
    Unavailable,
    ConditionalWrite,
    EventReplay,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub change_coordination: ChangeCoordination,
    pub incremental_discovery: bool,
    pub range_read: bool,
    /// Continuing an interrupted upload; S3 uses multipart internally but restarts from zero.
    pub resumable_upload: bool,
    pub max_single_media_bytes: u64,
    pub can_delete_owned_media: bool,
}
impl ProviderCapabilities {
    pub fn require_multi_writer(&self) -> Result<()> {
        if self.change_coordination == ChangeCoordination::Unavailable {
            bail!("BACKUP_CONDITIONAL_WRITE_UNSUPPORTED");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryDescriptor {
    pub library_id: String,
    pub protocol: u32,
}

/// An absent map entry means unchanged; a present Value::Null means explicitly cleared.
pub type FieldPatch = BTreeMap<String, Value>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FieldBaseline {
    #[default]
    Missing,
    Value(Value),
}
impl FieldBaseline {
    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Missing => None,
            Self::Value(v) => Some(v),
        }
    }
    pub fn from_value(value: Option<Value>) -> Self {
        value.map(Self::Value).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub enum ReferenceChange<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictVersion {
    pub field: String,
    pub value: Value,
    pub device_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSnapshot {
    pub library_id: String,
    pub asset_id: String,
    pub revision: u64,
    pub deleted: bool,
    pub fields: FieldPatch,
    pub media: Option<MediaRef>,
    pub cover: Option<MediaRef>,
    pub transcript: Option<MediaRef>,
    pub conflicts: Vec<ConflictVersion>,
    pub device_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetChange {
    #[serde(default)]
    pub recreate: bool,
    pub asset_id: String,
    pub base: Option<AssetSnapshot>,
    pub fields: FieldPatch,
    /// Per-edit bases survive acknowledgements of unrelated in-flight changes.
    #[serde(default)]
    pub field_bases: std::collections::BTreeMap<String, FieldBaseline>,
    pub media: Option<MediaRef>,
    #[serde(default)]
    pub cover: ReferenceChange<MediaRef>,
    #[serde(default)]
    pub transcript: ReferenceChange<MediaRef>,
    pub delete: bool,
    pub device_id: String,
    pub operation_id: String,
    #[serde(default)]
    pub conflicts: Vec<ConflictVersion>,
    /// Explicit user choices remove only these exact versions; concurrent alternatives survive.
    #[serde(default)]
    pub resolved_conflicts: Vec<ConflictVersion>,
}

impl AssetChange {
    pub fn validate_scope(&self, scope: &Scope) -> Result<()> {
        scope.provider_type.require_implemented()?;
        for reference in self
            .media
            .iter()
            .chain(match &self.cover {
                ReferenceChange::Set(r) => Some(r),
                _ => None,
            })
            .chain(match &self.transcript {
                ReferenceChange::Set(r) => Some(r),
                _ => None,
            })
        {
            reference.validate_scope(scope)?;
        }
        if let Some(base) = &self.base {
            if base.library_id != scope.library_id || base.asset_id != self.asset_id {
                bail!("BACKUP_SCOPE_MISMATCH");
            }
            for reference in base
                .media
                .iter()
                .chain(base.cover.iter())
                .chain(base.transcript.iter())
            {
                reference.validate_scope(scope)?;
            }
        }
        for conflict in self
            .conflicts
            .iter()
            .chain(self.resolved_conflicts.iter())
            .chain(self.base.iter().flat_map(|b| b.conflicts.iter()))
        {
            if matches!(conflict.field.as_str(), "cover" | "transcript")
                && !conflict.value.is_null()
            {
                let reference: MediaRef = serde_json::from_value(conflict.value.clone())?;
                reference.validate_scope(scope)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteAsset {
    pub snapshot: AssetSnapshot,
    pub version: VersionToken,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub assets: Vec<RemoteAsset>,
    pub checkpoint: Checkpoint,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PublishReceipt {
    Accepted { operation_id: String },
    Converged { asset: Box<RemoteAsset> },
    Retry { reason: String },
}

pub struct MediaStream {
    pub bytes: u64,
    pub offset: u64,
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
}
#[derive(Clone)]
pub struct MediaSource {
    pub recreate_revision: Option<u64>,
    pub asset_id: String,
    pub kind: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub container: String,
    pub operation_id: String,
    pub device_id: String,
    pub control: Option<std::sync::Arc<TransferControl>>,
}

pub struct TransferControl {
    pub check: Box<dyn Fn() -> Result<()> + Send + Sync>,
    pub checkpoint: Box<dyn Fn(ResumeState) -> Result<()> + Send + Sync>,
    pub progress: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadReceipt {
    pub reference: MediaRef,
    pub operation_id: String,
    pub device_id: String,
}

/// Providers own transport coordination; the engine owns merge policy and local state.
pub trait BackupProvider: Send + Sync {
    fn scope(&self) -> &Scope;
    fn capabilities(&self) -> ProviderCapabilities;
    fn open_library(&self, create: bool) -> ProviderFuture<'_, LibraryDescriptor>;
    fn pull_changes(&self, checkpoint: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch>;
    fn read_asset(&self, asset_id: &str) -> ProviderFuture<'_, Option<RemoteAsset>>;
    fn publish_change(
        &self,
        change: AssetChange,
        base: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt>;
    /// Opaque prior state may be used for cleanup; continuation depends on capabilities.
    fn upload_media(
        &self,
        source: MediaSource,
        resume: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt>;
    fn cancel_upload(&self, resume: ResumeState, operation_id: &str) -> ProviderFuture<'_, ()>;
    fn download_media(&self, reference: MediaRef, offset: u64) -> ProviderFuture<'_, MediaStream>;
    fn verify_media(&self, reference: MediaRef) -> ProviderFuture<'_, ()>;
    fn delete_media(&self, reference: MediaRef, operation_id: &str) -> ProviderFuture<'_, ()>;
    /// Compact confirmed deleted records; implementations retain portable deletion evidence.
    fn compact_deleted_records(&self, _candidates: Vec<RemoteAsset>, _retention_days: u32, _now: u64) -> ProviderFuture<'_, Vec<RemoteAsset>> {
        Box::pin(async { bail!("BACKUP_CLEANUP_UNSUPPORTED") })
    }
    fn reconcile_deleted_asset(&self, asset_id: &str, operation_id: &str)
        -> ProviderFuture<'_, ()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_and_success_semantics_are_explicit() {
        let scope = Scope {
            space_id: "a".into(),
            library_id: "lib".into(),
            provider_type: ProviderType::S3,
            protocol: 2,
        };
        let mut wrong = scope.clone();
        wrong.space_id = "b".into();
        assert!(scope.check(&wrong).is_err());
        wrong = scope.clone();
        wrong.protocol = 3;
        assert!(scope.check(&wrong).is_err());
        wrong = scope.clone();
        wrong.provider_type = ProviderType::Telegram;
        assert!(scope.check(&wrong).is_err());
        assert!(ProviderType::Telegram.require_implemented().is_err());
        let receipt: PublishReceipt =
            serde_json::from_str(r#"{"status":"accepted","operation_id":"op"}"#).unwrap();
        assert!(matches!(receipt, PublishReceipt::Accepted { .. }));
        let patch: FieldPatch = serde_json::from_str(r#"{"title":null}"#).unwrap();
        assert_eq!(patch.get("title"), Some(&Value::Null));
        assert!(!patch.contains_key("author"));
    }
}
