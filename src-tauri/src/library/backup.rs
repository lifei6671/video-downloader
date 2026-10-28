use super::transfer::{self, Payload, TransferCallback, TransferProgress};
use super::{
    assets::asset_row,
    documents::{validate_citations, validate_text},
    now, optional_unsigned, sql_integer, unsigned, Asset, Category, Note, Tag,
};
use crate::desktop_core::transcription::Source;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_MANIFEST: u64 = 256 * 1024 * 1024;
const FORMAT: &str = "yingqu-library";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    id: String,
    asset_id: String,
    filename: String,
    size: u64,
    mtime_ns: Option<String>,
    duration_ms: Option<u64>,
    width: Option<u64>,
    height: Option<u64>,
    media: Option<Payload>,
    matches_record: bool,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredSource {
    filename: String,
    size: u64,
    mtime_ns: String,
    current_file: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    id: String,
    asset_id: String,
    file_id: Option<String>,
    kind: String,
    text: String,
    version: u64,
    revision: u64,
    source_document_id: Option<String>,
    recognition_key: Option<String>,
    source: Option<StoredSource>,
    model_revision: Option<String>,
    model_name: Option<String>,
    pipeline_revision: Option<String>,
    backend: Option<String>,
    created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timing: Option<super::TranscriptTiming>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    asset_id: String,
    payload: Option<Payload>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Visit {
    asset_id: String,
    visited_at: u64,
    visit_order: i64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredClip {
    id: String,
    asset_id: String,
    file_id: String,
    title: String,
    reason: String,
    start_ms: u64,
    end_ms: u64,
    document_id: Option<String>,
    document_version: Option<u64>,
    quote: Option<String>,
    created_at: u64,
    source: StoredSource,
}
impl StoredClip {
    fn value(&self) -> super::Clip {
        super::Clip {
            id: self.id.clone(),
            asset_id: self.asset_id.clone(),
            file_id: self.file_id.clone(),
            title: self.title.clone(),
            reason: self.reason.clone(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            document_id: self.document_id.clone(),
            document_version: self.document_version,
            quote: self.quote.clone(),
            created_at: self.created_at,
            missing: false,
            source_changed: false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredKeyframe {
    data: super::KeyframeData,
    source: StoredSource,
    payload: Option<Payload>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    created_at: u64,
    include_media: bool,
    assets: Vec<Asset>,
    files: Vec<StoredFile>,
    categories: Vec<Category>,
    tags: Vec<Tag>,
    documents: Vec<StoredDocument>,
    notes: Vec<Note>,
    category_links: Vec<(String, String)>,
    tag_links: Vec<(String, String)>,
    resources: Vec<Resource>,
    visits: Vec<Visit>,
    #[serde(default)]
    clips: Vec<StoredClip>,
    #[serde(default)]
    keyframes: Vec<StoredKeyframe>,
    #[serde(default)]
    research_topics: Vec<super::ResearchTopic>,
    #[serde(default)]
    topic_members: Vec<super::TopicMember>,
    #[serde(default)]
    ideas: Vec<super::LibraryIdea>,
    #[serde(default)]
    idea_references: Vec<super::IdeaReference>,
}
pub struct BackupSnapshot {
    manifest: Manifest,
    sources: Vec<PathBuf>,
    covers: Vec<Vec<u8>>,
    frame_bodies: Vec<Vec<u8>>,
}
#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub ideas: usize,
    pub topics: usize,
    pub assets: usize,
    pub files: usize,
    pub missing_media: usize,
    pub bytes: u64,
}
#[derive(Debug, Serialize)]
pub struct RestoreReport {
    pub ideas: usize,
    pub topics: usize,
    pub assets: usize,
    pub documents: usize,
    pub notes: usize,
    pub missing_media: usize,
}

fn rows<T>(
    db: &Connection,
    sql: &str,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut query = db.prepare(sql)?;
    let result = query
        .query_map([], map)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(result)
}

/// Capture only explicitly listed library fields. Application settings, task
/// history, credentials and transient operation state are never queried.
pub fn capture_backup(db: &Connection, include_media: bool) -> Result<BackupSnapshot> {
    let assets = rows(db, "SELECT id,title,author,platform,video_id,canonical_url,created_at,updated_at,archived_at FROM library_assets ORDER BY id", asset_row)?;
    let originals = rows(db, "SELECT id,asset_id,path,size,mtime_ns,duration_ms,width,height FROM asset_files ORDER BY id", |r| Ok((StoredFile {
        id:r.get(0)?,asset_id:r.get(1)?,filename:transfer::filename(Path::new(&r.get::<_,String>(2)?)),size:unsigned(r,3)?,mtime_ns:r.get(4)?,duration_ms:optional_unsigned(r,5)?,width:optional_unsigned(r,6)?,height:optional_unsigned(r,7)?,media:None,matches_record:false,
    }, PathBuf::from(r.get::<_,String>(2)?))))?;
    let original_files: HashMap<_, _> = originals
        .iter()
        .map(|(f, p)| (f.id.as_str(), (f, p)))
        .collect();
    let raw_documents = rows(db, "SELECT id,asset_id,file_id,kind,text,version,revision,source_document_id,recognition_key,source_json,model_revision,model_name,pipeline_revision,backend,created_at FROM asset_documents ORDER BY id", |r| Ok((StoredDocument {
        id:r.get(0)?,asset_id:r.get(1)?,file_id:r.get(2)?,kind:r.get(3)?,text:r.get(4)?,version:unsigned(r,5)?,revision:unsigned(r,6)?,source_document_id:r.get(7)?,recognition_key:r.get(8)?,source:None,timing:None,model_revision:r.get(10)?,model_name:r.get(11)?,pipeline_revision:r.get(12)?,backend:r.get(13)?,created_at:unsigned(r,14)?,
    },r.get::<_,Option<String>>(9)?)))?;
    let documents = raw_documents
        .into_iter()
        .map(|(mut document, source)| {
            if let Some(source) = source {
                let source: Source = serde_json::from_str(&source)?;
                let current_file = document
                    .file_id
                    .as_deref()
                    .and_then(|id| original_files.get(id))
                    .is_some_and(|(f, p)| {
                        source.path == **p
                            && source.size == f.size
                            && f.mtime_ns.as_ref() == Some(&source.mtime_ns)
                    });
                document.source = Some(StoredSource {
                    filename: transfer::filename(&source.path),
                    size: source.size,
                    mtime_ns: source.mtime_ns,
                    current_file,
                });
            }
            document.timing = super::timing::read(db, &document.id)?;
            Ok(document)
        })
        .collect::<Result<Vec<_>>>()?;
    let notes = rows(db, "SELECT id,asset_id FROM asset_notes ORDER BY id", |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?
    .into_iter()
    .map(|(id, asset)| super::read_note(db, &asset, &id))
    .collect::<Result<_>>()?;
    let resources = rows(
        db,
        "SELECT asset_id,body FROM asset_resources WHERE kind='cover' ORDER BY asset_id",
        |r| {
            Ok((
                Resource {
                    asset_id: r.get(0)?,
                    payload: None,
                },
                r.get::<_, Vec<u8>>(1)?,
            ))
        },
    )?;
    let (resources, covers) = resources.into_iter().unzip();
    let mut clips = Vec::new();
    for asset in &assets {
        for record in super::clips::records(db, &asset.id)? {
            let c = record.clip;
            let source = record.source;
            let current_file = original_files
                .get(c.file_id.as_str())
                .is_some_and(|(f, p)| {
                    source.path == **p
                        && source.size == f.size
                        && f.mtime_ns.as_ref() == Some(&source.mtime_ns)
                });
            clips.push(StoredClip {
                id: c.id,
                asset_id: c.asset_id,
                file_id: c.file_id,
                title: c.title,
                reason: c.reason,
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                document_id: c.document_id,
                document_version: c.document_version,
                quote: c.quote,
                created_at: c.created_at,
                source: StoredSource {
                    filename: transfer::filename(&source.path),
                    size: source.size,
                    mtime_ns: source.mtime_ns,
                    current_file,
                },
            });
        }
    }
    let mut keyframes = Vec::new();
    let mut frame_bodies = Vec::new();
    for record in super::keyframes::frame_records(db)? {
        let current_file = original_files
            .get(record.data.file_id.as_str())
            .is_some_and(|(f, p)| {
                record.source.path == **p
                    && record.source.size == f.size
                    && f.mtime_ns.as_ref() == Some(&record.source.mtime_ns)
            });
        let source = StoredSource {
            filename: transfer::filename(&record.source.path),
            size: record.source.size,
            mtime_ns: record.source.mtime_ns,
            current_file,
        };
        keyframes.push(StoredKeyframe {
            data: record.data,
            source,
            payload: None,
        });
        frame_bodies.push(record.body);
    }
    let (files, sources) = originals.into_iter().unzip();
    Ok(BackupSnapshot {
        manifest: Manifest {
            format: FORMAT.into(),
            version: 9,
            created_at: now(),
            include_media,
            assets,
            files,
            categories: super::categories(db)?,
            tags: super::tags(db)?,
            documents,
            clips,
            keyframes,
            notes,
            research_topics: super::research_topics(db)?,
            topic_members: super::topics::member_records(db)?,
            ideas: super::ideas::records(db)?,
            idea_references: super::ideas::reference_records(db)?,
            category_links: rows(
                db,
                "SELECT asset_id,category_id FROM asset_categories ORDER BY asset_id,category_id",
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?,
            tag_links: rows(
                db,
                "SELECT asset_id,tag_id FROM asset_tags ORDER BY asset_id,tag_id",
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?,
            resources,
            visits: rows(
                db,
                "SELECT asset_id,visited_at,visit_order FROM asset_visits ORDER BY visit_order",
                |r| {
                    Ok(Visit {
                        asset_id: r.get(0)?,
                        visited_at: unsigned(r, 1)?,
                        visit_order: r.get(2)?,
                    })
                },
            )?,
        },
        sources,
        covers,
        frame_bodies,
    })
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = File::create_new(path).context("IO_ERROR")?;
    file.write_all(bytes).context("IO_ERROR")?;
    file.sync_all().context("IO_ERROR")
}

pub fn write_backup(
    mut snapshot: BackupSnapshot,
    target: &Path,
    notify: &mut TransferCallback<'_>,
) -> Result<BackupReport> {
    let target = transfer::new_target(target)?;
    let stage = tempfile::Builder::new()
        .prefix("yingqu-library-backup-")
        .tempdir_in(target.parent().context("IO_ERROR")?)
        .context("IO_ERROR")?;
    fs::create_dir(stage.path().join("files")).context("IO_ERROR")?;
    let manifest = &mut snapshot.manifest;
    let mut progress = TransferProgress {
        phase: "copying",
        completed_files: 0,
        total_files: manifest.resources.len()
            + manifest.keyframes.len()
            + if manifest.include_media {
                manifest.files.len()
            } else {
                0
            },
        bytes: 0,
    };
    notify(progress)?;
    let mut index = 0;
    for (payload, body) in manifest
        .resources
        .iter_mut()
        .map(|r| &mut r.payload)
        .chain(manifest.keyframes.iter_mut().map(|r| &mut r.payload))
        .zip(snapshot.covers.into_iter().chain(snapshot.frame_bodies))
    {
        let path = format!("files/{index:08}.bin");
        index += 1;
        let (size, sha256) = transfer::copy_hash(
            &mut body.as_slice(),
            Some(&stage.path().join(&path)),
            &mut progress,
            notify,
        )?;
        *payload = Some(Payload { path, size, sha256 });
        progress.completed_files += 1;
    }
    if manifest.include_media {
        for (file, path) in manifest.files.iter_mut().zip(snapshot.sources) {
            notify(progress)?;
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    progress.completed_files += 1;
                    continue;
                }
                Err(error) => return Err(error).context("IO_ERROR"),
                Ok(_) => transfer::regular(&path)?,
            }
            let before = Source::read(&path)?;
            let relative = format!("files/{index:08}.bin");
            index += 1;
            let (size, sha256) = transfer::copy_hash(
                &mut File::open(&path).context("IO_ERROR")?,
                Some(&stage.path().join(&relative)),
                &mut progress,
                notify,
            )?;
            if Source::read(&path)? != before || size != before.size {
                bail!("LIBRARY_BACKUP_SOURCE_CHANGED");
            }
            file.matches_record = size == file.size
                && file
                    .mtime_ns
                    .as_ref()
                    .is_none_or(|mtime| mtime == &before.mtime_ns);
            file.media = Some(Payload {
                path: relative,
                size,
                sha256,
            });
            progress.completed_files += 1;
        }
    }
    progress.phase = "validating";
    notify(progress)?;
    validate_manifest(manifest)?;
    let bytes = serde_json::to_vec_pretty(manifest)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        bail!("LIBRARY_PACKAGE_TOO_LARGE");
    }
    write_synced(&stage.path().join("manifest.json"), &bytes)?;
    write_synced(
        &stage.path().join("manifest.sha256"),
        format!("{:x}", Sha256::digest(&bytes)).as_bytes(),
    )?;
    progress.phase = "committing";
    notify(progress)?;
    transfer::publish(stage.path(), &target)?;
    Ok(BackupReport {
        ideas: manifest.ideas.len(),
        topics: manifest.research_topics.len(),
        assets: manifest.assets.len(),
        files: manifest.files.iter().filter(|f| f.media.is_some()).count(),
        missing_media: manifest.files.iter().filter(|f| f.media.is_none()).count(),
        bytes: progress.bytes,
    })
}

fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("manifest.json");
    transfer::regular(&path)?;
    let mut bytes = Vec::new();
    File::open(&path)
        .context("IO_ERROR")?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .context("IO_ERROR")?;
    if bytes.len() as u64 > MAX_MANIFEST {
        bail!("LIBRARY_PACKAGE_TOO_LARGE");
    }
    let checksum = root.join("manifest.sha256");
    transfer::regular(&checksum)?;
    let mut expected = String::new();
    File::open(checksum)
        .context("IO_ERROR")?
        .take(65)
        .read_to_string(&mut expected)
        .context("LIBRARY_PACKAGE_INVALID")?;
    if expected != format!("{:x}", Sha256::digest(&bytes)) {
        bail!("LIBRARY_PACKAGE_CHECKSUM");
    }
    let header: serde_json::Value =
        serde_json::from_slice(&bytes).context("LIBRARY_PACKAGE_INVALID")?;
    if header.get("format").and_then(|v| v.as_str()) != Some(FORMAT)
        || !matches!(header.get("version").and_then(|v| v.as_u64()), Some(1..=9))
    {
        bail!("LIBRARY_PACKAGE_VERSION");
    }
    let manifest: Manifest = serde_json::from_value(header).context("LIBRARY_PACKAGE_INVALID")?;
    if manifest.version == 1 && manifest.documents.iter().any(|d| d.timing.is_some()) {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    if manifest.version < 9 && (manifest.ideas.iter().any(|i|i.ai_source.is_some()) || manifest.research_topics.iter().any(|t|t.ai_source.is_some())) { bail!("LIBRARY_PACKAGE_INVALID"); }
    if manifest.version < 8 && (!manifest.ideas.is_empty() || !manifest.idea_references.is_empty()) { bail!("LIBRARY_PACKAGE_INVALID"); }
    if manifest.version < 7 && (!manifest.research_topics.is_empty() || !manifest.topic_members.is_empty()) { bail!("LIBRARY_PACKAGE_INVALID"); }
    if manifest.version < 6 && manifest.notes.iter().any(|n|n.comparison.is_some()) { bail!("LIBRARY_PACKAGE_INVALID"); }
    if manifest.version < 5 && manifest.notes.iter().any(|n|n.analysis.is_some()) { bail!("LIBRARY_PACKAGE_INVALID"); }
    if manifest.version < 4 && !manifest.keyframes.is_empty() {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    if manifest.version < 3 && !manifest.clips.is_empty() {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    Ok(manifest)
}

fn media_paths(manifest: &Manifest, root: &Path) -> HashMap<String, PathBuf> {
    manifest
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                f.id.clone(),
                root.join(transfer::media_name(i, &f.filename)),
            )
        })
        .collect()
}

fn rebase_source(
    manifest: &Manifest,
    root: &Path,
    staged_root: Option<&Path>,
    source: &StoredSource,
    file_id: Option<&String>,
) -> Result<Source> {
    // Shared historical source identity must survive restore -> backup cycles.
    let identity = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(file_id, source))?)
    );
    let mut value = Source {
        path: root.join(format!("sources/{identity}.missing")),
        size: source.size,
        mtime_ns: source.mtime_ns.clone(),
    };
    if source.current_file {
        let file_id = file_id.context("LIBRARY_PACKAGE_INVALID")?;
        let file_index = manifest
            .files
            .iter()
            .position(|f| &f.id == file_id)
            .context("LIBRARY_PACKAGE_INVALID")?;
        value.path = root.join(transfer::media_name(
            file_index,
            &manifest.files[file_index].filename,
        ));
        if let Some(stage) = staged_root {
            let file_index = manifest
                .files
                .iter()
                .position(|f| &f.id == file_id)
                .context("LIBRARY_PACKAGE_INVALID")?;
            let file = &manifest.files[file_index];
            if file.matches_record && file.media.is_some() {
                value.mtime_ns =
                    Source::read(&stage.join(transfer::media_name(file_index, &file.filename)))?
                        .mtime_ns;
            }
        }
    }
    Ok(value)
}

fn insert_manifest(
    db: &Connection,
    manifest: &Manifest,
    root: &Path,
    staged_root: Option<&Path>,
) -> Result<()> {
    let paths = media_paths(manifest, root);
    for asset in &manifest.assets {
        db.execute(
            "INSERT INTO library_assets VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                asset.id,
                asset.title,
                asset.author,
                asset.platform,
                asset.video_id,
                asset.canonical_url,
                sql_integer(asset.created_at)?,
                sql_integer(asset.updated_at)?,
                asset.archived_at.map(sql_integer).transpose()?
            ],
        )?;
    }
    for (i, file) in manifest.files.iter().enumerate() {
        let actual = staged_root
            .filter(|_| file.media.is_some())
            .map(|stage| Source::read(&stage.join(transfer::media_name(i, &file.filename))))
            .transpose()?;
        let mtime = if file.matches_record {
            actual
                .as_ref()
                .map(|s| s.mtime_ns.clone())
                .or_else(|| file.mtime_ns.clone())
        } else {
            file.mtime_ns.clone()
        };
        db.execute(
            "INSERT INTO asset_files VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                file.id,
                file.asset_id,
                paths[&file.id].to_str().context("INVALID_INPUT")?,
                sql_integer(file.size)?,
                mtime,
                file.duration_ms.map(sql_integer).transpose()?,
                file.width.map(sql_integer).transpose()?,
                file.height.map(sql_integer).transpose()?
            ],
        )?;
    }
    for category in &manifest.categories {
        db.execute(
            "INSERT INTO library_categories VALUES(?1,?2,?3,?4)",
            params![
                category.id,
                category.parent_id,
                category.name,
                category.position
            ],
        )?;
    }
    for tag in &manifest.tags {
        db.execute(
            "INSERT INTO library_tags VALUES(?1,?2)",
            params![tag.id, tag.name],
        )?;
    }
    for (asset, category) in &manifest.category_links {
        db.execute(
            "INSERT INTO asset_categories VALUES(?1,?2)",
            params![asset, category],
        )?;
    }
    for (asset, tag) in &manifest.tag_links {
        db.execute("INSERT INTO asset_tags VALUES(?1,?2)", params![asset, tag])?;
    }
    let restore_source = |source: &StoredSource, file_id: Option<&String>| -> Result<String> {
        Ok(serde_json::to_string(&rebase_source(
            manifest,
            root,
            staged_root,
            source,
            file_id,
        )?)?)
    };
    for document in &manifest.documents {
        let source = document
            .source
            .as_ref()
            .map(|source| restore_source(source, document.file_id.as_ref()))
            .transpose()?;
        // Restoring rebases paths/mtime; the idempotency key must describe that same result.
        let key = if document.kind == "recognized" {
            let result = crate::desktop_core::transcription::TranscriptResult {
                timing: document.timing.clone(),
                text: document.text.clone(),
                source: serde_json::from_str(
                    source.as_deref().context("LIBRARY_PACKAGE_INVALID")?,
                )?,
                model_revision: document
                    .model_revision
                    .clone()
                    .context("LIBRARY_PACKAGE_INVALID")?,
                model_name: document.model_name.clone(),
                pipeline_revision: document
                    .pipeline_revision
                    .clone()
                    .context("LIBRARY_PACKAGE_INVALID")?,
                backend: document.backend.clone(),
                completed_at: document.created_at,
            };
            Some(format!(
                "{}:{:x}",
                document.asset_id,
                Sha256::digest(serde_json::to_vec(&result)?)
            ))
        } else {
            None
        };
        db.execute("INSERT INTO asset_documents VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![document.id,document.asset_id,document.file_id,document.kind,document.text,sql_integer(document.version)?,sql_integer(document.revision)?,document.source_document_id,key,source,document.model_revision,document.model_name,document.pipeline_revision,document.backend,sql_integer(document.created_at)?])?;
        if let Some(timing) = &document.timing {
            super::timing::store(db, &document.id, timing)?;
        }
    }
    for c in &manifest.clips {
        let source = restore_source(&c.source, Some(&c.file_id))?;
        db.execute(
            "INSERT INTO asset_clips VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                c.id,
                c.asset_id,
                c.file_id,
                c.title,
                c.reason,
                sql_integer(c.start_ms)?,
                sql_integer(c.end_ms)?,
                source,
                c.document_id,
                c.document_version.map(sql_integer).transpose()?,
                c.quote,
                sql_integer(c.created_at)?
            ],
        )?;
    }
    for note in &manifest.notes {
        db.execute(
            "INSERT INTO asset_notes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                note.id,
                note.asset_id,
                note.kind,
                note.body,
                serde_json::to_value(note.origin)?.as_str(),
                note.model_name,
                serde_json::to_string(&note.citations)?,
                sql_integer(note.revision)?,
                sql_integer(note.created_at)?,
                sql_integer(note.updated_at)?
            ],
        )?;
        if let Some(comparison)=&note.comparison { super::comparison::store(db,&note.id,comparison)?; }
        if let Some(analysis)=&note.analysis { super::analysis::store(db,&note.id,analysis)?; }
    }
    for topic in &manifest.research_topics {
        db.execute("INSERT INTO research_topics(id,title,description,revision,created_at,updated_at,ai_source_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![topic.id,topic.title,topic.description,sql_integer(topic.revision)?,sql_integer(topic.created_at)?,sql_integer(topic.updated_at)?,topic.ai_source.as_ref().map(serde_json::to_string).transpose()?])?;
    }
    for member in &manifest.topic_members { super::topics::insert_member(db,member)?; }
    for idea in &manifest.ideas {
        db.execute("INSERT INTO library_ideas(id,title,viewpoint,body,revision,created_at,updated_at,ai_source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![idea.id,idea.title,idea.viewpoint,idea.body,sql_integer(idea.revision)?,sql_integer(idea.created_at)?,sql_integer(idea.updated_at)?,idea.ai_source.as_ref().map(serde_json::to_string).transpose()?])?;
    }
    for reference in &manifest.idea_references { super::ideas::insert_reference(db,reference)?; }
    for visit in &manifest.visits {
        db.execute(
            "INSERT INTO asset_visits VALUES(?1,?2,?3)",
            params![
                visit.asset_id,
                sql_integer(visit.visited_at)?,
                visit.visit_order
            ],
        )?;
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.assets.iter().any(|v| v.id.is_empty())
        || manifest.files.iter().any(|v| v.id.is_empty())
        || manifest.categories.iter().any(|v| v.id.is_empty())
        || manifest.tags.iter().any(|v| v.id.is_empty())
        || manifest.documents.iter().any(|v| v.id.is_empty())
        || manifest.notes.iter().any(|v| v.id.is_empty())
        || manifest.clips.iter().any(|v| v.id.is_empty())
        || manifest.keyframes.iter().any(|v| v.data.id.is_empty())
    {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    let mut payloads = HashSet::new();
    for payload in manifest
        .resources
        .iter()
        .map(|r| r.payload.as_ref())
        .chain(manifest.keyframes.iter().map(|r| r.payload.as_ref()))
        .chain(
            manifest
                .files
                .iter()
                .filter_map(|f| f.media.as_ref())
                .map(Some),
        )
    {
        let payload = payload.context("LIBRARY_PACKAGE_INVALID")?;
        let number = payload
            .path
            .strip_prefix("files/")
            .and_then(|s| s.strip_suffix(".bin"))
            .context("LIBRARY_PACKAGE_PATH")?;
        if number.len() != 8
            || !number.bytes().all(|c| c.is_ascii_digit())
            || !payloads.insert(&payload.path)
        {
            bail!("LIBRARY_PACKAGE_PATH");
        }
        if payload.sha256.len() != 64 || !payload.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("LIBRARY_PACKAGE_CHECKSUM");
        }
    }
    if !manifest.include_media && manifest.files.iter().any(|f| f.media.is_some()) {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    for file in &manifest.files {
        if file.matches_record
            && file
                .media
                .as_ref()
                .is_some_and(|payload| payload.size != file.size)
        {
            bail!("LIBRARY_PACKAGE_INVALID");
        }
    }
    for category in &manifest.categories {
        super::name(&category.name).context("LIBRARY_PACKAGE_INVALID")?;
    }
    for tag in &manifest.tags {
        super::name(&tag.name).context("LIBRARY_PACKAGE_INVALID")?;
    }
    for resource in &manifest.resources {
        if resource
            .payload
            .as_ref()
            .is_some_and(|p| p.size > 16 * 1024 * 1024)
        {
            bail!("LIBRARY_PACKAGE_TOO_LARGE");
        }
    }
    for document in &manifest.documents {
        if let Some(timing) = &document.timing {
            if document.kind != "recognized"
                || document.source.is_none()
                || document.file_id.is_none()
            {
                bail!("LIBRARY_PACKAGE_INVALID");
            }
            timing
                .validate(&document.text)
                .context("LIBRARY_PACKAGE_INVALID")?;
        }
        if document.source.as_ref().is_some_and(|s| s.current_file) {
            let source = document.source.as_ref().unwrap();
            let file = document
                .file_id
                .as_ref()
                .and_then(|id| manifest.files.iter().find(|f| &f.id == id))
                .context("LIBRARY_PACKAGE_INVALID")?;
            if source.size != file.size || file.mtime_ns.as_ref() != Some(&source.mtime_ns) {
                bail!("LIBRARY_PACKAGE_INVALID");
            }
        }
        validate_text(&document.text).context("LIBRARY_PACKAGE_INVALID")?;
        if document.kind == "recognized"
            && (document.source.is_none()
                || document.model_revision.is_none()
                || document.pipeline_revision.is_none())
        {
            bail!("LIBRARY_PACKAGE_INVALID");
        }
    }
    for note in &manifest.notes {
        super::name(&note.kind).context("LIBRARY_PACKAGE_INVALID")?;
        if note.revision == 0 {
            bail!("LIBRARY_PACKAGE_INVALID");
        }
        validate_text(&note.body).context("LIBRARY_PACKAGE_INVALID")?;
    }
    // Use the actual schema constraints, including deferred references, to
    // validate the complete graph before creating a restored output directory.
    let mut db = Connection::open_in_memory()?;
    super::initialize(&mut db, Path::new(":memory:"))?;
    let tx = db.transaction()?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
    insert_manifest(&tx, manifest, Path::new("restored"), None)
        .context("LIBRARY_PACKAGE_INVALID")?;
    let invalid_owner:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_documents d JOIN asset_files f ON f.id=d.file_id WHERE f.asset_id!=d.asset_id) OR EXISTS(SELECT 1 FROM asset_documents d JOIN asset_documents s ON s.id=d.source_document_id WHERE s.asset_id!=d.asset_id)",[],|r|r.get(0))?;
    let cycle:bool=tx.query_row("WITH RECURSIVE ancestors(id,parent) AS (SELECT id,parent_id FROM library_categories UNION SELECT a.id,c.parent_id FROM ancestors a JOIN library_categories c ON c.id=a.parent) SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=parent)",[],|r|r.get(0))?;
    let document_cycle:bool=tx.query_row("WITH RECURSIVE sources(id,parent) AS (SELECT id,source_document_id FROM asset_documents UNION SELECT s.id,d.source_document_id FROM sources s JOIN asset_documents d ON d.id=s.parent) SELECT EXISTS(SELECT 1 FROM sources WHERE id=parent)",[],|r|r.get(0))?;
    if invalid_owner || cycle || document_cycle {
        bail!("LIBRARY_PACKAGE_INVALID");
    }
    super::ideas::validate_backup(&tx).context("LIBRARY_PACKAGE_INVALID")?;
    super::topics::validate_backup(&tx).context("LIBRARY_PACKAGE_INVALID")?;
    for note in &manifest.notes {
        validate_citations(&tx, &note.citations).context("LIBRARY_PACKAGE_INVALID")?;
        super::comparison::validate_note(&tx,note).context("LIBRARY_PACKAGE_INVALID")?;
        super::analysis::validate_note(&tx,note).context("LIBRARY_PACKAGE_INVALID")?;
        if let Some(super::NoteAnalysis { content: super::AnalysisContent::Technique { observations }, .. }) = &note.analysis {
            for row in observations {
                if let Some(id) = &row.keyframe_id {
                    if let Some(frame) = manifest.keyframes.iter().find(|f|&f.data.id==id) {
                        frame.data.validate().context("LIBRARY_PACKAGE_INVALID")?;
                        let actual = ((u128::from(frame.data.pts)*u128::from(frame.data.time_base_num)*1000+u128::from(frame.data.time_base_den)/2)/u128::from(frame.data.time_base_den)) as u64;
                        if frame.data.asset_id!=note.asset_id || frame.data.file_id!=row.file_id || !super::analysis::frame_within(&frame.data,row.start_ms,row.end_ms) || row.frame_actual_ms!=Some(actual) || manifest.clips.iter().any(|clip|clip.id==row.clip_id && clip.source!=frame.source) { bail!("LIBRARY_PACKAGE_INVALID"); }
                    }
                }
            }
        }
    }
    for clip in &manifest.clips {
        super::clips::validate_reference(&tx, &clip.value()).context("LIBRARY_PACKAGE_INVALID")?;
        if let Some(document_id) = &clip.document_id {
            let mut document = manifest
                .documents
                .iter()
                .find(|d| &d.id == document_id)
                .context("LIBRARY_PACKAGE_INVALID")?;
            while document.source.is_none() {
                let Some(parent) = &document.source_document_id else {
                    break;
                };
                document = manifest
                    .documents
                    .iter()
                    .find(|d| &d.id == parent)
                    .context("LIBRARY_PACKAGE_INVALID")?;
            }
            if document
                .source
                .as_ref()
                .is_some_and(|source| source != &clip.source)
            {
                bail!("LIBRARY_PACKAGE_INVALID");
            }
        }
        if clip.source.current_file {
            let file = manifest
                .files
                .iter()
                .find(|f| f.id == clip.file_id)
                .context("LIBRARY_PACKAGE_INVALID")?;
            if clip.source.size != file.size
                || file.mtime_ns.as_ref() != Some(&clip.source.mtime_ns)
            {
                bail!("LIBRARY_PACKAGE_INVALID");
            }
        }
    }
    let mut ids = HashSet::new();
    for frame in &manifest.keyframes {
        if !ids.insert(&frame.data.id) {
            bail!("LIBRARY_PACKAGE_INVALID");
        }
        if frame.source.current_file {
            let file = manifest
                .files
                .iter()
                .find(|f| f.id == frame.data.file_id)
                .context("LIBRARY_PACKAGE_INVALID")?;
            if frame.source.size != file.size
                || file.mtime_ns.as_ref() != Some(&frame.source.mtime_ns)
            {
                bail!("LIBRARY_PACKAGE_INVALID");
            }
        }
        let source = rebase_source(
            manifest,
            Path::new("restored"),
            None,
            &frame.source,
            Some(&frame.data.file_id),
        )?;
        super::keyframes::validate_frame_reference(&tx, &frame.data, &source)
            .context("LIBRARY_PACKAGE_INVALID")?;
    }
    tx.commit().context("LIBRARY_PACKAGE_INVALID")
}

pub struct PreparedRestore {
    manifest: Manifest,
    target: PathBuf,
    stage: tempfile::TempDir,
    covers: Vec<Vec<u8>>,
    frame_bodies: Vec<Vec<u8>>,
}
pub fn prepare_restore(
    package: &Path,
    target: &Path,
    notify: &mut TransferCallback<'_>,
) -> Result<PreparedRestore> {
    let root = package.canonicalize().context("IO_ERROR")?;
    let target = transfer::new_target(target)?;
    let manifest = read_manifest(&root)?;
    let mut progress = TransferProgress {
        phase: "validating",
        completed_files: 0,
        total_files: manifest.resources.len()
            + manifest.keyframes.len()
            + manifest.files.iter().filter(|f| f.media.is_some()).count(),
        bytes: 0,
    };
    notify(progress)?;
    validate_manifest(&manifest)?;
    let stage = tempfile::Builder::new()
        .prefix("yingqu-library-restore-")
        .tempdir_in(target.parent().context("IO_ERROR")?)
        .context("IO_ERROR")?;
    fs::create_dir(stage.path().join("media")).context("IO_ERROR")?;
    let mut covers = Vec::new();
    for (index, file) in manifest.files.iter().enumerate() {
        if let Some(payload) = &file.media {
            progress.phase = "copying";
            let path = transfer::payload_path(&root, &payload.path)?;
            let (size, hash) = transfer::copy_hash(
                &mut File::open(path)
                    .context("IO_ERROR")?
                    .take(payload.size.saturating_add(1)),
                Some(
                    &stage
                        .path()
                        .join(transfer::media_name(index, &file.filename)),
                ),
                &mut progress,
                notify,
            )?;
            if size != payload.size || hash != payload.sha256 {
                bail!("LIBRARY_PACKAGE_CHECKSUM");
            }
            progress.completed_files += 1;
        }
    }
    for (payload, limit) in manifest
        .resources
        .iter()
        .map(|r| (r.payload.as_ref(), 16 * 1024 * 1024))
        .chain(
            manifest
                .keyframes
                .iter()
                .map(|r| (r.payload.as_ref(), super::MAX_FRAME_BYTES as u64)),
        )
    {
        let payload = payload.context("LIBRARY_PACKAGE_INVALID")?;
        if payload.size > limit {
            bail!("LIBRARY_PACKAGE_TOO_LARGE");
        }
        let path = transfer::payload_path(&root, &payload.path)?;
        let mut body = Vec::new();
        File::open(path)
            .context("IO_ERROR")?
            .take(payload.size + 1)
            .read_to_end(&mut body)
            .context("IO_ERROR")?;
        if body.len() as u64 != payload.size
            || format!("{:x}", Sha256::digest(&body)) != payload.sha256
        {
            bail!("LIBRARY_PACKAGE_CHECKSUM");
        }
        progress.bytes += body.len() as u64;
        progress.completed_files += 1;
        notify(progress)?;
        covers.push(body);
    }
    let frame_bodies = covers.split_off(manifest.resources.len());
    for (frame, body) in manifest.keyframes.iter().zip(&frame_bodies) {
        super::validate_frame_image(body, frame.data.width, frame.data.height)
            .context("LIBRARY_PACKAGE_INVALID")?;
    }
    progress.phase = "committing";
    notify(progress)?;
    Ok(PreparedRestore {
        manifest,
        target,
        stage,
        covers,
        frame_bodies,
    })
}

pub fn require_empty_library(db: &Connection) -> Result<()> {
    for table in [
        "library_assets",
        "library_ideas",
        "idea_references",
        "research_topics",
        "research_topic_members",
        "asset_files",
        "library_categories",
        "library_tags",
        "asset_clips",
        "asset_keyframes",
        "transcript_segments",
        "transcript_timings",
        "asset_documents",
        "asset_notes",
        "asset_categories",
        "asset_tags",
        "asset_resources",
        "asset_visits",
        "asset_transcriptions",
        "asset_task_links",
    ] {
        if db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
            r.get::<_, bool>(0)
        })? {
            bail!("LIBRARY_NOT_EMPTY");
        }
    }
    Ok(())
}

