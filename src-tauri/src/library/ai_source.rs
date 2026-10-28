//! Provenance for AI-created research and writing, separate from editable text.
use anyhow::{bail, Result};
#[cfg(feature = "mcp")]
use rmcp::schemars;
use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AiSource {
    pub model_name: String,
    pub citations: Vec<super::Citation>,
}
impl AiSource {
    pub(crate) fn validate(&self, db: &Connection) -> Result<()> {
        if self.model_name.trim().is_empty()
            || self.model_name.chars().count() > 128
            || self.citations.len() > 100
        {
            bail!("INVALID_INPUT");
        }
        super::documents::validate_citations(db, &self.citations)
    }
}

pub(super) fn from_row(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<AiSource>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}
