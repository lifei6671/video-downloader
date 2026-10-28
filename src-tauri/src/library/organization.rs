use super::name;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub position: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
}

pub fn categories(db: &Connection) -> Result<Vec<Category>> {
    let mut query = db.prepare(
        "SELECT id,parent_id,name,position FROM library_categories ORDER BY position,name,id",
    )?;
    let rows = query
        .query_map([], |r| {
            Ok(Category {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                position: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn require_category(db: &Connection, id: &str) -> Result<()> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_categories WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("CATEGORY_NOT_FOUND");
    }
    Ok(())
}

pub fn create_category(db: &Connection, parent: Option<&str>, value: &str) -> Result<String> {
    if let Some(parent) = parent {
        require_category(db, parent)?;
    }
    let id = Uuid::new_v4().to_string();
    let value = name(value)?;
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_categories WHERE parent_id IS ?1 AND name=?2)",
        params![parent, value],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("CATEGORY_EXISTS");
    }
    db.execute(
        "INSERT INTO library_categories(id,parent_id,name) VALUES(?1,?2,?3)",
        params![id, parent, value],
    )?;
    Ok(id)
}

pub fn update_category(
    db: &Connection,
    id: &str,
    parent: Option<&str>,
    value: &str,
    position: i64,
) -> Result<()> {
    require_category(db, id)?;
    if let Some(parent) = parent {
        require_category(db, parent)?;
        if db.query_row("WITH RECURSIVE descendants(id) AS (SELECT id FROM library_categories WHERE id=?1 UNION ALL SELECT c.id FROM library_categories c JOIN descendants d ON c.parent_id=d.id) SELECT EXISTS(SELECT 1 FROM descendants WHERE id=?2)", params![id,parent], |r| r.get::<_,bool>(0))? { bail!("CATEGORY_CYCLE"); }
    }
    let value = name(value)?;
    if db.query_row("SELECT EXISTS(SELECT 1 FROM library_categories WHERE parent_id IS ?1 AND name=?2 AND id!=?3)", params![parent,value,id], |r| r.get::<_,bool>(0))? { bail!("CATEGORY_EXISTS"); }
    db.execute(
        "UPDATE library_categories SET parent_id=?1,name=?2,position=?3 WHERE id=?4",
        params![parent, value, position, id],
    )?;
    Ok(())
}

pub fn delete_category(db: &Connection, id: &str) -> Result<()> {
    require_category(db, id)?;
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_categories WHERE parent_id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("CATEGORY_HAS_CHILDREN");
    }
    db.execute("DELETE FROM library_categories WHERE id=?1", [id])?;
    Ok(())
}

pub fn create_tag(db: &Connection, value: &str) -> Result<String> {
    let value = name(value)?;
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO library_tags(id,name) VALUES(?1,?2) ON CONFLICT(name) DO NOTHING",
        params![id, value],
    )?;
    Ok(
        db.query_row("SELECT id FROM library_tags WHERE name=?1", [value], |r| {
            r.get(0)
        })?,
    )
}

pub fn rename_tag(db: &Connection, id: &str, value: &str) -> Result<()> {
    let value = name(value)?;
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_tags WHERE name=?1 AND id!=?2)",
        params![value, id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("TAG_EXISTS");
    }
    if db.execute(
        "UPDATE library_tags SET name=?1 WHERE id=?2",
        params![value, id],
    )? == 0
    {
        bail!("TAG_NOT_FOUND");
    }
    Ok(())
}

pub fn delete_tag(db: &Connection, id: &str) -> Result<()> {
    if db.execute("DELETE FROM library_tags WHERE id=?1", [id])? == 0 {
        bail!("TAG_NOT_FOUND");
    }
    Ok(())
}

pub fn tags(db: &Connection) -> Result<Vec<Tag>> {
    let mut query = db.prepare("SELECT id,name FROM library_tags ORDER BY name,id")?;
    let rows = query
        .query_map([], |r| {
            Ok(Tag {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Each batch is atomic even when one of the requested IDs is invalid.
pub fn set_category_membership(
    db: &Connection,
    asset_ids: &[String],
    category_id: &str,
    add: bool,
) -> Result<()> {
    require_category(db, category_id)?;
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    for id in asset_ids {
        super::require_asset(&tx, id)?;
        if add {
            tx.execute(
                "INSERT INTO asset_categories VALUES(?1,?2) ON CONFLICT DO NOTHING",
                params![id, category_id],
            )?;
        } else {
            tx.execute(
                "DELETE FROM asset_categories WHERE asset_id=?1 AND category_id=?2",
                params![id, category_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn set_tag_membership(
    db: &Connection,
    asset_ids: &[String],
    tag_id: &str,
    add: bool,
) -> Result<()> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_tags WHERE id=?1)",
        [tag_id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("TAG_NOT_FOUND");
    }
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    for id in asset_ids {
        super::require_asset(&tx, id)?;
        if add {
            tx.execute(
                "INSERT INTO asset_tags VALUES(?1,?2) ON CONFLICT DO NOTHING",
                params![id, tag_id],
            )?;
        } else {
            tx.execute(
                "DELETE FROM asset_tags WHERE asset_id=?1 AND tag_id=?2",
                params![id, tag_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn category_assets(db: &Connection, category_id: &str) -> Result<Vec<String>> {
    require_category(db, category_id)?;
    let mut query = db.prepare("WITH RECURSIVE descendants(id) AS (SELECT id FROM library_categories WHERE id=?1 UNION ALL SELECT c.id FROM library_categories c JOIN descendants d ON c.parent_id=d.id) SELECT DISTINCT ac.asset_id FROM asset_categories ac JOIN descendants d ON ac.category_id=d.id JOIN library_assets a ON a.id=ac.asset_id WHERE a.archived_at IS NULL ORDER BY ac.asset_id")?;
    let rows = query
        .query_map([category_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Add many existing labels to one asset atomically; repeated additions are no-ops.
pub(crate) fn add_asset_organizations(db: &Connection, asset: &str, ids: &[String], categories: bool) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    super::require_asset(&tx, asset)?;
    for id in ids {
        if categories { require_category(&tx, id)?; }
        else if !tx.query_row("SELECT EXISTS(SELECT 1 FROM library_tags WHERE id=?1)", [id], |r| r.get::<_, bool>(0))? {
            bail!("TAG_NOT_FOUND");
        }
        tx.execute(if categories { "INSERT INTO asset_categories VALUES(?1,?2) ON CONFLICT DO NOTHING" }
            else { "INSERT INTO asset_tags VALUES(?1,?2) ON CONFLICT DO NOTHING" }, params![asset,id])?;
    }
    tx.commit()?;
    Ok(())
}
