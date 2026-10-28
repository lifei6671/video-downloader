use super::{asset_files, assets::asset_row, optional_unsigned, sql_integer, Asset};
use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileFilter {
    #[default]
    All,
    Available,
    Missing,
    Changed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchCursor {
    pub created_at: u64,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    pub category_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub platform: Option<String>,
    pub archived: bool,
    pub uncategorized: bool,
    pub unorganized: bool,
    pub file_state: FileFilter,
    pub cursor: Option<SearchCursor>,
    pub limit: u32,
}
impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            category_ids: vec![],
            tag_ids: vec![],
            platform: None,
            archived: false,
            uncategorized: false,
            unorganized: false,
            file_state: FileFilter::All,
            cursor: None,
            limit: 20,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub field: String,
    pub excerpt: String,
    pub document_id: Option<String>,
    pub document_version: Option<u64>,
    pub note_id: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct FileCounts {
    pub total: usize,
    pub available: usize,
    pub changed: usize,
}
#[derive(Debug, Serialize)]
pub struct SearchItem {
    pub asset: Asset,
    pub hit: Option<SearchHit>,
    pub files: FileCounts,
}
#[derive(Debug, Serialize)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<SearchCursor>,
}

pub fn search_assets(db: &Connection, request: &SearchRequest) -> Result<SearchPage> {
    if request.limit == 0
        || request.limit > 100
        || request.query.chars().count() > 512
        || (request.uncategorized && !request.category_ids.is_empty())
        || request.cursor.as_ref().is_some_and(|c| c.id.is_empty())
        || request
            .platform
            .as_deref()
            .is_some_and(|p| !matches!(p, "douyin" | "tiktok" | "youtube" | "telegram" | "local"))
    {
        bail!("INVALID_INPUT");
    }
    let categories = serde_json::to_string(&request.category_ids)?;
    let tags = serde_json::to_string(&request.tag_ids)?;
    if db.query_row("SELECT EXISTS(SELECT 1 FROM json_each(?1) selected WHERE NOT EXISTS(SELECT 1 FROM library_categories c WHERE c.id=selected.value))",[&categories],|r|r.get::<_,bool>(0))? {bail!("CATEGORY_NOT_FOUND");}
    if db.query_row("SELECT EXISTS(SELECT 1 FROM json_each(?1) selected WHERE NOT EXISTS(SELECT 1 FROM library_tags t WHERE t.id=selected.value))",[&tags],|r|r.get::<_,bool>(0))? {bail!("TAG_NOT_FOUND");}
    let mut query = db.prepare(include_str!("search.sql"))?;
    let rows = query.query_map(
        params![
            request.query.trim(),
            categories,
            tags,
            request.archived,
            request.platform,
            request.uncategorized,
            request
                .cursor
                .as_ref()
                .map(|c| sql_integer(c.created_at))
                .transpose()?,
            request.cursor.as_ref().map(|c| &c.id),
            request.unorganized
        ],
        |r| {
            let field: Option<String> = r.get(9)?;
            let hit = field
                .map(|field| -> rusqlite::Result<SearchHit> {
                    Ok(SearchHit {
                        field,
                        excerpt: r.get(10)?,
                        document_id: r.get(11)?,
                        document_version: optional_unsigned(r, 12)?,
                        note_id: r.get(13)?,
                    })
                })
                .transpose()?;
            Ok((asset_row(r)?, hit))
        },
    )?;
    let mut items = Vec::new();
    let mut more = false;
    for row in rows {
        let (asset, hit) = row?;
        let files = asset_files(db, &asset.id)?;
        let counts = FileCounts {
            total: files.len(),
            available: files.iter().filter(|f| !f.missing).count(),
            changed: files.iter().filter(|f| f.changed).count(),
        };
        let matches = match request.file_state {
            FileFilter::All => true,
            FileFilter::Available => counts.available > 0,
            FileFilter::Missing => counts.available == 0,
            FileFilter::Changed => counts.changed > 0,
        };
        if !matches {
            continue;
        }
        if items.len() == request.limit as usize {
            more = true;
            break;
        }
        items.push(SearchItem {
            asset,
            hit,
            files: counts,
        });
    }
    let next_cursor = if more {
        items.last().map(|item| SearchCursor {
            created_at: item.asset.created_at,
            id: item.asset.id.clone(),
        })
    } else {
        None
    };
    Ok(SearchPage { items, next_cursor })
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
