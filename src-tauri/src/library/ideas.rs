//! Original writing stays separate from live references to research material.
#[cfg(feature = "mcp")]
use rmcp::schemars;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryIdea {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_source: Option<super::AiSource>,
    pub id: String,
    pub title: String,
    pub viewpoint: String,
    pub body: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct IdeaSummary {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub characters: u64,
    pub reference_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub enum IdeaReferenceKind {
    Video,
    Clip,
    Topic,
}
impl IdeaReferenceKind {
    fn key(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Clip => "clip",
            Self::Topic => "topic",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct IdeaReferenceInput {
    pub kind: IdeaReferenceKind,
    pub asset_id: Option<String>,
    pub resource_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeaReference {
    pub idea_id: String,
    pub kind: IdeaReferenceKind,
    pub asset_id: Option<String>,
    pub resource_id: String,
    pub label: String,
    pub asset_title: Option<String>,
    pub position: u64,
    pub created_at: u64,
}
impl IdeaReference {
    fn input(&self) -> IdeaReferenceInput {
        IdeaReferenceInput {
            kind: self.kind,
            asset_id: self.asset_id.clone(),
            resource_id: self.resource_id.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct IdeaReferenceView {
    pub reference: IdeaReferenceInput,
    pub title: String,
    pub asset_title: Option<String>,
    pub missing: bool,
    pub archived: bool,
    pub file_state: Option<String>,
    pub topic_revision: Option<u64>,
}
#[derive(Serialize)]
pub struct IdeaDetail {
    pub idea: LibraryIdea,
    pub references: Vec<IdeaReferenceView>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct IdeaInput {
    pub title: String,
    pub viewpoint: String,
    pub body: String,
    pub references: Vec<IdeaReferenceInput>,
}
#[derive(Serialize)]
pub struct WorkbenchIdeas {
    pub items: Vec<IdeaSummary>,
    pub total: u64,
}

fn idea_row(row: &Row<'_>) -> rusqlite::Result<LibraryIdea> {
    Ok(LibraryIdea {
        ai_source: super::ai_source::from_row(row, 7)?,
        id: row.get(0)?,
        title: row.get(1)?,
        viewpoint: row.get(2)?,
        body: row.get(3)?,
        revision: super::unsigned(row, 4)?,
        created_at: super::unsigned(row, 5)?,
        updated_at: super::unsigned(row, 6)?,
    })
}
fn summary_row(row: &Row<'_>) -> rusqlite::Result<IdeaSummary> {
    Ok(IdeaSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        revision: super::unsigned(row, 2)?,
        characters: super::unsigned(row, 3)?,
        reference_count: super::unsigned(row, 4)?,
        created_at: super::unsigned(row, 5)?,
        updated_at: super::unsigned(row, 6)?,
    })
}
const SUMMARY: &str = "SELECT i.id,i.title,i.revision,length(i.body),(SELECT count(*) FROM idea_references r WHERE r.idea_id=i.id),i.created_at,i.updated_at FROM library_ideas i ORDER BY i.updated_at DESC,i.id DESC";
pub fn idea_summaries(db: &Connection) -> Result<Vec<IdeaSummary>> {
    Ok(db
        .prepare(SUMMARY)?
        .query_map([], summary_row)?
        .collect::<rusqlite::Result<_>>()?)
}
pub(super) fn recent_ideas(db: &Connection) -> Result<WorkbenchIdeas> {
    let total = db.query_row("SELECT count(*) FROM library_ideas", [], |r| {
        super::unsigned(r, 0)
    })?;
    let items = db
        .prepare(&format!("{SUMMARY} LIMIT 6"))?
        .query_map([], summary_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(WorkbenchIdeas { items, total })
}
pub(super) fn records(db: &Connection) -> Result<Vec<LibraryIdea>> {
    Ok(db.prepare("SELECT id,title,viewpoint,body,revision,created_at,updated_at,ai_source_json FROM library_ideas ORDER BY id")?.query_map([],idea_row)?.collect::<rusqlite::Result<_>>()?)
}
fn reference_row(row: &Row<'_>) -> rusqlite::Result<IdeaReference> {
    let kind: String = row.get(1)?;
    let kind = match kind.as_str() {
        "video" => IdeaReferenceKind::Video,
        "clip" => IdeaReferenceKind::Clip,
        "topic" => IdeaReferenceKind::Topic,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(IdeaReference {
        idea_id: row.get(0)?,
        kind,
        asset_id: row.get(2)?,
        resource_id: row.get(3)?,
        label: row.get(4)?,
        asset_title: row.get(5)?,
        position: super::unsigned(row, 6)?,
        created_at: super::unsigned(row, 7)?,
    })
}
const REFERENCES:&str="SELECT idea_id,kind,asset_id,resource_id,label,asset_title,position,created_at FROM idea_references";
pub(super) fn reference_records(db: &Connection) -> Result<Vec<IdeaReference>> {
    Ok(db
        .prepare(&format!("{REFERENCES} ORDER BY idea_id,position"))?
        .query_map([], reference_row)?
        .collect::<rusqlite::Result<_>>()?)
}
fn references_for(db: &Connection, id: &str) -> Result<Vec<IdeaReference>> {
    Ok(db
        .prepare(&format!("{REFERENCES} WHERE idea_id=?1 ORDER BY position"))?
        .query_map([id], reference_row)?
        .collect::<rusqlite::Result<_>>()?)
}
fn source(
    db: &Connection,
    input: &IdeaReferenceInput,
    label: &str,
    asset_title: Option<&str>,
) -> Result<IdeaReferenceView> {
    if input.resource_id.is_empty() {
        bail!("IDEA_REFERENCE_INVALID");
    }
    if input.kind == IdeaReferenceKind::Topic {
        if input.asset_id.is_some() || asset_title.is_some() {
            bail!("IDEA_REFERENCE_INVALID");
        }
        let current: Option<(String, u64)> = db
            .query_row(
                "SELECT title,revision FROM research_topics WHERE id=?1",
                [&input.resource_id],
                |r| Ok((r.get(0)?, super::unsigned(r, 1)?)),
            )
            .optional()?;
        return Ok(IdeaReferenceView {
            reference: input.clone(),
            title: current
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| label.into()),
            asset_title: None,
            missing: current.is_none(),
            archived: false,
            file_state: None,
            topic_revision: current.map(|r| r.1),
        });
    }
    let asset = input
        .asset_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .context("IDEA_REFERENCE_INVALID")?;
    let kind = if input.kind == IdeaReferenceKind::Video {
        super::TopicMemberKind::Video
    } else {
        super::TopicMemberKind::Clip
    };
    let value = super::topics::material_source(
        db,
        kind,
        asset,
        &input.resource_id,
        label,
        asset_title.unwrap_or(""),
    )
    .context("IDEA_REFERENCE_INVALID")?;
    Ok(IdeaReferenceView {
        reference: input.clone(),
        title: value.title,
        asset_title: Some(value.asset_title),
        missing: value.missing,
        archived: value.archived,
        file_state: value.file_state,
        topic_revision: None,
    })
}
pub fn prepare_idea_reference(
    db: &Connection,
    input: &IdeaReferenceInput,
) -> Result<IdeaReferenceView> {
    let view = source(db, input, "", None)?;
    if view.missing {
        bail!("IDEA_REFERENCE_INVALID");
    }
    Ok(view)
}
pub fn library_idea(db: &Connection, id: &str) -> Result<IdeaDetail> {
    let idea=db.query_row("SELECT id,title,viewpoint,body,revision,created_at,updated_at,ai_source_json FROM library_ideas WHERE id=?1",[id],idea_row).optional()?.context("IDEA_NOT_FOUND")?;
    let references = references_for(db, id)?
        .iter()
        .map(|r| source(db, &r.input(), &r.label, r.asset_title.as_deref()))
        .collect::<Result<_>>()?;
    Ok(IdeaDetail { idea, references })
}
fn validate_content(title: &str, viewpoint: &str, body: &str) -> Result<()> {
    super::name(title)?;
    if viewpoint.len() > 64 * 1024 || body.len() > 16 * 1024 * 1024 {
        bail!("IDEA_INVALID");
    }
    Ok(())
}
fn save_references(
    db: &Connection,
    id: &str,
    inputs: &[IdeaReferenceInput],
    previous: &[IdeaReference],
) -> Result<()> {
    let mut seen = HashSet::new();
    let mut resolved = Vec::new();
    for (position, input) in inputs.iter().enumerate() {
        if !seen.insert((input.kind, &input.resource_id)) {
            bail!("IDEA_REFERENCE_EXISTS");
        }
        let old = previous.iter().find(|r| r.input() == *input);
        let value = source(
            db,
            input,
            old.map(|r| r.label.as_str()).unwrap_or(""),
            old.and_then(|r| r.asset_title.as_deref()),
        )?;
        if value.missing && old.is_none() {
            bail!("IDEA_REFERENCE_INVALID");
        }
        resolved.push(IdeaReference {
            idea_id: id.into(),
            kind: input.kind,
            asset_id: input.asset_id.clone(),
            resource_id: input.resource_id.clone(),
            label: value.title.chars().take(512).collect(),
            asset_title: value.asset_title.map(|s| s.chars().take(512).collect()),
            position: position as u64,
            created_at: old.map(|r| r.created_at).unwrap_or_else(super::now),
        });
    }
    db.execute("DELETE FROM idea_references WHERE idea_id=?1", [id])?;
    for reference in &resolved {
        insert_reference(db, reference)?;
    }
    Ok(())
}
pub fn create_idea(db: &Connection, input: &IdeaInput) -> Result<String> {
    create_with_source(db, input, None)
}
pub(crate) fn create_with_source(db: &Connection, input: &IdeaInput, source: Option<&super::AiSource>) -> Result<String> {
    validate_content(&input.title, &input.viewpoint, &input.body)?;
    if let Some(source) = source { source.validate(db)?; }
    let tx = db.unchecked_transaction()?;
    let id = uuid::Uuid::new_v4().to_string();
    let time = super::sql_integer(super::now())?;
    tx.execute("INSERT INTO library_ideas(id,title,viewpoint,body,revision,created_at,updated_at,ai_source_json) VALUES(?1,?2,?3,?4,1,?5,?5,?6)",params![id,input.title.trim(),input.viewpoint,input.body,time,source.map(serde_json::to_string).transpose()?])?;
    save_references(&tx, &id, &input.references, &[])?;
    tx.commit()?;
    Ok(id)
}
pub fn edit_idea(db: &Connection, id: &str, input: &IdeaInput, revision: u64) -> Result<()> {
    validate_content(&input.title, &input.viewpoint, &input.body)?;
    let tx = db.unchecked_transaction()?;
    if tx.execute("UPDATE library_ideas SET title=?1,viewpoint=?2,body=?3,revision=revision+1,updated_at=?4 WHERE id=?5 AND revision=?6",params![input.title.trim(),input.viewpoint,input.body,super::sql_integer(super::now())?,id,super::sql_integer(revision)?])?!=1 {bail!("IDEA_CONFLICT");}
    save_references(&tx, id, &input.references, &references_for(&tx, id)?)?;
    tx.commit()?;
    Ok(())
}
pub(super) fn insert_reference(db: &Connection, r: &IdeaReference) -> Result<()> {
    db.execute("INSERT INTO idea_references(idea_id,kind,asset_id,resource_id,label,asset_title,position,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![r.idea_id,r.kind.key(),r.asset_id,r.resource_id,r.label,r.asset_title,super::sql_integer(r.position)?,super::sql_integer(r.created_at)?])?;
    Ok(())
}
pub(super) fn validate_backup(db: &Connection) -> Result<()> {
    for idea in records(db)? {
        if let Some(source) = &idea.ai_source { source.validate(db)?; }
        validate_content(&idea.title, &idea.viewpoint, &idea.body)?;
        if idea.id.is_empty() || idea.revision == 0 {
            bail!("IDEA_INVALID");
        }
        for (position, r) in references_for(db, &idea.id)?.iter().enumerate() {
            if r.position != position as u64
                || r.label.chars().count() > 512
                || r.asset_title
                    .as_ref()
                    .is_some_and(|s| s.chars().count() > 512)
                || (r.kind == IdeaReferenceKind::Topic) != r.asset_title.is_none()
            {
                bail!("IDEA_REFERENCE_INVALID");
            }
            source(db, &r.input(), &r.label, r.asset_title.as_deref())?;
        }
    }
    Ok(())
}
