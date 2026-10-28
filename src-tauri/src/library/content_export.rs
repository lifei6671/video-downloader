use super::{asset, read_document, read_note, Asset};
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    Txt,
    Markdown,
    Json,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ContentSelection {
    Document(String),
    Note(String),
}

fn document_references(
    db: &Connection,
    asset: &Asset,
    id: &str,
    quote: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    let mut result = Vec::new();
    let mut next = Some(id.to_owned());
    while let Some(id) = next {
        let document = read_document(db, &asset.id, &id)?;
        next = document.summary.source_document_id.clone();
        result.push(
            serde_json::json!({"asset":asset, "document":document.summary,
            "source":document.source,"source_changed":document.source_changed,
            "model_revision":document.model_revision,"pipeline_revision":document.pipeline_revision,
            "quote":if result.is_empty() {quote} else {None}}),
        );
    }
    Ok(result)
}

pub fn export_content(
    db: &Connection,
    asset_id: &str,
    selection: &ContentSelection,
    format: ContentFormat,
) -> Result<String> {
    let asset = asset(db, asset_id)?;
    let mut references = Vec::new();
    let (body, content) = match selection {
        ContentSelection::Document(id) => {
            let document = read_document(db, asset_id, id)?;
            if let Some(source) = &document.summary.source_document_id {
                references = document_references(db, &asset, source, None)?;
            }
            (document.text.clone(), serde_json::to_value(document)?)
        }
        ContentSelection::Note(id) => {
            let note = read_note(db, asset_id, id)?;
            for citation in &note.citations {
                references.extend(document_references(
                    db,
                    &super::asset(db, &citation.asset_id)?,
                    &citation.document_id,
                    Some(&citation.quote),
                )?);
            }
            (note.body.clone(), serde_json::to_value(note)?)
        }
    };
    let metadata = serde_json::json!({"format":"yingqu-content", "version":1, "asset":asset, "selection":selection, "content":content, "references":references});
    match format {
        ContentFormat::Txt => Ok(body),
        ContentFormat::Json => serde_json::to_string_pretty(&metadata).context("DATABASE_ERROR"),
        ContentFormat::Markdown => {
            let title = asset.title.replace(['\r', '\n'], " ");
            // Metadata is a JSON code block; arbitrary user text is the Markdown body.
            let mut provenance = metadata;
            provenance["content"]
                .as_object_mut()
                .unwrap()
                .remove("text");
            provenance["content"]
                .as_object_mut()
                .unwrap()
                .remove("body");
            let json = serde_json::to_string_pretty(&provenance)?;
            let longest = json.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            Ok(format!(
                "# {title}\n\n{body}\n\n## 来源信息\n\n{fence}json\n{json}\n{fence}\n"
            ))
        }
    }
}
