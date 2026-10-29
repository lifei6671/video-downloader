//! P1B public read views. Existing library services remain the domain authority.
mod types;
mod media;
pub use media::{AudioInput, AudioExcerpt, ClipInput, Clips, KeyframesInput, Keyframes};
pub use types::*;

use crate::library;
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadError {
    InvalidInput,
    InvalidCursor,
    AssetNotFound,
    DocumentNotFound,
    NoteNotFound,
    ClipNotFound,
    FileNotFound,
    MediaUnsupported,
    CategoryNotFound,
    TagNotFound,
    VersionConflict,
    RevisionConflict,
    OutOfRange,
    CoreReadFailed,
    ServiceUnavailable,
    FileMissing,
    AudioSourceChanged,
    AudioBusy,
    AudioNoTrack,
    AudioRangeUnavailable,
    AudioFailed,
    AudioTimeout,
    Canceled,
    FfmpegUnavailable,
}
impl ReadError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidCursor => "INVALID_CURSOR",
            Self::AssetNotFound => "ASSET_NOT_FOUND",
            Self::DocumentNotFound => "DOCUMENT_NOT_FOUND",
            Self::NoteNotFound => "NOTE_NOT_FOUND",
            Self::ClipNotFound => "CLIP_NOT_FOUND",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::MediaUnsupported => "MEDIA_UNSUPPORTED",
            Self::CategoryNotFound => "CATEGORY_NOT_FOUND",
            Self::TagNotFound => "TAG_NOT_FOUND",
            Self::VersionConflict => "VERSION_CONFLICT",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::CoreReadFailed => "CORE_READ_FAILED",
            Self::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            Self::FileMissing => "FILE_MISSING",
            Self::AudioSourceChanged => "AUDIO_SOURCE_CHANGED",
            Self::AudioBusy => "AUDIO_BUSY",
            Self::AudioNoTrack => "AUDIO_NO_TRACK",
            Self::AudioRangeUnavailable => "AUDIO_RANGE_UNAVAILABLE",
            Self::AudioFailed => "AUDIO_FAILED",
            Self::AudioTimeout => "AUDIO_TIMEOUT",
            Self::Canceled => "CANCELED",
            Self::FfmpegUnavailable => "FFMPEG_UNAVAILABLE",
        }
    }
    pub(crate) fn from_domain(error: anyhow::Error) -> Self {
        // Only propagate known domain classes, never arbitrary DB/IO diagnostics.
        serde_json::from_value(Value::String(error.to_string())).unwrap_or(Self::CoreReadFailed)
    }
}

pub(crate) fn dispatch(
    db: &Connection,
    method: &str,
    params: Value,
) -> std::result::Result<Value, ReadError> {
    fn input<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
        serde_json::from_value(value).context("INVALID_INPUT")
    }
    let result = (|| -> Result<Value> {
        Ok(match method {
            "mcp_search_assets" => serde_json::to_value(search_assets(db, input(params)?)?)?,
            "mcp_get_asset" => serde_json::to_value(get_asset(db, input(params)?)?)?,
            "mcp_read_transcript" => serde_json::to_value(read_transcript(db, input(params)?)?)?,
            "mcp_list_categories" => serde_json::to_value(list_categories(db, input(params)?)?)?,
            "mcp_list_tags" => serde_json::to_value(list_tags(db, input(params)?)?)?,
            "mcp_list_notes" => serde_json::to_value(list_notes(db, input(params)?)?)?,
            "mcp_get_clip" => serde_json::to_value(media::clips(db, input(params)?)?)?,
            "mcp_get_keyframes" => serde_json::to_value(media::keyframes(db, input(params)?)?)?,
            "mcp_read_note" => serde_json::to_value(read_note(db, input(params)?)?)?,
            _ => bail!("INVALID_INPUT"),
        })
    })();
    result.map_err(ReadError::from_domain)
}

