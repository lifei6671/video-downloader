//! Compact deletion evidence, partitioned by UUID prefix and coordinated by existing CAS writes.
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
            if !entry.device_id.is_empty() {
                uuid::Uuid::parse_str(&entry.device_id).context("BACKUP_INVALID_DELETION_INDEX")?;
            }
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

    async fn announce_indexes(&self) -> Result<()> {
        let key = format!("{}library.json", self.space.prefix);
        for _ in 0..5 {
            let object = self
                .store
                .get(&key)
                .await?
                .context("BACKUP_LIBRARY_NOT_FOUND")?;
            let mut library: LibraryMetadata = serde_json::from_slice(&object.body)?;
            library.validate()?;
            if library.library_id != self.scope.library_id {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            if library.deletion_indexes {
                return Ok(());
            }
            library.deletion_indexes = true;
            match self
                .store
                .write(Write::Put {
                    key: key.clone(),
                    body: serde_json::to_vec(&library)?,
                    condition: Condition::Match(object.etag),
                })
                .await
            {
                Ok(_) => return Ok(()),
                Err(e)
                    if matches!(
                        e.to_string().as_str(),
                        "BACKUP_PRECONDITION_FAILED"
                            | "BACKUP_CONCURRENT_UPDATE"
                            | "BACKUP_NETWORK_ERROR"
                            | "BACKUP_TIMEOUT"
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            }
        }
        bail!("BACKUP_CONCURRENT_UPDATE")
    }

    async fn archive(&self, wire: &WireRecord) -> Result<RemoteAsset> {
        self.announce_indexes().await?;
        let key = self.index_key(&wire.asset_id)?;
        for _ in 0..5 {
            let old = self.store.get(&key).await?;
            let mut index = match &old {
                Some(object) => self.parse_index(object, &key)?,
                None => DeletedIndex {
                    schema_version: 1,
                    library_id: self.scope.library_id.clone(),
                    assets: Default::default(),
                },
            };
            if let Some(entry) = index.assets.get(&wire.asset_id) {
                if entry.revision > wire.record_revision {
                    bail!("BACKUP_RECORD_ROLLBACK");
                }
                if entry.revision == wire.record_revision {
                    if entry.operation_id != wire.operation_id {
                        bail!("BACKUP_CONCURRENT_UPDATE");
                    }
                    return self
                        .read_deleted_index(&wire.asset_id)
                        .await?
                        .context("BACKUP_INVALID_DELETION_INDEX");
                }
            }
            index.assets.insert(
                wire.asset_id.clone(),
                DeletedEntry {
                    revision: wire.record_revision,
                    deleted_at: wire.deleted_at.context("BACKUP_DELETION_TIME_MISSING")?,
                    device_id: wire.device_id.clone(),
                    operation_id: wire.operation_id.clone(),
                },
            );
            let body = serde_json::to_vec(&index)?;
            if body.len() > 16 * 1024 * 1024 {
                bail!("BACKUP_METADATA_TOO_LARGE");
            }
            self.writable().await?;
            let condition = old.map_or(Condition::Absent, |o| Condition::Match(o.etag));
            match self
                .store
                .write(Write::Put {
                    key: key.clone(),
                    body,
                    condition,
                })
                .await
            {
                Ok(_) => {
                    return self
                        .read_deleted_index(&wire.asset_id)
                        .await?
                        .context("BACKUP_INVALID_DELETION_INDEX")
                }
                Err(e)
                    if matches!(
                        e.to_string().as_str(),
                        "BACKUP_PRECONDITION_FAILED"
                            | "BACKUP_CONCURRENT_UPDATE"
                            | "BACKUP_NETWORK_ERROR"
                            | "BACKUP_TIMEOUT"
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            }
        }
        bail!("BACKUP_CONCURRENT_UPDATE")
    }

    pub(super) async fn compact_records(
        &self,
        candidates: Vec<RemoteAsset>,
        days: u32,
        now: u64,
    ) -> Result<Vec<RemoteAsset>> {
        if !(1..=3650).contains(&days) {
            bail!("BACKUP_INVALID_RETENTION_DAYS");
        }
        self.writable().await?;
        if !self.capabilities.can_delete_owned_media {
            bail!("BACKUP_DELETE_UNSUPPORTED");
        }
        let mut result = Vec::new();
        for candidate in candidates {
            if result.len() >= 128 {
                break;
            }
            candidate.version.scope.check(&self.scope)?;
            if !candidate.snapshot.deleted {
                continue;
            }
            let token: Value = serde_json::from_str(&candidate.version.value)?;
            if token["archived"] == true {
                continue;
            }
            let asset = &candidate.snapshot.asset_id;
            let key = self.key(asset)?;
            let Some(object) = self.store.get(&key).await? else {
                if let Some(indexed) = self.read_deleted_index(asset).await? {
                    result.push(indexed);
                }
                continue;
            };
            let mut wire = validate_wire(
                &object.body,
                &self.scope.library_id,
                asset,
                &self.space.prefix,
            )?;
            if wire.state != "deleted" {
                continue;
            }
            let Some(deleted_at) = wire.deleted_at else {
                // Historical records have no reliable deletion clock. Begin retention when first observed.
                wire.deleted_at = Some(now);
                match self
                    .store
                    .write(Write::Put {
                        key,
                        body: serde_json::to_vec(&wire)?,
                        condition: Condition::Match(object.etag),
                    })
                    .await
                {
                    Ok(_) => {}
                    Err(e) if e.to_string() == "BACKUP_PRECONDITION_FAILED" => {}
                    Err(e) => return Err(e),
                }
                continue;
            };
            if now < deleted_at || now - deleted_at < u64::from(days) * 86400 {
                continue;
            }
            let indexed = self.archive(&wire).await?;
            // A concurrent re-upload may have replaced this record: never unconditionally delete it.
            self.writable().await?;
            match self
                .store
                .write(Write::DeleteIfMatch {
                    key: key.clone(),
                    etag: object.etag,
                })
                .await
            {
                Ok(_) => {}
                Err(e) if e.to_string() == "BACKUP_PRECONDITION_FAILED" => continue,
                Err(e) => {
                    if self.store.get(&key).await?.is_some() {
                        return Err(e);
                    }
                }
            }
            if self.store.get(&key).await?.is_none() {
                result.push(indexed);
            }
            if result.len() >= 128 {
                break;
            }
        }
        Ok(result)
    }
}
