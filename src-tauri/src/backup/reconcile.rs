//! Local display projections over the current synchronization state.
use super::model::Asset;
use crate::desktop_core::Core;
use anyhow::{bail, Result};
use serde::Serialize;
#[derive(Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub asset: Asset,
    pub space_id: String,
    pub remote_state: String,
    pub has_cover: bool,
    pub has_transcript: bool,
    pub remote_deleted: bool,
    pub has_pending_local_changes: bool,
    pub conflict_count: usize,
    pub remote_revision: u64,
}
#[derive(Clone, Serialize)]
pub struct BackupLocation {
    pub space_id: String,
    pub profile: String,
    pub display_name: String,
    pub channel_title: Option<String>,
}
#[derive(Serialize)]
pub struct DisplayEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub in_current_space: bool,
    pub backup_locations: Vec<BackupLocation>,
}

/// Local display projection only: switching a target neither removes receipts nor joins assets.
pub fn list_for_display(core: &Core, space_id: &str) -> Result<Vec<DisplayEntry>> {
    let spaces = super::spaces::list(core)?;
    if !spaces.iter().any(|s| s.space_id == space_id) {
        bail!("BACKUP_SPACE_NOT_FOUND");
    }
    let mut current = Vec::new();
    let mut locations = std::collections::HashMap::<String, Vec<BackupLocation>>::new();
    for space in spaces {
        let entries = list(core, &space.space_id)?;
        for entry in &entries {
            if entry.remote_state == "backed_up" && !entry.remote_deleted {
                locations
                    .entry(entry.asset.asset_id.clone())
                    .or_default()
                    .push(BackupLocation {
                        space_id: space.space_id.clone(),
                        profile: space.profile.clone(),
                        display_name: if !space.display_name.trim().is_empty() {
                            space.display_name.trim().to_owned()
                        } else if space.profile == "telegram" {
                            "Telegram".to_owned()
                        } else {
                            format!(
                                "{} / {}",
                                if space.profile == "r2" { "R2" } else { "S3" },
                                space.bucket
                            )
                        },
                        channel_title: space.telegram_channel_title.clone(),
                    });
            }
        }
        if space.space_id == space_id {
            current = entries;
        }
    }
    let mut seen: std::collections::HashSet<String> =
        current.iter().map(|e| e.asset.asset_id.clone()).collect();
    let mut result: Vec<DisplayEntry> = current
        .into_iter()
        .map(|entry| DisplayEntry {
            backup_locations: locations.remove(&entry.asset.asset_id).unwrap_or_default(),
            entry,
            in_current_space: true,
        })
        .collect();
    for mut asset in core.backup_assets()? {
        if !seen.insert(asset.asset_id.clone()) || asset.asset_state != "active" {
            continue;
        }
        if !asset.path.as_ref().is_some_and(|p| p.is_file()) {
            asset.path = None;
            asset.local_state = "missing".into();
            if !locations.contains_key(&asset.asset_id) || asset.task_id.is_none() {
                continue;
            }
        }
        let hidden: bool = core.backup_db(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_hidden_history WHERE asset_id=?)",
                [&asset.asset_id],
                |r| r.get(0),
            )?)
        })?;
        if hidden {
            continue;
        }
        let (has_cover, has_transcript) = core.backup_db(|db| {
            Ok((
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM asset_covers WHERE asset_id=?)",
                    [&asset.asset_id],
                    |r| r.get(0),
                )?,
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM asset_transcripts WHERE asset_id=?)",
                    [&asset.asset_id],
                    |r| r.get(0),
                )?,
            ))
        })?;
        result.push(DisplayEntry {
            backup_locations: locations.remove(&asset.asset_id).unwrap_or_default(),
            in_current_space: false,
            entry: Entry {
                asset,
                space_id: space_id.into(),
                remote_state: "not_backed_up".into(),
                has_cover,
                has_transcript,
                remote_deleted: false,
                has_pending_local_changes: false,
                conflict_count: 0,
                remote_revision: 0,
            },
        });
    }
    Ok(result)
}
#[derive(Serialize, Default)]
pub struct Refresh {
    pub imported: usize,
    pub unchanged: usize,
    pub rejected: Vec<String>,
}

pub fn list(core: &Core, space_id: &str) -> Result<Vec<Entry>> {
    super::sync_read::list(core, space_id)
}