pub(crate) fn id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 {
        bail!("INVALID_INPUT");
    }
    Ok(())
}
pub(crate) fn require_collected(db: &Connection, asset_id: &str) -> Result<()> {
    id(asset_id)?;
    if library::asset(db, asset_id)?.archived_at.is_some() { bail!("ASSET_NOT_FOUND"); }
    Ok(())
}
fn require_note_collected(db: &Connection, note: &library::Note) -> Result<()> {
    require_collected(db, &note.asset_id)?;
    for citation in &note.citations { require_collected(db, &citation.asset_id)?; }
    if let Some(comparison) = &note.comparison {
        for asset in &comparison.asset_ids { require_collected(db, asset)?; }
        for source in comparison.points.iter().flat_map(|point| &point.sources) { require_collected(db, &source.pin.asset_id)?; }
    }
    Ok(())
}
fn excluded_note_hits(db: &Connection, query: &str) -> Result<Vec<String>> {
    if query.trim().is_empty() { return Ok(vec![]); }
    let mut statement = db.prepare("SELECT n.id,n.asset_id FROM asset_notes n JOIN library_assets a ON a.id=n.asset_id WHERE a.archived_at IS NULL AND instr(lower(n.body),lower(?1))>0")?;
    let candidates = statement.query_map([query.trim()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut excluded = vec![];
    for (id, asset) in candidates {
        let note = library::read_note(db, &asset, &id)?;
        match require_note_collected(db, &note) {
            Ok(()) => {},
            Err(error) if error.to_string() == "ASSET_NOT_FOUND" => excluded.push(id),
            Err(error) => return Err(error),
        }
    }
    Ok(excluded)
}
fn limit(value: u32) -> Result<()> {
    if !(1..=100).contains(&value) {
        bail!("INVALID_INPUT");
    }
    Ok(())
}
fn query(value: &str) -> Result<()> {
    if value.chars().count() > 512 {
        bail!("INVALID_INPUT");
    }
    Ok(())
}
fn page<T>(
    mut items: Vec<T>,
    cursor: Option<&str>,
    count: u32,
    key: impl Fn(&T) -> &str,
) -> Result<Page<T>> {
    limit(count)?;
    items.sort_by(|a, b| key(a).cmp(key(b)));
    if let Some(cursor) = cursor {
        id(cursor)?;
        if !items.iter().any(|item| key(item) == cursor) {
            bail!("INVALID_CURSOR");
        }
    }
    let total = items.len() as u64;
    let mut selected: Vec<T> = items
        .into_iter()
        .filter(|item| cursor.is_none_or(|cursor| key(item) > cursor))
        .take(count as usize + 1)
        .collect();
    let more = selected.len() > count as usize;
    if more {
        selected.pop();
    }
    let next_cursor = if more {
        selected.last().map(|item| key(item).to_owned())
    } else {
        None
    };
    Ok(Page {
        items: selected,
        next_cursor,
        total,
    })
}

fn counts(db: &Connection, categories: bool) -> Result<HashMap<String, u64>> {
    let sql = if categories {
        "SELECT c.category_id,count(*) FROM asset_categories c JOIN library_assets a ON a.id=c.asset_id WHERE a.archived_at IS NULL GROUP BY c.category_id"
    } else {
        "SELECT t.tag_id,count(*) FROM asset_tags t JOIN library_assets a ON a.id=t.asset_id WHERE a.archived_at IS NULL GROUP BY t.tag_id"
    };
    let mut statement = db.prepare(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, count)| Ok((id, u64::try_from(count)?)))
        .collect()
}
fn list_categories(db: &Connection, request: ListInput) -> Result<Categories> {
    query(&request.query)?;
    let counts = counts(db, true)?;
    let items = library::categories(db)?
        .into_iter()
        .filter(|item| item.name.contains(request.query.trim()))
        .map(|item| Category {
            asset_count: counts.get(&item.id).copied().unwrap_or(0),
            id: item.id,
            parent_id: None,
            name: item.name,
            position: item.position,
        })
        .collect();
    let page = page(items, request.cursor.as_deref(), request.limit, |item| {
        &item.id
    })?;
    Ok(Categories {
        categories: page.items,
        next_cursor: page.next_cursor,
        total: page.total,
    })
}
fn list_tags(db: &Connection, request: ListInput) -> Result<Tags> {
    query(&request.query)?;
    let counts = counts(db, false)?;
    let items = library::tags(db)?
        .into_iter()
        .filter(|item| item.name.contains(request.query.trim()))
        .map(|item| Tag {
            asset_count: counts.get(&item.id).copied().unwrap_or(0),
            id: item.id,
            name: item.name,
        })
        .collect();
    let page = page(items, request.cursor.as_deref(), request.limit, |item| {
        &item.id
    })?;
    Ok(Tags {
        tags: page.items,
        next_cursor: page.next_cursor,
        total: page.total,
    })
}
fn asset(value: library::Asset) -> Asset {
    Asset {
        id: value.id,
        title: value.title,
        author: value.author,
        platform: value.platform,
        video_id: value.video_id,
        created_at: value.created_at,
        updated_at: value.updated_at,
        archived: value.archived_at.is_some(),
    }
}
fn document(value: library::DocumentSummary) -> DocumentSummary {
    DocumentSummary {
        id: value.id,
        asset_id: value.asset_id,
        file_id: value.file_id,
        kind: value.kind,
        version: value.version,
        source_document_id: value.source_document_id,
        created_at: value.created_at,
    }
}
fn origin(value: library::NoteOrigin) -> String {
    match value {
        library::NoteOrigin::Human => "human",
        library::NoteOrigin::Ai => "ai",
    }
    .into()
}
fn note(value: library::NoteSummary) -> NoteSummary {
    NoteSummary {
        id: value.id,
        asset_id: value.asset_id,
        kind: value.kind,
        origin: origin(value.origin),
        revision: value.revision,
        citation_count: value.citation_count,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}
fn search_assets(db: &Connection, request: SearchInput) -> Result<SearchPage> {
    if request.archived { bail!("INVALID_INPUT"); }
    limit(request.limit)?;
    if request.category_ids.len() > 100 || request.tag_ids.len() > 100 {
        bail!("INVALID_INPUT");
    }
    for value in request.category_ids.iter().chain(request.tag_ids.iter()) {
        id(value)?;
    }
    if let Some(cursor) = &request.cursor {
        id(&cursor.id)?;
        let exists = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM library_assets WHERE id=?1 AND created_at=?2 AND archived_at IS NULL)",
            rusqlite::params![
                cursor.id,
                i64::try_from(cursor.created_at).context("INVALID_CURSOR")?
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            bail!("INVALID_CURSOR");
        }
    }
    let excluded = excluded_note_hits(db, &request.query)?;
    let page = library::search_assets_excluding_notes(
        db,
        &library::SearchRequest {
            query: request.query,
            category_ids: request.category_ids,
            tag_ids: request.tag_ids,
            platform: request.platform,
            archived: request.archived,
            uncategorized: request.uncategorized,
            unorganized: request.unorganized,
            file_state: match request.file_state {
                FileFilter::All => library::FileFilter::All,
                FileFilter::Available => library::FileFilter::Available,
                FileFilter::Missing => library::FileFilter::Missing,
                FileFilter::Changed => library::FileFilter::Changed,
            },
            cursor: request.cursor.map(|cursor| library::SearchCursor {
                id: cursor.id,
                created_at: cursor.created_at,
            }),
            limit: request.limit,
        },
        &excluded,
    )?;
    let items = page
        .items
        .into_iter()
        .map(|item| -> Result<SearchItem> {
            let hit = item
                .hit
                .map(|hit| -> Result<SearchHit> {
                    let note_revision = hit
                        .note_id
                        .as_ref()
                        .map(|id| {
                            db.query_row(
                                "SELECT revision FROM asset_notes WHERE id=?1",
                                [id],
                                |row| row.get::<_, i64>(0),
                            )
                        })
                        .transpose()?
                        .map(u64::try_from)
                        .transpose()?;
                    Ok(SearchHit {
                        field: hit.field,
                        excerpt: hit.excerpt,
                        document_id: hit.document_id,
                        document_version: hit.document_version,
                        note_id: hit.note_id,
                        note_revision,
                    })
                })
                .transpose()?;
            Ok(SearchItem {
                asset: asset(item.asset),
                hit,
                files: FileCounts {
                    total: item.files.total as u64,
                    available: item.files.available as u64,
                    changed: item.files.changed as u64,
                },
            })
        })
        .collect::<Result<_>>()?;
    Ok(SearchPage {
        items,
        next_cursor: page.next_cursor.map(|cursor| SearchCursor {
            id: cursor.id,
            created_at: cursor.created_at,
        }),
    })
}
fn notes_page(
    items: Vec<library::NoteSummary>,
    cursor: Option<&str>,
    count: u32,
) -> Result<NotesPage> {
    let page = page(
        items.into_iter().map(note).collect(),
        cursor,
        count,
        |item| &item.id,
    )?;
    Ok(NotesPage {
        notes: page.items,
        next_cursor: page.next_cursor,
        total: page.total,
    })
}
fn get_asset(db: &Connection, request: AssetInput) -> Result<AssetDetail> {
    require_collected(db, &request.asset_id)?;
    limit(request.limit)?;
    let detail = library::asset_detail(db, &request.asset_id)?;
    Ok(AssetDetail {
        asset: asset(detail.asset),
        files: detail
            .files
            .into_iter()
            .map(|file| FileSummary {
                id: file.id,
                size: file.size,
                duration_ms: file.duration_ms,
                width: file.width,
                height: file.height,
                missing: file.missing,
                changed: file.changed,
            })
            .collect(),
        category_ids: detail.category_ids,
        tag_ids: detail.tag_ids,
        documents: page(
            detail.documents.into_iter().map(document).collect(),
            request.document_cursor.as_deref(),
            request.limit,
            |item| &item.id,
        )?,
        notes: notes_page(detail.notes, None, request.limit)?,
    })
}
fn text_slice(text: &str, offset: u64, length: u32) -> Result<TextSlice> {
    if !(1..=16000).contains(&length) {
        bail!("INVALID_INPUT");
    }
    let total_characters = text.chars().count() as u64;
    if offset > total_characters {
        bail!("OUT_OF_RANGE");
    }
    let text: String = text
        .chars()
        .skip(offset as usize)
        .take(length as usize)
        .collect();
    let length = text.chars().count() as u32;
    let end = offset + u64::from(length);
    Ok(TextSlice {
        text,
        total_characters,
        offset,
        length,
        next_offset: (end < total_characters).then_some(end),
        eof: end == total_characters,
    })
}
fn document_reference(summary: &DocumentSummary, offset: u64, length: u64) -> DocumentReference {
    DocumentReference {
        asset_id: summary.asset_id.clone(),
        document_id: summary.id.clone(),
        document_version: summary.version,
        kind: summary.kind.clone(),
        offset,
        length,
    }
}
fn read_transcript(db: &Connection, request: TranscriptInput) -> Result<Transcript> {
    require_collected(db, &request.asset_id)?;
    let documents = library::document_summaries(db, &request.asset_id)?;
    let selected = match request.document_id.as_deref() {
        Some(value) => {
            id(value)?;
            documents.into_iter().find(|document| document.id == value)
        }
        None => documents
            .into_iter()
            .max_by_key(|document| (document.kind == "edited", document.version)),
    }
    .context("DOCUMENT_NOT_FOUND")?;
    if request
        .version
        .is_some_and(|version| version != selected.version)
    {
        bail!("VERSION_CONFLICT");
    }
    let document = document(selected);
    let text = library::read_document(db, &request.asset_id, &document.id)?.text;
    let content = text_slice(&text, request.offset, request.length)?;
    let reference = document_reference(&document, content.offset, u64::from(content.length));
    Ok(Transcript {
        document,
        content,
        reference,
    })
}
fn list_notes(db: &Connection, request: NotesInput) -> Result<NotesPage> {
    require_collected(db, &request.asset_id)?;
    notes_page(
        library::note_summaries(db, &request.asset_id)?,
        request.cursor.as_deref(),
        request.limit,
    )
}
fn read_note(db: &Connection, request: NoteInput) -> Result<Note> {
    id(&request.note_id)?;
    limit(request.citation_limit)?;
    let asset_id: String = db
        .query_row(
            "SELECT asset_id FROM asset_notes WHERE id=?1",
            [&request.note_id],
            |row| row.get(0),
        )
        .optional()?
        .context("NOTE_NOT_FOUND")?;
    if let Some(expected) = &request.asset_id {
        id(expected)?;
        if *expected != asset_id {
            bail!("NOTE_NOT_FOUND");
        }
    }
    require_collected(db, &asset_id)?;
    let value = library::read_note(db, &asset_id, &request.note_id)?;
    require_note_collected(db, &value)?;
    if request
        .revision
        .is_some_and(|revision| revision != value.revision)
    {
        bail!("REVISION_CONFLICT");
    }
    let content = text_slice(&value.body, request.offset, request.length)?;
    let citation_total = value.citations.len() as u64;
    if request.citation_offset > citation_total {
        bail!("OUT_OF_RANGE");
    }
    let citations = value
        .citations
        .iter()
        .skip(request.citation_offset as usize)
        .take(request.citation_limit as usize)
        .map(|citation| -> Result<Citation> {
            let source = library::read_document(db, &citation.asset_id, &citation.document_id)?;
            let start = source
                .text
                .find(&citation.quote)
                .context("CORE_READ_FAILED")?;
            let offset = source.text[..start].chars().count() as u64;
            let length = citation.quote.chars().count() as u64;
            let quote_excerpt = citation.quote.chars().take(120).collect();
            Ok(Citation {
                reference: document_reference(&document(source.summary), offset, length),
                quote_excerpt,
                quote_truncated: length > 120,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let next = request.citation_offset + citations.len() as u64;
    Ok(Note {
        reference: NoteReference {
            asset_id: value.asset_id.clone(),
            note_id: value.id.clone(),
            revision: value.revision,
            offset: content.offset,
            length: u64::from(content.length),
        },
        summary: NoteSummary {
            id: value.id,
            asset_id: value.asset_id,
            kind: value.kind,
            origin: origin(value.origin),
            revision: value.revision,
            citation_count: citation_total,
            created_at: value.created_at,
            updated_at: value.updated_at,
        },
        content,
        citations,
        citation_offset: request.citation_offset,
        citation_total,
        next_citation_offset: (next < citation_total).then_some(next),
    })
}

#[cfg(test)]
mod tests;
