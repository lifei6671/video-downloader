//! Versioned, portable record contract. Only these fields can leave the device.
use super::model::{Asset, Library};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectRef {
    pub key: String,
    pub bytes: u64,
    pub sha256: String,
    pub container: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub platform: String,
    pub video_id: String,
    pub canonical_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Active {
    pub schema_version: u32,
    pub library_id: String,
    pub asset_id: String,
    pub record_revision: u64,
    pub writer_epoch: u64,
    pub source: Source,
    pub title: Option<String>,
    pub author: Option<String>,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub original_filename: String,
    pub completed_at: Option<String>,
    pub media: ObjectRef,
    pub cover: Option<ObjectRef>,
    pub transcript: Option<ObjectRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Deleted {
    pub schema_version: u32,
    pub library_id: String,
    pub asset_id: String,
    pub record_revision: u64,
    pub writer_epoch: u64,
    pub deleted_at: String,
    pub delete_operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Record {
    Active(Box<Active>),
    Deleted(Deleted),
}

impl Record {
    pub fn revision(&self) -> u64 {
        match self {
            Self::Active(v) => v.record_revision,
            Self::Deleted(v) => v.record_revision,
        }
    }
    pub fn asset_id(&self) -> &str {
        match self {
            Self::Active(v) => &v.asset_id,
            Self::Deleted(v) => &v.asset_id,
        }
    }
    pub fn validate(&self, library: &Library, prefix: &str) -> Result<()> {
        let (schema, id, epoch) = match self {
            Self::Active(v) => (v.schema_version, &v.library_id, v.writer_epoch),
            Self::Deleted(v) => (v.schema_version, &v.library_id, v.writer_epoch),
        };
        if schema != 1
            || id != &library.library_id
            || epoch > library.writer_epoch
            || epoch == 0
            || self.revision() == 0
            || self.revision() > i64::MAX as u64
        {
            bail!("BACKUP_INVALID_RECORD");
        }
        uuid::Uuid::parse_str(self.asset_id()).context("BACKUP_INVALID_RECORD")?;
        if let Self::Active(v) = self {
            if let Some(completed) = &v.completed_at {
                chrono::DateTime::parse_from_rfc3339(completed).context("BACKUP_INVALID_RECORD")?;
            }
            if let Some(source) = &v.source.canonical_url {
                let url = url::Url::parse(source).context("BACKUP_INVALID_SOURCE")?;
                let host = url.host_str().unwrap_or("");
                let valid = match v.source.platform.as_str() {
                    "youtube" => matches!(host, "www.youtube.com" | "youtube.com" | "youtu.be"),
                    "douyin" => matches!(host, "www.douyin.com" | "douyin.com"),
                    "tiktok" => matches!(host, "www.tiktok.com" | "tiktok.com"),
                    "telegram" => host == "t.me",
                    _ => false,
                };
                if !valid
                    || url.scheme() != "https"
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                    || url.query_pairs().any(|(key, value)| {
                        v.source.platform != "youtube" || key != "v" || value != v.source.video_id
                    })
                {
                    bail!("BACKUP_INVALID_SOURCE");
                }
            }

            if !matches!(
                v.source.platform.as_str(),
                "douyin" | "tiktok" | "youtube" | "telegram"
            ) || v.source.video_id.len() > 128
                || v.source.video_id.is_empty()
                || v.original_filename.len() > 1024
                || v.original_filename.is_empty()
                || v.original_filename.contains(['/', '\\', '\0'])
                || matches!(v.original_filename.as_str(), "." | "..")
                || v.title.as_ref().is_some_and(|s| s.len() > 65536)
                || v.author.as_ref().is_some_and(|s| s.len() > 8192)
            {
                bail!("BACKUP_INVALID_RECORD");
            }
            for (object, kind) in std::iter::once((&v.media, "video"))
                .chain(v.cover.as_ref().map(|v| (v, "cover")))
                .chain(v.transcript.as_ref().map(|v| (v, "transcript")))
            {
                if object.bytes == 0
                    || object.bytes > 5 * 1024_u64.pow(4)
                    || object.sha256.len() != 64
                    || !object.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    bail!("BACKUP_INVALID_OBJECT");
                }
                let ext = &object.container;
                let valid = match kind {
                    "video" => matches!(ext.as_str(), "mp4" | "webm" | "mov" | "mkv"),
                    "cover" => matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp"),
                    _ => ext == "json",
                };
                let folder = if kind == "transcript" {
                    "transcripts"
                } else {
                    "media"
                };
                let version = if kind == "video" {
                    String::new()
                } else {
                    format!("{}/", object.sha256)
                };
                let expected = format!(
                    "{prefix}{folder}/{}/{}/{version}{kind}.{ext}",
                    v.asset_id, v.media.sha256
                );
                if !valid || object.key != expected {
                    bail!("BACKUP_INVALID_KEY");
                }
                if kind != "video" && object.bytes > 16 * 1024 * 1024 {
                    bail!("BACKUP_METADATA_TOO_LARGE");
                }
            }
        }
        Ok(())
    }
}

impl Active {
    pub fn from_asset(asset: &Asset, library: &Library, prefix: &str) -> Result<Self> {
        let hash = asset
            .content_sha256
            .as_ref()
            .context("BACKUP_HASH_PENDING")?;
        let ext = std::path::Path::new(&asset.original_filename)
            .extension()
            .and_then(|s| s.to_str())
            .context("BACKUP_INVALID_FILENAME")?
            .to_ascii_lowercase();
        let m = &asset.metadata;
        Ok(Self {
            schema_version: 1,
            library_id: library.library_id.clone(),
            asset_id: asset.asset_id.clone(),
            record_revision: 1,
            writer_epoch: library.writer_epoch,
            source: Source {
                platform: m["platform"]
                    .as_str()
                    .context("BACKUP_INVALID_RECORD")?
                    .into(),
                video_id: m["video_id"]
                    .as_str()
                    .context("BACKUP_INVALID_RECORD")?
                    .into(),
                canonical_url: m["canonical_url"].as_str().map(str::to_owned),
            },
            title: m["title"].as_str().map(str::to_owned),
            author: m["author"].as_str().map(str::to_owned),
            duration_ms: m["duration_ms"].as_u64(),
            width: m["width"].as_u64().and_then(|v| v.try_into().ok()),
            height: m["height"].as_u64().and_then(|v| v.try_into().ok()),
            original_filename: asset.original_filename.clone(),
            completed_at: m["completed_at"]
                .as_i64()
                .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
                .map(|v| v.to_rfc3339())
                .or_else(|| m["completed_at"].as_str().map(str::to_owned)),
            media: ObjectRef {
                key: format!("{prefix}media/{}/{hash}/video.{ext}", asset.asset_id),
                bytes: asset.content_bytes,
                sha256: hash.clone(),
                container: ext,
            },
            cover: None,
            transcript: None,
        })
    }
}
