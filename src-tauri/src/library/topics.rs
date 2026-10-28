//! Research collections store references, never copies or ownership of source material.
#[cfg(feature = "mcp")]
use rmcp::schemars;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchTopic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_source: Option<super::AiSource>,
    pub id: String,
    pub title: String,
    pub description: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopicMemberKind {
    Video,
    Clip,
    Note,
}
impl TopicMemberKind {
    fn key(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Clip => "clip",
            Self::Note => "note",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicMember {
    pub id: String,
    pub topic_id: String,
    pub kind: TopicMemberKind,
    pub asset_id: String,
    pub resource_id: String,
    pub label: String,
    pub asset_title: String,
    pub position: u64,
    pub created_at: u64,
}
#[derive(Serialize)]
pub struct TopicMemberView {
    pub member: TopicMember,
    pub title: String,
    pub asset_title: String,
    pub missing: bool,
    pub archived: bool,
    pub file_state: Option<String>,
    pub note_revision: Option<u64>,
}
#[derive(Serialize)]
pub struct ResearchTopicDetail {
    pub topic: ResearchTopic,
    pub members: Vec<TopicMemberView>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct TopicInput {
    pub title: String,
    pub description: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicMemberInput {
    pub kind: TopicMemberKind,
    pub asset_id: String,
    pub resource_id: String,
}

fn topic_row(r: &Row<'_>) -> rusqlite::Result<ResearchTopic> {
    Ok(ResearchTopic {
        ai_source: super::ai_source::from_row(r, 6)?,
        id: r.get(0)?,
        title: r.get(1)?,
        description: r.get(2)?,
        revision: super::unsigned(r, 3)?,
        created_at: super::unsigned(r, 4)?,
        updated_at: super::unsigned(r, 5)?,
    })
}
fn member_row(r: &Row<'_>) -> rusqlite::Result<TopicMember> {
    let kind: String = r.get(2)?;
    let kind = match kind.as_str() {
        "video" => TopicMemberKind::Video,
        "clip" => TopicMemberKind::Clip,
        "note" => TopicMemberKind::Note,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(TopicMember {
        id: r.get(0)?,
        topic_id: r.get(1)?,
        kind,
        asset_id: r.get(3)?,
        resource_id: r.get(4)?,
        label: r.get(5)?,
        asset_title: r.get(6)?,
        position: super::unsigned(r, 7)?,
        created_at: super::unsigned(r, 8)?,
    })
}
pub fn research_topics(db: &Connection) -> Result<Vec<ResearchTopic>> {
    Ok(db.prepare("SELECT id,title,description,revision,created_at,updated_at,ai_source_json FROM research_topics ORDER BY updated_at DESC,id")?.query_map([],topic_row)?.collect::<rusqlite::Result<_>>()?)
}
fn topic(db: &Connection, id: &str) -> Result<ResearchTopic> {
    db.query_row("SELECT id,title,description,revision,created_at,updated_at,ai_source_json FROM research_topics WHERE id=?1",[id],topic_row).optional()?.context("TOPIC_NOT_FOUND")
}
pub(super) fn member_records(db: &Connection) -> Result<Vec<TopicMember>> {
    Ok(db.prepare("SELECT id,topic_id,kind,asset_id,resource_id,label,asset_title,position,created_at FROM research_topic_members ORDER BY topic_id,position,id")?.query_map([],member_row)?.collect::<rusqlite::Result<_>>()?)
}
fn members_for(db: &Connection, id: &str) -> Result<Vec<TopicMember>> {
    Ok(db.prepare("SELECT id,topic_id,kind,asset_id,resource_id,label,asset_title,position,created_at FROM research_topic_members WHERE topic_id=?1 ORDER BY position,id")?.query_map([id],member_row)?.collect::<rusqlite::Result<_>>()?)
}
fn note_label(body: &str) -> String {
    body.lines()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("笔记")
        .chars()
        .take(120)
        .collect()
}
pub(super) struct MaterialSource {
    pub title: String, pub asset_title: String, pub missing: bool, pub archived: bool,
    pub file_state: Option<String>, pub note_revision: Option<u64>,
}
pub(super) fn material_source(db: &Connection, kind: TopicMemberKind, asset_id: &str, resource_id: &str, label: &str, asset_title: &str) -> Result<MaterialSource> {
    let asset: Option<(String, bool)> = db
        .query_row(
            "SELECT title,archived_at IS NOT NULL FROM library_assets WHERE id=?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut view = MaterialSource {
        title: label.into(),
        asset_title: asset
            .as_ref()
            .map(|a| a.0.clone())
            .unwrap_or_else(|| asset_title.into()),
        missing: asset.is_none(),
        archived: asset.as_ref().is_some_and(|a| a.1),
        file_state: None,
        note_revision: None,
    };
    match kind {
        TopicMemberKind::Video => {
            if resource_id != asset_id {
                bail!("TOPIC_REFERENCE_INVALID");
            }
            if let Some((title, _)) = asset {
                view.title = title;
                let files = super::asset_files(db, asset_id)?;
                view.file_state = Some(
                    if files.is_empty() {
                        "unknown"
                    } else if files.iter().all(|f| f.missing) {
                        "missing"
                    } else if files.iter().any(|f| f.changed) {
                        "changed"
                    } else {
                        "available"
                    }
                    .into(),
                );
            }
        }
        TopicMemberKind::Clip => {
            let row: Option<String> = db
                .query_row(
                    "SELECT asset_id FROM asset_clips WHERE id=?1",
                    [resource_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(owner) = row {
                if owner != asset_id {
                    bail!("TOPIC_REFERENCE_INVALID");
                }
                let clip = super::clips(db, &owner)?
                    .into_iter()
                    .find(|c| c.id == resource_id)
                    .context("TOPIC_REFERENCE_INVALID")?;
                view.title = clip.title;
                view.file_state = Some(
                    if clip.missing {
                        "missing"
                    } else if clip.source_changed {
                        "changed"
                    } else {
                        "available"
                    }
                    .into(),
                );
            } else {
                view.missing = true;
            }
        }
        TopicMemberKind::Note => {
            let row: Option<(String, String, u64)> = db
                .query_row(
                    "SELECT asset_id,body,revision FROM asset_notes WHERE id=?1",
                    [resource_id],
                    |r| Ok((r.get(0)?, r.get(1)?, super::unsigned(r, 2)?)),
                )
                .optional()?;
            if let Some((owner, body, revision)) = row {
                if owner != asset_id {
                    bail!("TOPIC_REFERENCE_INVALID");
                }
                view.title = note_label(&body);
                view.note_revision = Some(revision);
            } else {
                view.missing = true;
            }
        }
    }
    Ok(view)
}
fn resolve(db: &Connection, member: TopicMember) -> Result<TopicMemberView> {
    let source = material_source(db, member.kind, &member.asset_id, &member.resource_id, &member.label, &member.asset_title)?;
    Ok(TopicMemberView { member, title: source.title, asset_title: source.asset_title, missing: source.missing, archived: source.archived, file_state: source.file_state, note_revision: source.note_revision })
}

pub fn research_topic(db: &Connection, id: &str) -> Result<ResearchTopicDetail> {
    let topic = topic(db, id)?;
    let members = members_for(db, id)?
        .into_iter()
        .filter(|m| m.topic_id == id)
        .map(|m| resolve(db, m))
        .collect::<Result<_>>()?;
    Ok(ResearchTopicDetail { topic, members })
}
fn validate_input(input: &TopicInput) -> Result<()> {
    super::name(&input.title)?;
    if input.description.len() > 64 * 1024 {
        bail!("TOPIC_INVALID");
    }
    Ok(())
}
pub fn create_research_topic(db: &Connection, input: &TopicInput) -> Result<String> {
    create_with_source(db, input, None)
}
pub(crate) fn create_with_source(db: &Connection, input: &TopicInput, source: Option<&super::AiSource>) -> Result<String> {
    validate_input(input)?;
    if let Some(source) = source { source.validate(db)?; }
    let id = uuid::Uuid::new_v4().to_string();
    let time = super::sql_integer(super::now())?;
    db.execute("INSERT INTO research_topics(id,title,description,revision,created_at,updated_at,ai_source_json) VALUES(?1,?2,?3,1,?4,?4,?5)",params![id,input.title.trim(),input.description,time,source.map(serde_json::to_string).transpose()?])?;
    Ok(id)
}
fn advance(db: &Connection, id: &str, revision: u64) -> Result<()> {
    if db.execute(
        "UPDATE research_topics SET revision=revision+1,updated_at=?1 WHERE id=?2 AND revision=?3",
        params![
            super::sql_integer(super::now())?,
            id,
            super::sql_integer(revision)?
        ],
    )? != 1
    {
        bail!("TOPIC_CONFLICT");
    }
    Ok(())
}
pub fn edit_research_topic(
    db: &Connection,
    id: &str,
    input: &TopicInput,
    revision: u64,
) -> Result<()> {
    validate_input(input)?;
    let tx = db.unchecked_transaction()?;
    topic(&tx, id)?;
    advance(&tx, id, revision)?;
    tx.execute(
        "UPDATE research_topics SET title=?1,description=?2 WHERE id=?3",
        params![input.title.trim(), input.description, id],
    )?;
    tx.commit()?;
    Ok(())
}
pub fn add_topic_member(
    db: &Connection,
    id: &str,
    input: &TopicMemberInput,
    revision: u64,
) -> Result<String> {
    let tx = db.unchecked_transaction()?;
    topic(&tx, id)?;
    super::require_asset(&tx, &input.asset_id)?;
    let mut member = TopicMember {
        id: uuid::Uuid::new_v4().to_string(),
        topic_id: id.into(),
        kind: input.kind,
        asset_id: input.asset_id.clone(),
        resource_id: input.resource_id.clone(),
        label: String::new(),
        asset_title: String::new(),
        position: 0,
        created_at: super::now(),
    };
    let view = resolve(&tx, member.clone())?;
    if view.missing {
        bail!("TOPIC_REFERENCE_INVALID");
    }
    member.label = view.title.chars().take(512).collect();
    member.asset_title = view.asset_title.chars().take(512).collect();
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM research_topic_members WHERE topic_id=?1 AND kind=?2 AND resource_id=?3)",params![id,input.kind.key(),input.resource_id],|r|r.get::<_,bool>(0))? { bail!("TOPIC_MEMBER_EXISTS"); }
    advance(&tx, id, revision)?;
    member.position = tx.query_row(
        "SELECT COUNT(*) FROM research_topic_members WHERE topic_id=?1",
        [id],
        |r| super::unsigned(r, 0),
    )?;
    insert_member(&tx, &member)?;
    tx.commit()?;
    Ok(member.id)
}
pub(super) fn insert_member(db: &Connection, m: &TopicMember) -> Result<()> {
    db.execute("INSERT INTO research_topic_members(id,topic_id,kind,asset_id,resource_id,label,asset_title,position,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![m.id,m.topic_id,m.kind.key(),m.asset_id,m.resource_id,m.label,m.asset_title,super::sql_integer(m.position)?,super::sql_integer(m.created_at)?])?;
    Ok(())
}
pub fn remove_topic_member(
    db: &Connection,
    id: &str,
    member_id: &str,
    revision: u64,
) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    topic(&tx, id)?;
    advance(&tx, id, revision)?;
    if tx.execute(
        "DELETE FROM research_topic_members WHERE id=?1 AND topic_id=?2",
        params![member_id, id],
    )? != 1
    {
        bail!("TOPIC_MEMBER_NOT_FOUND");
    }
    let remaining = members_for(&tx, id)?
        .into_iter()
        .filter(|m| m.topic_id == id)
        .map(|m| m.id)
        .collect::<Vec<_>>();
    order(&tx, id, &remaining)?;
    tx.commit()?;
    Ok(())
}
fn order(db: &Connection, id: &str, members: &[String]) -> Result<()> {
    for (position, member) in members.iter().enumerate() {
        db.execute(
            "UPDATE research_topic_members SET position=?1 WHERE id=?2 AND topic_id=?3",
            params![position as i64, member, id],
        )?;
    }
    Ok(())
}
pub fn reorder_topic_members(
    db: &Connection,
    id: &str,
    members: &[String],
    revision: u64,
) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    topic(&tx, id)?;
    let current = members_for(&tx, id)?
        .into_iter()
        .filter(|m| m.topic_id == id)
        .map(|m| m.id)
        .collect::<HashSet<_>>();
    if members.len() != current.len()
        || members.iter().collect::<HashSet<_>>().len() != members.len()
        || members.iter().any(|id| !current.contains(id))
    {
        bail!("TOPIC_INVALID");
    }
    advance(&tx, id, revision)?;
    order(&tx, id, members)?;
    tx.commit()?;
    Ok(())
}
pub(super) fn validate_backup(db: &Connection) -> Result<()> {
    for topic in research_topics(db)? {
        if let Some(source) = &topic.ai_source { source.validate(db)?; }
        validate_input(&TopicInput {
            title: topic.title,
            description: topic.description,
        })?;
        if topic.id.is_empty() || topic.revision == 0 {
            bail!("TOPIC_INVALID");
        }
        for (position, member) in members_for(db, &topic.id)?
            .into_iter()
            .filter(|m| m.topic_id == topic.id)
            .enumerate()
        {
            if member.id.is_empty()
                || member.asset_id.is_empty()
                || member.resource_id.is_empty()
                || member.position != position as u64
                || member.label.chars().count() > 512
                || member.asset_title.chars().count() > 512
            {
                bail!("TOPIC_INVALID");
            }
            // Missing records are legitimate historical references; existing records must still have the declared owner.
            resolve(db, member)?;
        }
    }
    Ok(())
}
