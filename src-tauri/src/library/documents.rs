#[cfg(feature = "mcp")]
use rmcp::schemars;
use super::{now, require_asset, sql_integer, unsigned};
use crate::desktop_core::transcription::TranscriptResult;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_TEXT: usize = 16 * 1024 * 1024;

#[derive(Serialize)]
pub struct Document {
    pub id: String,
    pub asset_id: String,
    pub kind: String,
    pub text: String,
    pub version: u64,
    pub source_document_id: Option<String>,
    pub recognition: Option<TranscriptResult>,
    pub created_at: u64,
}

pub fn documents(db: &Connection, asset_id: &str) -> Result<Vec<Document>> {
    require_asset(db, asset_id)?;
    let mut query = db.prepare("SELECT id,kind,text,version,source_document_id,source_json,model_revision,model_name,pipeline_revision,backend,created_at FROM asset_documents WHERE asset_id=?1 ORDER BY kind,version DESC")?;
    let rows = query.query_map([asset_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            unsigned(r, 3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            unsigned(r, 10)?,
        ))
    })?;
    rows.map(|row| {
        let (
            id,
            kind,
            text,
            version,
            source_document_id,
            source_json,
            model_revision,
            model_name,
            pipeline_revision,
            backend,
            created_at,
        ) = row?;
        let recognition = if kind == "recognized" {
            Some(TranscriptResult { timing: super::timing::read(db, &id)?,
                text: text.clone(),
                source: serde_json::from_str(&source_json.context("DATABASE_ERROR")?)?,
                model_revision: model_revision.context("DATABASE_ERROR")?,
                model_name,
                pipeline_revision: pipeline_revision.context("DATABASE_ERROR")?,
                backend,
                completed_at: created_at,
            })
        } else {
            None
        };
        Ok(Document {
            id,
            asset_id: asset_id.into(),
            kind,
            text,
            version,
            source_document_id,
            recognition,
            created_at,
        })
    })
    .collect()
}

/// The caller includes this write in the operation/asset transaction. The key
/// makes startup backfill and retry of an already committed result idempotent.
pub fn save_recognized(
    db: &Connection,
    asset_id: &str,
    file_id: Option<&str>,
    result: &TranscriptResult,
) -> Result<String> {
    require_asset(db, asset_id)?;
    validate_text(&result.text)?;
    if let Some(timing) = &result.timing {
        timing.validate(&result.text)?;
        if file_id.is_none() { bail!("FILE_NOT_FOUND"); }
    }
    if let Some(file_id) = file_id {
        if !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_files WHERE id=?1 AND asset_id=?2)",
            params![file_id, asset_id],
            |r| r.get::<_, bool>(0),
        )? {
            bail!("FILE_NOT_FOUND");
        }
    }
    let key = format!(
        "{asset_id}:{:x}",
        Sha256::digest(serde_json::to_vec(result)?)
    );
    if let Some(id) = db
        .query_row(
            "SELECT id FROM asset_documents WHERE recognition_key=?1",
            [&key],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    if result.timing.is_some() {
        let path: String = db.query_row("SELECT path FROM asset_files WHERE id=?1", [file_id.unwrap()], |r|r.get(0))?;
        if std::path::Path::new(&path) != result.source.path { bail!("TRANSCRIPTION_SOURCE_CHANGED"); }
    }
    let id = Uuid::new_v4().to_string();
    db.execute("INSERT INTO asset_documents(id,asset_id,file_id,kind,text,version,revision,recognition_key,source_json,model_revision,model_name,pipeline_revision,backend,created_at) SELECT ?1,?2,?3,'recognized',?4,COALESCE(MAX(version),0)+1,1,?5,?6,?7,?8,?9,?10,?11 FROM asset_documents WHERE asset_id=?2 AND kind='recognized'",
        params![id,asset_id,file_id,result.text,key,serde_json::to_string(&result.source)?,result.model_revision,result.model_name,result.pipeline_revision,result.backend,sql_integer(result.completed_at)?])?;
    if let Some(timing) = &result.timing { super::timing::store(db, &id, timing)?; }
    Ok(id)
}

pub fn latest_recognition(db: &Connection, file_id: &str) -> Result<Option<TranscriptResult>> {
    let stored = db.query_row(
        "SELECT text,source_json,model_revision,model_name,pipeline_revision,backend,created_at,id FROM asset_documents WHERE file_id=?1 AND kind='recognized' ORDER BY version DESC LIMIT 1",
        [file_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,Option<String>>(3)?, r.get::<_,String>(4)?, r.get::<_,Option<String>>(5)?, unsigned(r,6)?,r.get::<_,String>(7)?)),
    ).optional()?;
    stored
        .map(
            |(
                text,
                source,
                model_revision,
                model_name,
                pipeline_revision,
                backend,
                completed_at,
                id,
            )| {
                Ok(TranscriptResult { timing: super::timing::read(db, &id)?,
                    text,
                    source: serde_json::from_str(&source)?,
                    model_revision,
                    model_name,
                    pipeline_revision,
                    backend,
                    completed_at,
                })
            },
        )
        .transpose()
}