pub fn commit_restore(db: &mut Connection, prepared: PreparedRestore) -> Result<RestoreReport> {
    require_empty_library(db)?;
    let PreparedRestore {
        manifest,
        target,
        stage,
        covers,
        frame_bodies,
    } = prepared;
    let tx = db.transaction().context("DATABASE_ERROR")?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
    insert_manifest(&tx, &manifest, &target, Some(stage.path())).context("DATABASE_ERROR")?;
    for (resource, body) in manifest.resources.iter().zip(covers) {
        tx.execute(
            "INSERT INTO asset_resources VALUES(?1,'cover',?2)",
            params![resource.asset_id, body],
        )
        .context("DATABASE_ERROR")?;
    }
    for (frame, body) in manifest.keyframes.iter().zip(frame_bodies) {
        let source = rebase_source(
            &manifest,
            &target,
            Some(stage.path()),
            &frame.source,
            Some(&frame.data.file_id),
        )?;
        super::keyframes::insert_frame(&tx, &frame.data, &source, &body)
            .context("LIBRARY_PACKAGE_INVALID")?;
    }
    transfer::publish(stage.path(), &target)?;
    if let Err(error) = tx.commit() {
        // Only the just-published, formerly nonexistent task-owned directory
        // is removed. It must still resolve to its recorded absolute target.
        if target
            .canonicalize()
            .context("LIBRARY_RESTORE_CLEANUP_FAILED")?
            != target
        {
            bail!("LIBRARY_RESTORE_CLEANUP_FAILED");
        }
        fs::remove_dir_all(&target).context("LIBRARY_RESTORE_CLEANUP_FAILED")?;
        return Err(error).context("DATABASE_ERROR");
    }
    Ok(RestoreReport {
        ideas: manifest.ideas.len(),
        topics: manifest.research_topics.len(),
        assets: manifest.assets.len(),
        documents: manifest.documents.len(),
        notes: manifest.notes.len(),
        missing_media: manifest.files.iter().filter(|f| f.media.is_none()).count(),
    })
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
