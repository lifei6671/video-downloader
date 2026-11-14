//! Read legacy deletion indexes. New tombstones remain in individual video records.
use super::*;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeletedEntry {
    revision: u64,
    deleted_at: u64,
    device_id: String,
    operation_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeletedIndex {
    schema_version: u32,
    library_id: String,
    assets: BTreeMap<String, DeletedEntry>,
}

impl<S: ObjectStore> S3Provider<S> {
    fn index_key(&self, asset: &str) -> Result<String> {
        uuid::Uuid::parse_str(asset).context("BACKUP_INVALID_RECORD")?;
        Ok(format!(
            "{}deletions/{}.json",
            self.space.prefix,
            asset[..2].to_ascii_lowercase()
        ))
    }

    fn parse_index(&self, object: &super::super::store::Object, key: &str) -> Result<DeletedIndex> {
        let index: DeletedIndex =
            serde_json::from_slice(&object.body).context("BACKUP_INVALID_DELETION_INDEX")?;
        if index.schema_version != 1 || index.library_id != self.scope.library_id {
            bail!("BACKUP_INVALID_DELETION_INDEX");
        }
        for (id, entry) in &index.assets {
            if self.index_key(id)? != key
                || entry.revision == 0
                || entry.revision > i64::MAX as u64
                || entry.deleted_at > i64::MAX as u64
            {
                bail!("BACKUP_INVALID_DELETION_INDEX");
            }
            uuid::Uuid::parse_str(&entry.operation_id).context("BACKUP_INVALID_DELETION_INDEX")?;
            uuid::Uuid::parse_str(&entry.device_id).context("BACKUP_INVALID_DELETION_INDEX")?;
        }
        Ok(index)
    }

    fn indexed_remote(&self, id: String, entry: DeletedEntry, etag: &str) -> Result<RemoteAsset> {
        let wire = WireRecord {
            schema_version: 2,
            library_id: self.scope.library_id.clone(),
            asset_id: id,
            record_revision: entry.revision,
            state: "deleted".into(),
            deleted_at: Some(entry.deleted_at),
            fields: Default::default(),
            media: None,
            cover: None,
            transcript: None,
            conflicts: vec![],
            device_id: entry.device_id,
            operation_id: entry.operation_id,
        };
        let mut remote = self.remote(wire, etag.into())?;
        let mut token: Value = serde_json::from_str(&remote.version.value)?;
        token["archived"] = true.into();
        remote.version.value = token.to_string();
        Ok(remote)
    }

    pub(super) async fn read_deleted_index(&self, asset: &str) -> Result<Option<RemoteAsset>> {
        let key = self.index_key(asset)?;
        let Some(object) = self.store.get(&key).await? else {
            return Ok(None);
        };
        let index = self.parse_index(&object, &key)?;
        index
            .assets
            .into_iter()
            .find(|(id, _)| id == asset)
            .map(|(id, entry)| self.indexed_remote(id, entry, &object.etag))
            .transpose()
    }

    pub(super) async fn read_all_deleted_indexes(
        &self,
        known: &BTreeMap<String, String>,
    ) -> Result<(Vec<RemoteAsset>, BTreeMap<String, String>)> {
        let prefix = format!("{}deletions/", self.space.prefix);
        let mut next = None;
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        let mut versions = BTreeMap::new();
        loop {
            let page = self.store.list(&prefix, next).await?;
            for info in page.objects {
                let shard = info
                    .key
                    .strip_prefix(&prefix)
                    .and_then(|v| v.strip_suffix(".json"))
                    .context("BACKUP_INVALID_DELETION_INDEX")?;
                if shard.len() != 2
                    || !shard
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    bail!("BACKUP_INVALID_DELETION_INDEX");
                }
                let cache_key = format!("@deletions/{shard}");
                versions.insert(cache_key.clone(), info.etag.clone());
                if known.get(&cache_key) == Some(&info.etag) {
                    continue;
                }
                let Some(object) = self.store.get(&info.key).await? else {
                    continue;
                };
                let index = self.parse_index(&object, &info.key)?;
                for (id, entry) in index.assets {
                    result.push(self.indexed_remote(id, entry, &object.etag)?);
                }
            }
            next = page.next;
            if let Some(token) = &next {
                if !seen.insert(token.clone()) {
                    bail!("BACKUP_INVALID_PAGE");
                }
            } else {
                break;
            }
        }
        Ok((result, versions))
    }
}
