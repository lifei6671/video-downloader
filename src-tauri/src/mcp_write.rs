//! Explicitly opted-in additive tools. No document edits, deletion or settings routes.
use crate::{desktop_core::library_api::LibraryScope, library};
use anyhow::{bail, Context, Result};
#[cfg(feature = "mcp")]
use rmcp::schemars;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const METHODS: &[&str] = &[
    "mcp_create_note",
    "mcp_create_category",
    "mcp_add_asset_categories",
    "mcp_add_asset_tags",
    "mcp_create_topic",
    "mcp_create_idea",
];

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Created {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub asset_id: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub asset_id: String,
    pub body: String,
    pub source: library::AiSource,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CategoryInput {
    pub name: String,
    /// Legacy compatibility field; only null is accepted. Categories have one level.
    pub parent_id: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MembershipInput {
    pub asset_id: String,
    pub ids: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TopicInput {
    pub input: library::TopicInput,
    pub source: library::AiSource,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct IdeaInput {
    pub input: library::IdeaInput,
    pub source: library::AiSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WriteError {
    WriteDisabled,
    InvalidInput,
    InvalidCitation,
    AssetNotFound,
    CategoryNotFound,
    CategoryExists,
    TagNotFound,
    TopicInvalid,
    IdeaInvalid,
    IdeaReferenceInvalid,
    IdeaReferenceExists,
    ServiceUnavailable,
    CoreWriteFailed,
}
impl WriteError {
    pub fn code(self) -> &'static str {
        match self {
            Self::WriteDisabled => "WRITE_DISABLED",
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidCitation => "INVALID_CITATION",
            Self::AssetNotFound => "ASSET_NOT_FOUND",
            Self::CategoryNotFound => "CATEGORY_NOT_FOUND",
            Self::CategoryExists => "CATEGORY_EXISTS",
            Self::TagNotFound => "TAG_NOT_FOUND",
            Self::TopicInvalid => "TOPIC_INVALID",
            Self::IdeaInvalid => "IDEA_INVALID",
            Self::IdeaReferenceInvalid => "IDEA_REFERENCE_INVALID",
            Self::IdeaReferenceExists => "IDEA_REFERENCE_EXISTS",
            Self::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            Self::CoreWriteFailed => "CORE_WRITE_FAILED",
        }
    }
    pub(crate) fn from_domain(error: anyhow::Error) -> Self {
        serde_json::from_value(Value::String(error.to_string())).unwrap_or(Self::CoreWriteFailed)
    }
}
pub(crate) struct Written {
    pub value: Value,
    pub scope: LibraryScope,
    pub asset_ids: Vec<String>,
}
fn input<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).context("INVALID_INPUT")
}
pub(crate) fn dispatch(db: &Connection, method: &str, params: Value) -> Result<Written> {
    let created = |id, scope, asset_ids| Written {
        value: json!({"id":id,"origin":"ai"}),
        scope,
        asset_ids,
    };
    Ok(match method {
        "mcp_create_note" => {
            let request: NoteInput = input(params)?;
            crate::mcp_read::require_collected(db, &request.asset_id)?;
            require_collected_sources(db, &request.source)?;
            request.source.validate(db)?;
            let id = library::create_note(
                db,
                &request.asset_id,
                library::NewNote {
                    kind: "general",
                    body: &request.body,
                    origin: library::NoteOrigin::Ai,
                    model_name: Some(&request.source.model_name),
                    citations: &request.source.citations,
                },
            )?;
            created(id, LibraryScope::Notes, vec![request.asset_id])
        }
        "mcp_create_category" => {
            let request: CategoryInput = input(params)?;
            if request.parent_id.is_some() { bail!("INVALID_INPUT"); }
            Written {
                value: json!({"id":library::create_category(db, request.parent_id.as_deref(), &request.name)?}),
                scope: LibraryScope::Categories,
                asset_ids: vec![],
            }
        }
        "mcp_add_asset_categories" | "mcp_add_asset_tags" => {
            let request: MembershipInput = input(params)?;
            crate::mcp_read::require_collected(db, &request.asset_id)?;
            if request.ids.is_empty() || request.ids.len() > 100 {
                bail!("INVALID_INPUT");
            }
            library::add_asset_organizations(
                db,
                &request.asset_id,
                &request.ids,
                method == "mcp_add_asset_categories",
            )?;
            Written {
                value: json!({"asset_id":request.asset_id,"ids":request.ids}),
                scope: LibraryScope::Assets,
                asset_ids: vec![request.asset_id],
            }
        }
        "mcp_create_topic" => {
            let request: TopicInput = input(params)?;
            require_collected_sources(db, &request.source)?;
            created(
                library::topics::create_with_source(db, &request.input, Some(&request.source))?,
                LibraryScope::Topics,
                vec![],
            )
        }
        "mcp_create_idea" => {
            let request: IdeaInput = input(params)?;
            require_collected_sources(db, &request.source)?;
            for reference in &request.input.references {
                if reference.kind == library::IdeaReferenceKind::Topic {
                    for member in library::research_topic(db, &reference.resource_id)?.members {
                        crate::mcp_read::require_collected(db, &member.member.asset_id)?;
                    }
                } else {
                    crate::mcp_read::require_collected(db, reference.asset_id.as_deref().context("INVALID_INPUT")?)?;
                }
            }
            if request.input.references.len() > 100 {
                bail!("INVALID_INPUT");
            }
            created(
                library::ideas::create_with_source(db, &request.input, Some(&request.source))?,
                LibraryScope::Ideas,
                vec![],
            )
        }
        _ => bail!("INVALID_INPUT"),
    })
}

fn require_collected_sources(db: &Connection, source: &library::AiSource) -> Result<()> {
    for citation in &source.citations { crate::mcp_read::require_collected(db, &citation.asset_id)?; }
    Ok(())
}

#[cfg(test)]
mod tests;
