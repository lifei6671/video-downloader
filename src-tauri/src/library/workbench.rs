use super::{assets::asset_row, now, require_asset, sql_integer, unsigned, Asset};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Serialize)]
pub struct WorkbenchAssets {
    pub items: Vec<Asset>,
    pub total: u64,
}

#[derive(Serialize)]
pub struct WorkbenchLibrary {
    pub ideas: super::WorkbenchIdeas,
    pub recent: WorkbenchAssets,
    pub uncategorized: WorkbenchAssets,
    pub unorganized: WorkbenchAssets,
}

pub fn record_visit(db: &Connection, asset_id: &str) -> Result<()> {
    require_asset(db, asset_id)?;
    db.execute("INSERT INTO asset_visits(asset_id,visited_at,visit_order)
        VALUES(?1,?2,(SELECT coalesce(max(visit_order),0)+1 FROM asset_visits))
        ON CONFLICT(asset_id) DO UPDATE SET visited_at=excluded.visited_at,visit_order=excluded.visit_order",
        params![asset_id, sql_integer(now())?])?;
    Ok(())
}

pub fn workbench_library(db: &Connection) -> Result<WorkbenchLibrary> {
    fn section(
        db: &Connection,
        join: &str,
        condition: &str,
        order: &str,
    ) -> Result<WorkbenchAssets> {
        let from =
            format!("FROM library_assets a {join} WHERE a.archived_at IS NULL AND ({condition})");
        let total = db.query_row(&format!("SELECT count(*) {from}"), [], |r| unsigned(r, 0))?;
        let mut query = db.prepare(&format!("SELECT a.id,a.title,a.author,a.platform,a.video_id,a.canonical_url,a.created_at,a.updated_at,a.archived_at {from} ORDER BY {order} LIMIT 6"))?;
        let items = query
            .query_map([], asset_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(WorkbenchAssets { items, total })
    }
    Ok(WorkbenchLibrary {
        ideas: super::ideas::recent_ideas(db)?,
        recent: section(db, "JOIN asset_visits v ON v.asset_id=a.id", "1", "v.visit_order DESC,a.id DESC")?,
        uncategorized: section(db, "", "NOT EXISTS(SELECT 1 FROM asset_categories WHERE asset_id=a.id)", "a.created_at DESC,a.id DESC")?,
        unorganized: section(db, "", "NOT EXISTS(SELECT 1 FROM asset_documents WHERE asset_id=a.id AND kind='edited') AND NOT EXISTS(SELECT 1 FROM asset_notes WHERE asset_id=a.id)", "a.created_at DESC,a.id DESC")?,
    })
}
