use super::{require_asset, unsigned, Citation, Note, NoteOrigin};
use crate::desktop_core::transcription::Source;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct DocumentSummary {
    pub id: String,
    pub asset_id: String,
    pub file_id: Option<String>,
    pub kind: String,
    pub version: u64,
    pub source_document_id: Option<String>,
    pub characters: u64,
    pub model_name: Option<String>,
    pub backend: Option<String>,
    pub created_at: u64,
}
fn document_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentSummary> {
    Ok(DocumentSummary {
        id: row.get(0)?,
        asset_id: row.get(1)?,
        file_id: row.get(2)?,
        kind: row.get(3)?,
        version: unsigned(row, 4)?,
        source_document_id: row.get(5)?,
        characters: unsigned(row, 6)?,
        model_name: row.get(7)?,
        backend: row.get(8)?,
        created_at: unsigned(row, 9)?,
    })
}
const DOCUMENT_COLUMNS:&str="id,asset_id,file_id,kind,version,source_document_id,length(text),model_name,backend,created_at";
pub fn document_summaries(db: &Connection, asset_id: &str) -> Result<Vec<DocumentSummary>> {
    require_asset(db, asset_id)?;
    let mut query=db.prepare(&format!("SELECT {DOCUMENT_COLUMNS} FROM asset_documents WHERE asset_id=?1 ORDER BY kind,version DESC"))?;
    let rows = query
        .query_map([asset_id], document_summary)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[derive(Serialize)]
pub struct DocumentContent {
    pub timing: Option<super::TranscriptTiming>,
    pub summary: DocumentSummary,
    pub text: String,
    pub source: Option<Source>,
    pub source_changed: Option<bool>,
    pub model_revision: Option<String>,
    pub pipeline_revision: Option<String>,
}
pub fn read_document(db: &Connection, asset_id: &str, id: &str) -> Result<DocumentContent> {
    require_asset(db, asset_id)?;
    let (summary,text,source,model_revision,pipeline_revision)=db.query_row(&format!("SELECT {DOCUMENT_COLUMNS},text,source_json,model_revision,pipeline_revision FROM asset_documents WHERE asset_id=?1 AND id=?2"),params![asset_id,id],|r|Ok((document_summary(r)?,r.get::<_,String>(10)?,r.get::<_,Option<String>>(11)?,r.get::<_,Option<String>>(12)?,r.get::<_,Option<String>>(13)?))).optional()?.context("DOCUMENT_NOT_FOUND")?;
    let source: Option<Source> = source.map(|json| serde_json::from_str(&json)).transpose()?;
    let source_changed = match (&source, &summary.file_id) {
        (Some(source), Some(file_id)) => {
            let path = db
                .query_row(
                    "SELECT path FROM asset_files WHERE id=?1 AND asset_id=?2",
                    params![file_id, asset_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            Some(
                path.and_then(|p| Source::read(&PathBuf::from(p)).ok())
                    .as_ref()
                    != Some(source),
            )
        }
        (Some(_), None) => Some(true),
        _ => None,
    };
    Ok(DocumentContent {
        timing: super::timing::read(db, id)?,
        summary,
        text,
        source,
        source_changed,
        model_revision,
        pipeline_revision,
    })
}

#[derive(Debug, Serialize)]
pub struct NoteSummary {
    pub id: String,
    pub asset_id: String,
    pub kind: String,
    pub origin: NoteOrigin,
    pub model_name: Option<String>,
    pub revision: u64,
    pub characters: u64,
    pub citation_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
pub fn note_summaries(db: &Connection, asset_id: &str) -> Result<Vec<NoteSummary>> {
    require_asset(db, asset_id)?;
    let mut query=db.prepare("SELECT id,kind,origin,model_name,revision,length(body),json_array_length(citations_json),created_at,updated_at FROM asset_notes WHERE asset_id=?1 ORDER BY created_at,id")?;
    let rows = query.query_map([asset_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            unsigned(r, 4)?,
            unsigned(r, 5)?,
            unsigned(r, 6)?,
            unsigned(r, 7)?,
            unsigned(r, 8)?,
        ))
    })?;
    rows.map(|row| {
        let (
            id,
            kind,
            origin,
            model_name,
            revision,
            characters,
            citation_count,
            created_at,
            updated_at,
        ) = row?;
        Ok(NoteSummary {
            id,
            asset_id: asset_id.into(),
            kind,
            origin: serde_json::from_value(serde_json::Value::String(origin))?,
            model_name,
            revision,
            characters,
            citation_count,
            created_at,
            updated_at,
        })
    })
    .collect()
}
pub fn read_note(db: &Connection, asset_id: &str, id: &str) -> Result<Note> {
    require_asset(db, asset_id)?;
    let(kind,body,origin,model_name,citations,revision,created_at,updated_at)=db.query_row("SELECT kind,body,origin,model_name,citations_json,revision,created_at,updated_at FROM asset_notes WHERE asset_id=?1 AND id=?2",params![asset_id,id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,unsigned(r,5)?,unsigned(r,6)?,unsigned(r,7)?))).optional()?.context("NOTE_NOT_FOUND")?;
    let citations: Vec<Citation> = serde_json::from_str(&citations)?;
    Ok(Note {
        comparison: super::comparison::read(db, id)?,
        analysis: super::analysis::read(db, id)?,
        id: id.into(),
        asset_id: asset_id.into(),
        kind,
        body,
        origin: serde_json::from_value(serde_json::Value::String(origin))?,
        model_name,
        citations,
        revision,
        created_at,
        updated_at,
    })
}

#[derive(Serialize)]
pub struct AssetDetail {
    pub asset: super::Asset,
    pub files: Vec<super::AssetFile>,
    pub category_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub documents: Vec<DocumentSummary>,
    pub notes: Vec<NoteSummary>,
}
pub fn asset_detail(db: &Connection, id: &str) -> Result<AssetDetail> {
    let asset = super::asset(db, id)?;
    let ids = |sql: &str| -> Result<Vec<String>> {
        let mut query = db.prepare(sql)?;
        let rows = query
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    };
    Ok(AssetDetail {
        asset,
        files: super::asset_files(db, id)?,
        category_ids: ids(
            "SELECT category_id FROM asset_categories WHERE asset_id=?1 ORDER BY category_id",
        )?,
        tag_ids: ids("SELECT tag_id FROM asset_tags WHERE asset_id=?1 ORDER BY tag_id")?,
        documents: document_summaries(db, id)?,
        notes: note_summaries(db, id)?,
    })
}