/// Each save creates an immutable edition; citations to older editions survive.
pub fn save_edited(
    db: &Connection,
    asset_id: &str,
    text: &str,
    expected_version: u64,
    source_document_id: Option<&str>,
) -> Result<String> {
    require_asset(db, asset_id)?;
    validate_text(text)?;
    if let Some(source_id) = source_document_id {
        if !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_documents WHERE id=?1 AND asset_id=?2)",
            params![source_id, asset_id],
            |r| r.get::<_, bool>(0),
        )? {
            bail!("DOCUMENT_NOT_FOUND");
        }
    }
    let id = Uuid::new_v4().to_string();
    let changed = db.execute("INSERT INTO asset_documents(id,asset_id,kind,text,version,revision,source_document_id,created_at) SELECT ?1,?2,'edited',?3,?4+1,?4+1,?5,?6 WHERE COALESCE((SELECT MAX(version) FROM asset_documents WHERE asset_id=?2 AND kind='edited'),0)=?4",
        params![id,asset_id,text,sql_integer(expected_version)?,source_document_id,sql_integer(now())?])?;
    if changed == 0 {
        bail!("DOCUMENT_CONFLICT");
    }
    Ok(id)
}

pub(super) fn validate_text(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.len() > MAX_TEXT {
        bail!("INVALID_INPUT");
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub asset_id: String,
    pub document_id: String,
    pub quote: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteOrigin {
    Human,
    Ai,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Note {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<super::NoteComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<super::NoteAnalysis>,
    pub id: String,
    pub asset_id: String,
    pub kind: String,
    pub body: String,
    pub origin: NoteOrigin,
    pub model_name: Option<String>,
    pub citations: Vec<Citation>,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct NewNote<'a> {
    pub kind: &'a str,
    pub body: &'a str,
    pub origin: NoteOrigin,
    pub model_name: Option<&'a str>,
    pub citations: &'a [Citation],
}

pub(super) fn validate_citations(db: &Connection, citations: &[Citation]) -> Result<()> {
    for citation in citations {
        if citation.quote.is_empty() || !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_documents WHERE id=?1 AND asset_id=?2 AND instr(text,?3)>0)",
            params![citation.document_id,citation.asset_id,citation.quote], |r| r.get::<_,bool>(0),
        )? { bail!("INVALID_CITATION"); }
    }
    Ok(())
}

pub fn create_note(db: &Connection, asset_id: &str, note: NewNote<'_>) -> Result<String> {
    require_asset(db, asset_id)?;
    validate_text(note.body)?;
    super::name(note.kind)?;
    validate_citations(db, note.citations)?;
    let id = Uuid::new_v4().to_string();
    let origin = match note.origin {
        NoteOrigin::Human => "human",
        NoteOrigin::Ai => "ai",
    };
    db.execute("INSERT INTO asset_notes(id,asset_id,kind,body,origin,model_name,citations_json,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?8)",
        params![id,asset_id,note.kind.trim(),note.body,origin,note.model_name,serde_json::to_string(note.citations)?,sql_integer(now())?])?;
    Ok(id)
}

pub fn notes(db: &Connection, asset_id: &str) -> Result<Vec<Note>> {
    require_asset(db, asset_id)?;
    let mut query = db.prepare("SELECT id,kind,body,origin,model_name,citations_json,revision,created_at,updated_at FROM asset_notes WHERE asset_id=?1 ORDER BY created_at,id")?;
    let rows = query.query_map([asset_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            unsigned(r, 6)?,
            unsigned(r, 7)?,
            unsigned(r, 8)?,
        ))
    })?;
    rows.map(|row| {
        let (id, kind, body, origin, model_name, citations, revision, created_at, updated_at) =
            row?;
        let origin = match origin.as_str() {
            "human" => NoteOrigin::Human,
            "ai" => NoteOrigin::Ai,
            _ => bail!("DATABASE_ERROR"),
        };
        Ok(Note {
            comparison: super::comparison::read(db, &id)?,
            analysis: super::analysis::read(db, &id)?,
            id,
            asset_id: asset_id.into(),
            kind,
            body,
            origin,
            model_name,
            citations: serde_json::from_str(&citations)?,
            revision,
            created_at,
            updated_at,
        })
    })
    .collect()
}

pub fn edit_note(db: &Connection, id: &str, body: &str, expected_revision: u64) -> Result<()> {
    validate_text(body)?;
    if super::comparison::read(db,id)?.is_some() { bail!("COMPARISON_REQUIRED"); }
    if super::analysis::read(db,id)?.is_some() { bail!("ANALYSIS_REQUIRED"); }
    if db.execute("UPDATE asset_notes SET body=?1,revision=revision+1,updated_at=?2 WHERE id=?3 AND revision=?4", params![body,sql_integer(now())?,id,sql_integer(expected_revision)?])? == 0 {
        bail!("NOTE_CONFLICT");
    }
    Ok(())
}
