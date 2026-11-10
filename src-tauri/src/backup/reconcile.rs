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

/// File checks are independent of the one-second database status refresh.
#[derive(Default)]
pub(crate) struct FilePresence(
    std::collections::HashMap<std::path::PathBuf, (std::time::Instant, bool)>,
);
impl FilePresence {
    fn check(&mut self, assets: &mut [Asset], force: bool) {
        let now = std::time::Instant::now();
        let paths: std::collections::HashSet<_> =
            assets.iter().filter_map(|a| a.path.clone()).collect();
        self.0.retain(|path, value| {
            !force
                && paths.contains(path)
                && now.duration_since(value.0) < std::time::Duration::from_secs(30)
        });
        for asset in assets {
            let Some(path) = &asset.path else {
                continue;
            };
            let value = self
                .0
                .entry(path.clone())
                .or_insert_with(|| (now, path.is_file()));
            if !value.1 {
                asset.path = None;
                asset.local_state = "missing".into();
            }
        }
    }
}

/// On-demand callers see current files; polling reuses the short-lived file snapshot.
pub fn list_for_display(core: &Core, space_id: &str) -> Result<Vec<DisplayEntry>> {
    list_for_display_cached(core, space_id, true)
}

pub fn list_for_display_cached(
    core: &Core,
    space_id: &str,
    verify_local: bool,
) -> Result<Vec<DisplayEntry>> {
    core.backup_display_db(|db| {
        let tx = db.unchecked_transaction()?;
        let mut query = tx.prepare("SELECT s.space_id,p.profile,COALESCE(n.display_name,''),s.bucket,l.channel_title
            FROM backup_spaces s JOIN backup_provider_config p USING(space_id)
            LEFT JOIN backup_space_names n USING(space_id) LEFT JOIN telegram_space_labels l USING(space_id)
            WHERE NOT EXISTS(SELECT 1 FROM backup_removed_spaces x WHERE x.space_id=s.space_id) ORDER BY s.rowid DESC")?;
        let spaces = query.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !spaces.iter().any(|s| s.0 == space_id) { bail!("BACKUP_SPACE_NOT_FOUND"); }
        let space_order: std::collections::HashMap<_,_> = spaces.iter().enumerate().map(|(i,s)| (s.0.clone(),i)).collect();
        let spaces: std::collections::HashMap<_,_> = spaces.into_iter().map(|(space_id,profile,name,bucket,channel_title)| {
            let display_name = if !name.trim().is_empty() { name.trim().to_owned() }
                else if profile == "telegram" { "Telegram".into() }
                else { format!("{} / {}", if profile == "r2" { "R2" } else { "S3" }, bucket) };
            (space_id.clone(), BackupLocation { space_id, profile, display_name, channel_title })
        }).collect();
        // Other spaces contribute only membership/location fields, never full assets or remote snapshots.
        let mut query = tx.prepare("SELECT m.local_asset_id,m.space_id,m.pending_json FROM backup_space_assets m
            LEFT JOIN asset_remote_objects o ON o.space_id=m.space_id AND o.asset_id=m.local_asset_id
            WHERE m.remote_deleted=0 AND COALESCE(o.remote_state,'')<>'pending_delete'
            AND NOT EXISTS(SELECT 1 FROM asset_hidden_history h WHERE h.asset_id=m.local_asset_id)
            AND NOT EXISTS(SELECT 1 FROM asset_deletion_hidden_history h WHERE h.space_id=m.space_id AND h.asset_id=m.local_asset_id)")?;
        let mut locations = std::collections::HashMap::<String, Vec<BackupLocation>>::new();
        for row in query.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))? {
            let (asset, space, pending) = row?;
            let Some(location) = spaces.get(&space) else { continue; };
            if serde_json::from_str::<super::sync_engine::PendingChanges>(&pending)?.is_empty() {
                locations.entry(asset).or_default().push(location.clone());
            }
        }
        for values in locations.values_mut() { values.sort_by_key(|v| space_order[&v.space_id]); }
        let mut assets = super::repository::list_assets(&tx)?;
        core.backup_file_presence.lock().expect("backup file presence").check(&mut assets, verify_local);
        let current = super::sync_read::list_in(&tx, space_id, assets.clone())?;
        let mut query = tx.prepare("SELECT a.asset_id,h.asset_id IS NOT NULL,c.asset_id IS NOT NULL,t.asset_id IS NOT NULL
            FROM media_assets a LEFT JOIN asset_hidden_history h USING(asset_id)
            LEFT JOIN asset_covers c USING(asset_id) LEFT JOIN asset_transcripts t USING(asset_id)")?;
        let flags = query.query_map([], |r| Ok((r.get::<_,String>(0)?,(r.get::<_,bool>(1)?,r.get::<_,bool>(2)?,r.get::<_,bool>(3)?))))?
            .collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?;
        let mut seen: std::collections::HashSet<_> = current.iter().map(|e| e.asset.asset_id.clone()).collect();
        let mut result: Vec<DisplayEntry> = current.into_iter().map(|entry| DisplayEntry {
            backup_locations: locations.remove(&entry.asset.asset_id).unwrap_or_default(), entry, in_current_space: true,
        }).collect();
        for asset in assets {
            if !seen.insert(asset.asset_id.clone()) || asset.asset_state != "active" { continue; }
            if asset.path.is_none() && (!locations.contains_key(&asset.asset_id) || asset.task_id.is_none()) { continue; }
            let (hidden,has_cover,has_transcript) = flags[&asset.asset_id];
            if hidden { continue; }
            result.push(DisplayEntry {
                backup_locations: locations.remove(&asset.asset_id).unwrap_or_default(), in_current_space: false,
                entry: Entry { asset, space_id: space_id.into(), remote_state: "not_backed_up".into(), has_cover, has_transcript,
                    remote_deleted: false, has_pending_local_changes: false, conflict_count: 0, remote_revision: 0 },
            });
        }
        Ok(result)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_presence_refresh_is_separate_from_polling_and_can_be_forced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        std::fs::write(&path, b"video").unwrap();
        let original = Asset {
            asset_id: "asset".into(),
            task_id: None,
            metadata: serde_json::json!({}),
            original_filename: "video.mp4".into(),
            content_sha256: None,
            content_bytes: 5,
            asset_state: "active".into(),
            path: Some(path.clone()),
            local_state: "present".into(),
        };
        let mut cache = FilePresence::default();
        cache.check(&mut [original.clone()], false);
        std::fs::remove_file(&path).unwrap();
        let mut polled = [original.clone()];
        cache.check(&mut polled, false);
        assert!(polled[0].path.is_some());
        let mut refreshed = [original.clone()];
        cache.check(&mut refreshed, true);
        assert!(refreshed[0].path.is_none());
        std::fs::write(&path, b"video").unwrap();
        cache.0.get_mut(&path).unwrap().0 -= std::time::Duration::from_secs(31);
        let mut expired = [original];
        cache.check(&mut expired, false);
        assert!(expired[0].path.is_some());
        cache.check(&mut [], false);
        assert!(cache.0.is_empty());
    }
}
