//! Telegram documents and append-only original edits behind the existing backup contract.
use super::{
    delete_timing::IndexTiming,
    provider::*,
    telegram_replay::{self, Cursor, Event, Projection},
    telegram_transfer::{self, Envelope, Io, Target, Transport},
    telegram_transfer_journal::{CoreJournal, Journal, SendState},
    telegram_transport::{GrammersTransport,ChannelSnapshot},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf},
    sync::{oneshot, watch},
};

#[path = "telegram_archive_segments.rs"]
pub(super) mod segments;
use segments::{Catalog, SegmentLink, SEGMENT_TARGET};

pub(super) fn discard_mismatched_map(core: &Core, scope: &Scope, error: &anyhow::Error) -> Result<bool> {
    if !matches!(error.to_string().as_str(), "TELEGRAM_MEDIA_CHANGED" | "TELEGRAM_MEDIA_MISSING"
        | "TELEGRAM_INDEX_CHANGED" | "BACKUP_RECORD_UNAVAILABLE" | "BACKUP_LIBRARY_MISMATCH"
        | "BACKUP_PUBLICATION_UNCONFIRMED" | "BACKUP_TOMBSTONE_REQUIRED") {
        return Ok(false);
    }
    core.backup_db(|db| Ok(db.execute(
        "DELETE FROM telegram_message_map WHERE space_id=?1 AND json_extract(target_json,'$.scope.library_id')=?2",
        rusqlite::params![scope.space_id,scope.library_id],
    )? != 0))
}

pub struct TelegramProvider {
    core: Weak<Core>,
    target: Target,
    capabilities: ProviderCapabilities,
}
/// Resolve the remote identity within its space before reading local presentation metadata.
fn asset_presentation(
    db: &rusqlite::Connection,
    space: &str,
    remote: &str,
) -> Result<Option<(serde_json::Value, String)>> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT a.metadata_json,a.original_filename FROM backup_space_assets b \
         JOIN media_assets a ON a.asset_id=b.local_asset_id \
         WHERE b.space_id=?1 AND b.remote_asset_id=?2",
            rusqlite::params![space, remote],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(metadata, filename)| Ok((serde_json::from_str(&metadata)?, filename)))
        .transpose()
}

fn thumbnail_source(
    db: &rusqlite::Connection,
    space: &str,
    remote: &str,
    operation: &str,
) -> Result<Option<Vec<u8>>> {
    let payload: Option<String> = db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?1 AND space_id=?2 AND kind='upload'",rusqlite::params![operation,space],|row|row.get(0)).optional()?;
    if let Some(payload) = payload {
        let payload: serde_json::Value = serde_json::from_str(&payload)?;
        if payload["capture"]["remote"] == remote && payload["capture"]["cover"].is_array() {
            return Ok(Some(serde_json::from_value(
                payload["capture"]["cover"].clone(),
            )?));
        }
    }
    Ok(db.query_row("SELECT c.body FROM asset_covers c JOIN backup_space_assets b ON c.asset_id=b.local_asset_id WHERE b.space_id=?1 AND b.remote_asset_id=?2",rusqlite::params![space,remote],|row|row.get(0)).optional()?)
}

pub(super) fn for_upload(
    core: &Core,
    transport: &GrammersTransport,
    target: &Target,
    source: &MediaSource,
) -> Result<GrammersTransport> {
    if source.kind != "video" {
        return Ok(transport.clone());
    }
    let presentation =
        core.backup_db(|db| asset_presentation(db, &target.scope.space_id, &source.asset_id))?;
    let title = presentation
        .as_ref()
        .and_then(|(metadata, _)| metadata["title"].as_str());
    let filename = presentation
        .as_ref()
        .map(|(_, filename)| filename.as_str())
        .or_else(|| source.path.file_name().and_then(|name| name.to_str()));
    Ok(transport.with_heading(title, filename))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Library {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_segment: Option<SegmentLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direct_delete: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    archived_events: Vec<ArchivedEvent>,
    schema_version: u32,
    provider_type: ProviderType,
    mode: String,
    library_id: String,
    account_id: String,
    channel_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArchivedEvent {
    pub revision: i64,
    pub body: Vec<u8>,
}

struct ArchivePlan {
    library: Library,
    previous: Vec<(super::telegram_transfer::RemoteDocument, Vec<u8>)>,
    messages: std::collections::BTreeMap<i32, Vec<super::telegram_transfer::RemoteDocument>>,
}

impl Library {
    fn validate(&self, target: &Target, envelope: &Envelope) -> Result<()> {
        uuid::Uuid::parse_str(&self.library_id).context("TELEGRAM_INDEX_INVALID")?;
        if self.schema_version != 2
            || self.provider_type != ProviderType::Telegram
            || self.mode != "single_writer"
            || self.account_id != target.account_id
            || self.channel_id != target.channel_id
            || target.scope.library_id != self.library_id
            || envelope.library_id != self.library_id
        {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        if let Some(body) = &self.direct_delete {
            let event = Event::decode(target, body)?;
            if !event.change.delete || event.change.recreate || event.change.operation_id != envelope.operation_id
                || event.change.device_id != envelope.device_id || envelope.asset_id != self.library_id {
                bail!("TELEGRAM_INDEX_INVALID");
            }
        }
        Ok(())
    }
    fn merge_archive(&mut self, events: Vec<ArchivedEvent>) -> Result<()> {
        self.archived_events = merge_events(
            self.archived_events
                .iter()
                .chain(&events)
                .map(|event| (event.revision, event.body.clone()))
                .collect(),
        )?
        .into_iter()
        .map(|(revision, body)| ArchivedEvent { revision, body })
        .collect();
        Ok(())
    }

    fn append_archived_events(&self, target: &Target, message: i32, high: &mut i64, events: &mut Vec<(i64, Vec<u8>)>) -> Result<()> {
        for archived in &self.archived_events {
            if archived.revision <= 0 || archived.revision >= (i64::from(message) << 32) {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            Event::decode(target, &archived.body)?;
            // An archive-only channel no longer has a physical rich event to establish this range.
            *high = (*high).max(archived.revision);
            events.push((archived.revision, archived.body.clone()));
        }
        if let Some(body) = &self.direct_delete {
            // Only a newly authored tombstone takes its revision from this carrier.
            // When consolidated into a later archive it is copied with this fixed revision.
            let revision = (i64::from(message) << 32) | 1;
            let event = Event::decode(target, body)?;
            if message <= 0 || !event.change.delete || event.change.recreate { bail!("TELEGRAM_INDEX_INVALID"); }
            *high = (*high).max(revision);
            events.push((revision, body.clone()));
        }
        Ok(())
    }
}

fn merge_events(events: Vec<(i64, Vec<u8>)>) -> Result<Vec<(i64, Vec<u8>)>> {
    let mut merged = std::collections::BTreeMap::new();
    for (revision, body) in events {
        if let Some(previous) = merged.insert(revision, body.clone()) {
            if previous != body { bail!("TELEGRAM_INDEX_CHANGED"); }
        }
    }
    Ok(merged.into_iter().collect())
}

impl TelegramProvider {
    pub(super) fn new(
        core: &Arc<Core>,
        target: Target,
        capabilities: ProviderCapabilities,
    ) -> Self {
        Self {
            core: Arc::downgrade(core),
            target,
            capabilities,
        }
    }
    pub(super) async fn request<T>(
        &self,
        kind: &'static str,
        control: Option<Arc<TransferControl>>,
        work: impl for<'a> FnOnce(&'a Core, &'a GrammersTransport, &'a Io) -> ProviderFuture<'a, T>,
    ) -> Result<T> {
        let core = self.core.upgrade().context("CANCELED")?;
        let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let view = {
            let _ready = super::delete_timing::setup_span("session_ready");
            binding.session.ready_for_backup(binding.namespace).await?
        };
        if !matches!(view.phase, super::telegram_session::Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        let id = view.operation_id.context("TELEGRAM_AUTH_REQUIRED")?;
        if view.account.as_ref().map(|a| a.id.as_str()) != Some(&self.target.account_id) {
            bail!("TELEGRAM_ACCOUNT_MISMATCH");
        }
        binding.session
            .with_transport(
                &core,
                &id,
                self.target.clone(),
                core.backup_cancellation(),
                (kind, control),
                work,
            )
            .await
    }
    pub(super) fn caps(transport: &GrammersTransport) -> ProviderCapabilities {
        ProviderCapabilities {
            change_coordination: ChangeCoordination::EventReplay,
            incremental_discovery: false,
            range_read: false,
            resumable_upload: false,
            max_single_media_bytes: transport.max_bytes(),
            can_delete_owned_media: transport.can_delete(),
        }
    }
    pub(super) fn descriptor_library_id(probe: &Target, envelope: &Envelope, body: &[u8]) -> Result<String> {
        let library: Library = serde_json::from_slice(body).context("TELEGRAM_INDEX_INVALID")?;
        // A new device knows the account/channel but does not yet know its remote library ID.
        let mut target = probe.clone();
        target.scope.library_id.clone_from(&envelope.library_id);
        library.validate(&target, envelope)?;
        Ok(library.library_id)
    }

    /// Read-only discovery does not publish a descriptor or exercise deletion rights.
    pub(super) async fn discover(
        target: &Target,
        transport: &GrammersTransport,
        io: &Io,
    ) -> Result<Option<String>> {
        let (_, documents) = transport.indexes(io).await?;
        let mut library = None;
        for index in documents {
            let document = &index.document;
            let envelope: Envelope =
                serde_json::from_str(&document.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.kind != "library" {
                continue;
            }
            let body = transport.index_body(&index, io).await?;
            let id = Self::descriptor_library_id(target, &envelope, &body)?;
            if library.as_ref().is_some_and(|old| old != &id) {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            library = Some(id);
        }
        Ok(library)
    }
    async fn projection(&self, kind: &'static str, previous: Option<Cursor>) -> Result<Projection> {
        let target = self.target.clone();
        self.request(kind, None, move |core, transport, io| {
            Box::pin(
                async move { Self::project(core, &target, transport, io, previous.as_ref()).await },
            )
        })
        .await
    }
    async fn project(
        core: &Core,
        target: &Target,
        transport: &GrammersTransport,
        io: &Io,
        previous: Option<&Cursor>,
    ) -> Result<Projection> {
        let snapshot=transport.mapped_snapshot(core,target,io).await?;
        Self::project_snapshot(core,target,io,previous,&snapshot)
    }
    pub(super) fn project_snapshot(core:&Core,target:&Target,io:&Io,previous:Option<&Cursor>,snapshot:&ChannelSnapshot)->Result<Projection> {
        let mut high = snapshot.high;
        let catalog = Catalog::read(target, snapshot, io)?;
        let mut events = vec![];
        for segment in catalog.segments.into_values() {
            for record in segment.records {
                high = high.max(record.0);
                events.push(record);
            }
        }
        for index in &snapshot.indexes {
            io.check()?;
            let document = &index.document;
            let envelope: Envelope =
                serde_json::from_str(&document.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.library_id != target.scope.library_id {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            if envelope.kind == "event" {
                let id = index.revision;
                let body = index.verified_body()?;
                let event = Event::decode(target, body)?;
                if event.change.operation_id != envelope.operation_id
                    || event.change.asset_id != envelope.asset_id
                    || event.change.device_id != envelope.device_id
                {
                    bail!("TELEGRAM_INDEX_INVALID");
                }
                events.push((id, body.to_vec()));
            }
        }
        let projected = telegram_replay::replay(target, high, merge_events(events)?, previous)?;
        Self::observe(core, &target.scope.space_id, &projected.cursor)?;
        Ok(projected)
    }
    pub(super) fn observe(core: &Core, space: &str, cursor: &Cursor) -> Result<()> {
        use rusqlite::OptionalExtension;
        core.backup_db(|db| {
            let tx=db.unchecked_transaction()?;
            let old:Option<String>=tx.query_row("SELECT cursor_json FROM telegram_observed_prefix WHERE space_id=?",[space],|r|r.get(0)).optional()?;
            if let Some(old)=old {cursor.covers(&serde_json::from_str(&old)?)?;}
            tx.execute("INSERT INTO telegram_observed_prefix VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET cursor_json=excluded.cursor_json",rusqlite::params![space,serde_json::to_string(cursor)?])?;
            tx.commit()?;Ok(())
        })
    }
    fn cursor(&self, checkpoint: Option<Checkpoint>) -> Result<Option<Cursor>> {
        checkpoint
            .map(|c| {
                self.scope().check(&c.scope)?;
                Ok(serde_json::from_str(&c.value)?)
            })
            .transpose()
    }
    fn saved_cursor(&self) -> Result<Option<Cursor>> {
        use rusqlite::OptionalExtension;
        let core = self.core.upgrade().context("CANCELED")?;
        let raw: Option<String> = core.backup_db(|db| {
            Ok(db
                .query_row(
                    "SELECT checkpoint_json FROM backup_sync_checkpoints WHERE space_id=?",
                    [&self.target.scope.space_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten())
        })?;
        self.cursor(raw.map(|v| serde_json::from_str(&v)).transpose()?)
    }
    pub(super) fn direct_archive_body(io: &Io, target: &Target, asset: &str,
        snapshot: &ChannelSnapshot, tombstone: Vec<u8>) -> Result<Vec<u8>> {
        let mut plan = Self::collect_archive(io, target, &[(asset, u64::MAX)], snapshot)?;
        plan.library.direct_delete = Some(tombstone);
        plan.encode(target, SEGMENT_TARGET)
    }

    // The complete library can change while a rejected upload is waiting to retry.
    // Only Prepared proves no remote send is outstanding. Never change an uncertain send identity.
    pub(super) fn prepare_archive_retry(core: &Core, target: &Target, change: &AssetChange, body: &[u8]) -> Result<()> {
        core.backup_db(|db| {
            let Some(entry) = super::telegram_transfer_journal::get(db, &change.operation_id, "library")? else { return Ok(()) };
            let envelope = &entry.envelope;
            if entry.target != *target || envelope.asset_id != target.scope.library_id
                || envelope.device_id != change.device_id || envelope.operation_id != change.operation_id
                || envelope.library_id != target.scope.library_id || envelope.kind != "library" {
                bail!("TELEGRAM_OPERATION_MISMATCH");
            }
            if envelope.sha256 == format!("{:x}", Sha256::digest(body)) { return Ok(()); }
            match entry.state {
                SendState::Prepared if entry.document.is_none() => {
                    db.execute("DELETE FROM telegram_media_sends WHERE operation_id=? AND kind='library'", [&change.operation_id])?;
                    Ok(())
                }
                SendState::Sending => bail!("TELEGRAM_SEND_UNCONFIRMED"),
                _ => bail!("TELEGRAM_OPERATION_MISMATCH"),
            }
        })
    }

    pub(super) fn publication_exists(journal: &impl Journal, target: &Target, change: &AssetChange,
        body: &[u8], projected: &Projection, snapshot: &ChannelSnapshot) -> Result<bool> {
        let Some(hash) = projected.operations.get(&change.operation_id) else { return Ok(false) };
        if hash != &format!("{:x}", Sha256::digest(body)) { bail!("TELEGRAM_OPERATION_MISMATCH"); }
        if change.delete {
            Self::recover_archive_receipt(journal, target, &change.operation_id, snapshot)?;
        }
        Ok(true)
    }

    // A complete replay may find the archive even when sending it returned a connection error.
    fn recover_archive_receipt(journal: &impl Journal, target: &Target, operation: &str, snapshot: &ChannelSnapshot) -> Result<()> {
        let Some(mut entry) = journal.get(operation, "library")? else { return Ok(()) };
        if entry.target != *target { bail!("BACKUP_SCOPE_MISMATCH"); }
        if entry.state != SendState::Sending { return Ok(()); }
        let caption = entry.envelope.caption()?;
        for index in &snapshot.indexes {
            if index.document.caption == caption {
                index.document.check(&entry.envelope)?;
                if format!("{:x}", Sha256::digest(index.verified_body()?)) != entry.envelope.sha256 {
                    bail!("TELEGRAM_MEDIA_CHANGED");
                }
                entry.document = Some(index.document.clone());
                entry.state = SendState::Confirmed;
                entry.last_error = None;
                return journal.save(&entry);
            }
        }
        // A later consolidated archive can already carry this operation at its original revision.
        Ok(())
    }

    fn collect_archive(
        io: &Io, target: &Target, deletions: &[(&str, u64)], snapshot: &ChannelSnapshot,
    ) -> Result<ArchivePlan> {
        let catalog = Catalog::read(target, snapshot, io)?;
        let parent = catalog.head().parent.clone();
        let mut archive = vec![];
        let mut libraries = vec![];
        let mut sealed = std::collections::BTreeMap::new();
        for (id, segment) in catalog.segments {
            if catalog.sealed.contains(&id) {
                for (revision, body) in segment.records {
                    if sealed.insert(revision, body.clone()).is_some_and(|old| old != body) {
                        bail!("TELEGRAM_INDEX_CHANGED");
                    }
                }
            } else {
                archive.extend(segment.records.into_iter().map(|(revision, body)| ArchivedEvent { revision, body }));
                libraries.push((segment.doc.clone(), segment.body.to_vec()));
            }
        }
        let mut messages =
            std::collections::BTreeMap::<i32, Vec<super::telegram_transfer::RemoteDocument>>::new();
        for index in &snapshot.indexes {
            io.check()?;
            let envelope: Envelope =
                serde_json::from_str(&index.document.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.kind != "event"
                || envelope.library_id != target.scope.library_id
            {
                continue;
            }
            let Some(&(asset, last_revision)) = deletions.iter().find(|(asset,_)| *asset == envelope.asset_id) else { continue };
            let revision = u64::try_from(index.revision).context("TELEGRAM_INDEX_INVALID")?;
            if revision > last_revision {
                bail!("BACKUP_REUPLOAD_STALE");
            }
            let body = index.verified_body()?;
            let event = Event::decode(target, body)?;
            if event.change.asset_id != asset
                || event.change.operation_id != envelope.operation_id
                || event.change.device_id != envelope.device_id
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            archive.push(ArchivedEvent {
                revision: index.revision,
                body: body.to_vec(),
            });
            messages
                .entry(index.document.message_id)
                .or_default()
                .push(index.document.clone());
        }
        let mut library = Library {
            previous_segment: parent, direct_delete: None, archived_events: vec![],
            schema_version: 2,
            provider_type: ProviderType::Telegram,
            mode: "single_writer".into(),
            library_id: target.scope.library_id.clone(),
            account_id: target.account_id.clone(),
            channel_id: target.channel_id.clone(),
        };
        for event in &archive {
            if sealed.get(&event.revision).is_some_and(|body| body != &event.body) {
                bail!("TELEGRAM_INDEX_CHANGED");
            }
        }
        archive.retain(|event| !sealed.contains_key(&event.revision));
        library.merge_archive(archive)?;
        Ok(ArchivePlan { library, previous: libraries, messages })
    }

    #[allow(clippy::too_many_arguments)]
    async fn archive_deleted_events(
        core: &Core,
        transport: &GrammersTransport,
        io: &Io,
        target: &Target,
        operation: &str,
        deletions: &[(&str,u64)],
        snapshot:ChannelSnapshot,
        timing: IndexTiming<'_>,
    ) -> Result<Option<(Vec<(i32, Vec<super::telegram_transfer::RemoteDocument>)>,ChannelSnapshot)>> {
        let mut plan =
            Self::collect_archive(io, target, deletions, &snapshot)?;
        // Compare resolved event revisions: a direct-delete carrier has different wire bytes
        // from the same history frozen in the next archive.
        let mut existing = None;
        if let Some((doc, bytes)) = plan.previous.last() {
            let candidate: Library = serde_json::from_slice(bytes)?;
            let mut records = Vec::new();
            candidate.append_archived_events(target, doc.message_id, &mut 0, &mut records)?;
            if records.iter().map(|(r,b)| (*r,b)).eq(plan.library.archived_events.iter().map(|e| (e.revision,&e.body))) {
                existing = Some((doc.clone(), bytes.clone()));
            }
        }
        let (confirmed, body, snapshot) = if let Some((doc, bytes)) = existing {
            (doc, bytes, snapshot)
        } else {
            // Legacy tombstones and interrupted consolidations can still need a replacement archive.
            let body = plan.encode(target, SEGMENT_TARGET)?;
            let mut identity: [u8; 16] = Sha256::digest(&body)[..16].try_into()?;
            identity[6] = (identity[6] & 0x0f) | 0x80;
            identity[8] = (identity[8] & 0x3f) | 0x80;
            Self::send_index(core, transport, io, target, "library", &target.scope.library_id,
                &uuid::Uuid::from_bytes(identity).to_string(), &core.telegram_storage_device_id()?, body.clone(), timing).await?;
            let snapshot = timing.refresh(transport.mapped_snapshot(core, target, io)).await?;
            let current = Self::project_snapshot(core,target,io,None,&snapshot)?;
            if !deletions.iter().all(|&(id,rev)| current.assets.get(id).is_some_and(|s| s.deleted && s.revision == rev)) {
                bail!("BACKUP_REUPLOAD_STALE");
            }
            let mut confirmed = None;
            for index in &snapshot.indexes {
                let envelope: Envelope = serde_json::from_str(&index.document.caption)?;
                if envelope.kind == "library" && envelope.library_id == target.scope.library_id
                    && envelope.sha256 == format!("{:x}", Sha256::digest(&body))
                    && transport.index_body(index, io).await? == body {
                    confirmed = Some(index.document.clone());
                    break;
                }
            }
            (confirmed.context("BACKUP_PUBLICATION_UNCONFIRMED")?, body, snapshot)
        };
        timing.run("confirm", super::telegram_transport::confirm_archive(transport, target, &confirmed, &body, io)).await?;
        let catalog = Catalog::read(target, &snapshot, io)?;
        if catalog.head().doc != &confirmed { bail!("BACKUP_PUBLICATION_UNCONFIRMED"); }
        // A paired or retried deletion may still own original media in an already sealed segment.
        for id in &catalog.sealed {
            let segment = &catalog.segments[id];
            if segment.records.iter().any(|(r, _)| deletions.iter().any(|(_, revision)| *revision == *r as u64)) {
                timing.run("confirm", super::telegram_transport::confirm_archive(transport, target, segment.doc, segment.body, io)).await?;
            }
        }
        timing.run("replay", async { catalog.confirm_deletions(target, deletions) }).await?;
        if let &[(_, revision)] = deletions {
            if core.backup_db(|db| super::sync_delete::park_confirmed_archive(db, &target.scope.space_id, operation, revision))? {
                return Ok(None);
            }
        }
        // Keep the confirmed head and every immutable ancestor it references.
        let mut old_indexes = plan.previous.into_iter().filter(|(doc, _)| catalog.superseded(doc.message_id)).peekable();
        if old_indexes.peek().is_some() {
            timing.run("delete_old", async {
                for (old, _) in old_indexes {
                    io.run(transport.remove(old)).await?;
                }
                Ok(())
            }).await?;
        }
        Ok(Some((plan.messages.into_iter().collect(),snapshot)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_index(
        core: &Core,
        transport: &GrammersTransport,
        io: &Io,
        target: &Target,
        kind: &str,
        asset: &str,
        operation: &str,
        device: &str,
        body: Vec<u8>,
        timing: IndexTiming<'_>,
    ) -> Result<()> {
        if body.len() > 16 * 1024 * 1024 {
            bail!("BACKUP_METADATA_TOO_LARGE");
        }
        let mut file = tempfile::NamedTempFile::new()?;
        use std::io::Write;
        file.write_all(&body)?;
        file.flush()?;
        let source = MediaSource {
            video: None,
            base_revision: None,
            recreate_revision: None,
            asset_id: asset.into(),
            kind: kind.into(),
            path: file.path().to_owned(),
            bytes: body.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&body)),
            container: "json".into(),
            operation_id: operation.into(),
            device_id: device.into(),
            control: None,
        };
        telegram_transfer::upload_with_timing(transport, &CoreJournal(core), target, source, io, timing).await?;
        Ok(())
    }
}
impl BackupProvider for TelegramProvider {
    fn scope(&self) -> &Scope {
        &self.target.scope
    }
    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }
    fn open_library(&self, create: bool) -> ProviderFuture<'_, LibraryDescriptor> {
        let target = self.target.clone();
        Box::pin(async move {
            self.request("open_library", None, move |core, transport, io| {
                Box::pin(async move {
                    match Self::discover(&target, transport, io).await? {
                        Some(id) if id == target.scope.library_id => {}
                        Some(_) => bail!("BACKUP_LIBRARY_MISMATCH"),
                        None if create => {
                            let body = serde_json::to_vec(&Library {
                                previous_segment: None, direct_delete: None, archived_events: vec![],
                                schema_version: 2,
                                provider_type: ProviderType::Telegram,
                                mode: "single_writer".into(),
                                library_id: target.scope.library_id.clone(),
                                account_id: target.account_id.clone(),
                                channel_id: target.channel_id.clone(),
                            })?;
                            Self::send_index(
                                core,
                                transport,
                                io,
                                &target,
                                "library",
                                &target.scope.library_id,
                                &target.scope.library_id,
                                &core.telegram_storage_device_id()?,
                                body,
                                IndexTiming::default(),
                            )
                            .await?;
                            if Self::discover(&target, transport, io).await?.as_deref()
                                != Some(&target.scope.library_id)
                            {
                                bail!("BACKUP_PUBLICATION_UNCONFIRMED");
                            }
                        }
                        None => bail!("BACKUP_LIBRARY_NOT_FOUND"),
                    }
                    Ok(LibraryDescriptor {
                        library_id: target.scope.library_id,
                        protocol: 2,
                    })
                })
            })
            .await
        })
    }
    fn pull_changes(&self, checkpoint: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch> {
        Box::pin(async move {
            let projected = self.projection("pull_changes", self.cursor(checkpoint)?).await?;
            Ok(ChangeBatch {
                assets: projected
                    .assets
                    .into_values()
                    .map(|s| telegram_replay::remote(self.scope(), s))
                    .collect(),
                checkpoint: OpaqueState {
                    scope: self.scope().clone(),
                    value: serde_json::to_string(&projected.cursor)?,
                },
                complete: true,
            })
        })
    }
    fn read_asset(&self, asset: &str) -> ProviderFuture<'_, Option<RemoteAsset>> {
        let asset = asset.to_owned();
        Box::pin(async move {
            Ok(self
                .projection("read_asset", self.saved_cursor()?)
                .await?
                .assets
                .remove(&asset)
                .map(|s| telegram_replay::remote(self.scope(), s)))
        })
    }
    fn publish_change(
        &self,
        change: AssetChange,
        _base: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt> {
        Box::pin(async move {
            let target = self.target.clone();
            let previous = self.saved_cursor()?;
            let body = Event::encode(&target, &change)?;
            self.request("publish_change", None, move |core, transport, io| {
                Box::pin(async move {
                    if change.delete && !transport.can_delete() {
                        bail!("BACKUP_DELETE_UNSUPPORTED");
                    }
                    let timing = if change.delete {
                        IndexTiming::new(core, &change.operation_id, "publish")
                    } else {
                        IndexTiming::default()
                    };
                    let snapshot = timing.run("locate", transport.mapped_snapshot(core, &target, io)).await?;
                    // An upgraded writer must finish a previously reserved rich tombstone with its original send identity.
                    let legacy_delete = change.delete && CoreJournal(core).get(&change.operation_id, "event")?.is_some();
                    let mut projected = Self::project_snapshot(core, &target, io, previous.as_ref(), &snapshot)?;
                    if !Self::publication_exists(&CoreJournal(core), &target, &change, &body, &projected, &snapshot)? {
                        if change.delete
                            && projected
                                .assets
                                .get(&change.asset_id)
                                .is_some_and(|s| s.deleted)
                        {
                            return Ok(PublishReceipt::Converged {
                                asset: Box::new(telegram_replay::remote(
                                    &target.scope,
                                    projected
                                        .assets
                                        .remove(&change.asset_id)
                                        .expect("checked tombstone"),
                                )),
                            });
                        }
                        telegram_replay::validate_incarnation(
                            &change,
                            projected.assets.get(&change.asset_id),
                            projected.last_deletions.get(&change.asset_id).copied(),
                        )?;
                        if change.delete && !legacy_delete {
                            let archive = Self::direct_archive_body(io, &target, &change.asset_id, &snapshot, body)?;
                            Self::prepare_archive_retry(core, &target, &change, &archive)?;
                            Self::send_index(core, transport, io, &target, "library", &target.scope.library_id,
                                &change.operation_id, &change.device_id, archive, timing).await?;
                        } else {
                            let merged = super::sync_engine::merge(&change, projected.assets.get(&change.asset_id))?;
                            let mut fields = projected.assets.get(&change.asset_id)
                                .map(|asset| asset.fields.clone()).unwrap_or_default();
                            fields.extend(merged.fields);
                            for reference in change
                                .media
                                .iter()
                                .chain(match &change.cover {
                                    ReferenceChange::Set(r) => Some(r),
                                    _ => None,
                                })
                                .chain(match &change.transcript {
                                    ReferenceChange::Set(r) => Some(r),
                                    _ => None,
                                })
                            {
                                let doc = io
                                    .run(transport.inspect_reference(&target,reference))
                                    .await?;
                                let envelope =
                                    super::telegram_lifecycle::metadata(&target, reference, &doc)?;
                                if envelope.asset_id != change.asset_id
                                    || envelope.generation
                                        != projected.last_deletions.get(&change.asset_id).copied()
                                {
                                    bail!("BACKUP_REUPLOAD_STALE");
                                }
                            }
                            let destination=if change.delete {None} else {change.media.as_ref().or_else(||projected.assets.get(&change.asset_id).filter(|s|!s.deleted).and_then(|s|s.media.as_ref())).map(|r|target.message_id(r)).transpose()?};
                            let destination=transport.for_message(destination).with_heading(
                                fields.get("title").and_then(serde_json::Value::as_str),
                                fields.get("original_filename").and_then(serde_json::Value::as_str),
                            );
                            let destination = if legacy_delete && !snapshot.has_operation(&change.operation_id)? {
                                destination.for_new_delete(&change.operation_id)
                            } else {
                                destination
                            };
                            Self::send_index(
                                core,
                                &destination,
                                io,
                                &target,
                                "event",
                                &change.asset_id,
                                &change.operation_id,
                                &change.device_id,
                                body,
                                timing,
                            )
                            .await?;
                        }
                        let refresh = timing.refresh(Self::project(core, &target, transport, io, Some(&projected.cursor)));
                        projected = if change.delete && !legacy_delete {
                            super::telegram_transport::capture_publish_refresh(&target, &change.operation_id, refresh).await?
                        } else {
                            refresh.await?
                        };
                    }
                    if !projected.operations.contains_key(&change.operation_id) {
                        return Ok(PublishReceipt::Accepted {
                            operation_id: change.operation_id,
                        });
                    }
                    let snapshot = projected
                        .assets
                        .remove(&change.asset_id)
                        .context("TELEGRAM_INDEX_INVALID")?;
                    Ok(PublishReceipt::Converged {
                        asset: Box::new(telegram_replay::remote(&target.scope, snapshot)),
                    })
                })
            })
            .await
        })
    }
    fn upload_media(
        &self,
        source: MediaSource,
        resume: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt> {
        Box::pin(async move {
            if let Some(resume) = resume {
                self.scope().check(&resume.scope)?;
            }
            let target = self.target.clone();
            self.request("upload_media", source.control.clone(), move |core, transport, io| {
                Box::pin(async move {
                    let projected = Self::project(core, &target, transport, io, None).await?;
                    let mut source = source;
                    let generation = if let Some(generation) =
                        projected.operation_generations.get(&source.operation_id)
                    {
                        *generation
                    } else {
                        let generation = projected.last_deletions.get(&source.asset_id).copied();
                        if let Some(current) = projected.assets.get(&source.asset_id) {
                            if current.deleted {
                                if source.recreate_revision != Some(current.revision) {
                                    bail!("BACKUP_ASSET_DELETED");
                                }
                            } else if generation
                                .is_some_and(|g| source.base_revision.is_none_or(|r| r < g))
                            {
                                bail!("BACKUP_REUPLOAD_STALE");
                            }
                        } else if source.recreate_revision.is_some() {
                            bail!("BACKUP_REUPLOAD_STALE");
                        }
                        generation
                    };
                    source.recreate_revision = generation;
                    let destination=if source.kind=="video" {None} else {projected.assets.get(&source.asset_id).filter(|s|!s.deleted).and_then(|s|s.media.as_ref()).map(|r|target.message_id(r)).transpose()?};
                    let mut destination=for_upload(core, &transport.for_message(destination), &target, &source)?;
                    telegram_transfer::prepare_video(core, &mut source, io).await?;
                    if source.kind == "video" && CoreJournal(core).get(&source.operation_id, &source.kind)?.is_none_or(|entry|entry.state==SendState::Prepared) {
                        if let Some(body)=core.backup_db(|db|thumbnail_source(db,&target.scope.space_id,&source.asset_id,&source.operation_id))? {
                            let thumbnail=io.run(async {tokio::task::spawn_blocking(move ||crate::telegram_cover::thumbnail(&body)).await.context("COVER_UNAVAILABLE")?}).await?;
                            destination=destination.with_thumbnail(thumbnail);
                        }
                    }
                    telegram_transfer::upload(&destination, &CoreJournal(core), &target, source, io)
                        .await
                })
            })
            .await
        })
    }
    fn cancel_upload(&self, resume: ResumeState, operation_id: &str) -> ProviderFuture<'_, ()> {
        let operation = operation_id.to_owned();
        let target = self.target.clone();
        Box::pin(async move {
            self.scope().check(&resume.scope)?;
            let value: serde_json::Value = serde_json::from_str(&resume.value)?;
            if value["operation_id"] != operation {
                bail!("BACKUP_UPLOAD_NOT_OWNED");
            }
            let kind = value["kind"]
                .as_str()
                .context("BACKUP_INVALID_RESUME_STATE")?
                .to_owned();
            self.request("cancel_upload", None, move |core, transport, io| {
                Box::pin(async move {
                    let journal = CoreJournal(core);
                    let Some(entry) = journal.get(&operation, &kind)? else {
                        return Ok(());
                    };
                    if entry.target != target
                        || value["asset_id"] != entry.envelope.asset_id
                        || value["device_id"] != entry.envelope.device_id
                    {
                        bail!("BACKUP_UPLOAD_NOT_OWNED");
                    }
                    let projected = Self::project(core, &target, transport, io, None).await?;
                    if projected.operations.contains_key(&operation) {
                        return Ok(());
                    }
                    super::telegram_lifecycle::cancel(transport, &target, &entry, &projected, io)
                        .await
                })
            })
            .await
        })
    }
    fn download_media(&self, reference: MediaRef, _offset: u64) -> ProviderFuture<'_, MediaStream> {
        Box::pin(async move {
            self.target.message_id(&reference)?;
            let provider = Self {
                core: self.core.clone(),
                target: self.target.clone(),
                capabilities: self.capabilities.clone(),
            };
            let target = self.target.clone();
            let bytes = reference.bytes;
            let (cancel, rx) = watch::channel(false);
            let (reader, mut writer) = tokio::io::duplex(65536);
            let (done, completion) = oneshot::channel();
            let task = tokio::spawn(async move {
                let mut stop = rx.clone();
                let request=provider.request("download_media", None,move |_,transport,io|Box::pin(async move {
                    let mut source=io.run(transport.media_reader(&target,&reference)).await?;
                    let mut buffer=vec![0;65536];
                    let mut rx=rx;
                    loop {
                        if *rx.borrow() {bail!("TELEGRAM_TRANSFER_CANCELED");}
                        let count=tokio::select! {biased;_=rx.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),result=io.run(async {source.read(&mut buffer).await.map_err(anyhow::Error::from)})=>result?};
                        if count==0 {break;}
                        tokio::select! {biased;_=rx.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),result=io.run(async {writer.write_all(&buffer[..count]).await.map_err(anyhow::Error::from)})=>result?};
                    }
                    Ok(())
                }));
                let result = tokio::select! {biased;_=stop.wait_for(|v|*v)=>Err(anyhow::anyhow!("TELEGRAM_TRANSFER_CANCELED")),result=request=>result};
                let _ = done.send(result);
            });
            Ok(MediaStream {
                bytes,
                offset: 0,
                reader: Box::new(StreamReader {
                    reader,
                    completion,
                    cancel,
                    _task: task,
                    finished: false,
                }),
            })
        })
    }
    fn verify_media(&self, reference: MediaRef) -> ProviderFuture<'_, ()> {
        let target = self.target.clone();
        Box::pin(async move {
            self.request("verify_media", None, move |_, transport, io| {
                Box::pin(async move {
                    super::telegram_lifecycle::verify(transport, &target, &reference, io).await
                })
            })
            .await
        })
    }
    fn delete_media(&self, reference: MediaRef, operation_id: &str) -> ProviderFuture<'_, ()> {
        let operation = operation_id.to_owned();
        let target = self.target.clone();
        Box::pin(async move {
            uuid::Uuid::parse_str(&operation).context("BACKUP_INVALID_OPERATION")?;
            target.message_id(&reference)?;
            self.request("delete_media", None, move |core, transport, io| {
                Box::pin(async move {
                    let doc = match io
                        .run(transport.inspect_reference(&target,&reference))
                        .await
                    {
                        Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
                        other => other?,
                    };
                    let envelope = super::telegram_lifecycle::metadata(&target, &reference, &doc)?;
                    let projected = Self::project(core, &target, transport, io, None).await?;
                    if super::telegram_lifecycle::in_use(&target, &projected, doc.message_id) {
                        bail!("BACKUP_MEDIA_IN_USE");
                    }
                    let deletion = projected
                        .assets
                        .get(&envelope.asset_id)
                        .filter(|s| s.deleted)
                        .context("BACKUP_TOMBSTONE_REQUIRED")?
                        .revision;
                    if super::telegram_lifecycle::candidate(
                        &target,
                        &envelope.asset_id,
                        deletion,
                        &doc,
                        Some(&reference),
                    )?
                    .is_none()
                    {
                        bail!("BACKUP_REUPLOAD_STALE");
                    }
                    super::telegram_lifecycle::remove(transport, &target, &reference, io).await
                })
            })
            .await
        })
    }
    fn reconcile_deleted_asset(
        &self,
        asset_id: &str,
        operation_id: &str,
    ) -> ProviderFuture<'_, DeleteCleanup> {
        let asset = asset_id.to_owned();
        let operation = operation_id.to_owned();
        let target = self.target.clone();
        Box::pin(async move {
            uuid::Uuid::parse_str(&asset).context("BACKUP_INVALID_RECORD")?;
            uuid::Uuid::parse_str(&operation).context("BACKUP_INVALID_OPERATION")?;
            self.request("remote_cleanup", None, move |core, transport, io| {
                Box::pin(async move {
                    let snapshot=super::delete_timing::timed(core,&operation,"locate_messages",transport.mapped_snapshot(core,&target,io)).await?;
                    let projected=super::delete_timing::timed(core,&operation,"read_index",async {Self::project_snapshot(core,&target,io,None,&snapshot)}).await?;
                    let deletion=projected.assets.get(&asset).filter(|s|s.deleted).context("BACKUP_TOMBSTONE_REQUIRED")?.revision;
                    let parked = core.backup_db(|db| super::sync_delete::parked_archive(db, &target.scope.space_id, &operation))?;
                    let parked = parked.filter(|p| projected.assets.get(&p.asset).is_some_and(|s| s.deleted && i64::try_from(s.revision).ok() == Some(p.revision)));
                    let mut deletions = vec![(asset.as_str(),deletion)];
                    if let Some(peer) = &parked {
                        deletions.push((&peer.asset,u64::try_from(peer.revision)?));
                    }
                    let archive = super::delete_timing::timed(core,&operation,"archive",Self::archive_deleted_events(core,transport,io,&target,&operation,&deletions,snapshot,IndexTiming::new(core,&operation,"archive"))).await?;
                    let Some((archived,snapshot)) = archive else { return Ok(DeleteCleanup::ArchiveReady) };
                    let media=snapshot.media.clone();
                    let fresh=super::delete_timing::timed(core,&operation,"prepare_cleanup",async {Self::project_snapshot(core,&target,io,Some(&projected.cursor),&snapshot)}).await?;
                    if !fresh.assets.get(&asset).is_some_and(|s|s.deleted&&s.revision==deletion) {bail!("BACKUP_REUPLOAD_STALE");}
                    let remove = super::delete_timing::timed_setup(core,&operation,"remove_messages",super::telegram_lifecycle::reconcile_deleted_messages(transport,&target,&deletions,io,&fresh,media,archived));
                    let messages = super::delete_timing::with_companion(parked.as_ref().map(|p| p.operation.as_str()),remove).await?;
                    eprintln!("telegram_delete operation={} messages={messages}",operation);
                    let final_state=super::delete_timing::timed(core,&operation,"verify",Self::project(core,&target,transport,io,Some(&fresh.cursor))).await?;
                    if !deletions.iter().all(|&(id,rev)| final_state.assets.get(id).is_some_and(|s|s.deleted && s.revision==rev)) {
                        bail!("BACKUP_REUPLOAD_STALE");
                    }
                    if let Some(peer) = parked {
                        let messages = i64::try_from(messages)?;
                        core.backup_db(|db| {
                            let tx = db.unchecked_transaction()?;
                            tx.execute("UPDATE backup_operations SET phase='local_cleanup',payload_json=json_set(payload_json,'$.cleanup_batch',?2,'$.cleanup_messages',?3)
                                WHERE operation_id=?1 AND state='pending' AND phase='archive_ready' AND json_extract(payload_json,'$.archive_next')=?2",
                                rusqlite::params![peer.operation,operation,messages])?;
                            tx.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.cleanup_batch',?1,'$.cleanup_messages',?2) WHERE operation_id=?1",
                                rusqlite::params![operation,messages])?;
                            tx.commit()?; Ok(())
                        })?;
                    }
                    Ok(DeleteCleanup::Complete)
                })
            })
            .await
        })
    }
}
struct StreamReader {
    reader: tokio::io::DuplexStream,
    completion: oneshot::Receiver<Result<()>>,
    cancel: watch::Sender<bool>,
    _task: tokio::task::JoinHandle<()>,
    finished: bool,
}
impl Drop for StreamReader {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}
impl AsyncRead for StreamReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        use std::future::Future;
        if self.finished || buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let before = buffer.filled().len();
        match Pin::new(&mut self.reader).poll_read(cx, buffer) {
            Poll::Ready(Ok(())) if buffer.filled().len() == before => {
                match Pin::new(&mut self.completion).poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(result) => {
                        self.finished = true;
                        Poll::Ready(match result {
                            Ok(Ok(())) => Ok(()),
                            Ok(Err(error)) => Err(std::io::Error::other(error)),
                            Err(_) => Err(std::io::Error::other("TELEGRAM_TRANSFER_IO")),
                        })
                    }
                }
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod caption_tests {
    use super::*;
    #[test]
    fn thumbnail_uses_captured_original_cover_and_keeps_space_isolation() {
        let db=rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE backup_operations(operation_id TEXT,space_id TEXT,kind TEXT,payload_json TEXT); CREATE TABLE asset_covers(asset_id TEXT,body BLOB); CREATE TABLE backup_space_assets(space_id TEXT,remote_asset_id TEXT,local_asset_id TEXT);").unwrap();
        let captured=vec![1u8,2,3];let current=vec![4u8,5,6];let other=vec![7u8,8,9];
        db.execute("INSERT INTO backup_operations VALUES('operation','a','upload',?1)",[serde_json::json!({"capture":{"remote":"remote","cover":captured}}).to_string()]).unwrap();
        for (space,local,body) in [("a","local-a",&current),("b","local-b",&other)] {
            db.execute("INSERT INTO backup_space_assets VALUES(?1,'remote',?2)",rusqlite::params![space,local]).unwrap();
            db.execute("INSERT INTO asset_covers VALUES(?1,?2)",rusqlite::params![local,body]).unwrap();
        }
        assert_eq!(thumbnail_source(&db,"a","remote","operation").unwrap(),Some(captured.clone()));
        assert_eq!(thumbnail_source(&db,"a","remote","unknown").unwrap(),Some(current));
        assert_eq!(thumbnail_source(&db,"b","remote","operation").unwrap(),Some(other));
        assert!(thumbnail_source(&db,"a","missing","operation").unwrap().is_none());
        let saved:String=db.query_row("SELECT payload_json FROM backup_operations",[],|row|row.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&saved).unwrap()["capture"]["cover"],serde_json::json!(captured));
    }
    #[test]
    fn single_scan_library_validation_rejects_wrong_identity_and_protocol() {
        let id = uuid::Uuid::new_v4().to_string();
        let target = Target {
            scope: Scope {
                space_id: id.clone(),
                library_id: id.clone(),
                provider_type: ProviderType::Telegram,
                protocol: 2,
            },
            account_id: "1".into(),
            channel_id: "2".into(),
        };
        let library = Library {
            previous_segment: None, direct_delete: None, archived_events: vec![],
            schema_version: 2,
            provider_type: ProviderType::Telegram,
            mode: "single_writer".into(),
            library_id: id.clone(),
            account_id: "1".into(),
            channel_id: "2".into(),
        };
        let envelope = Envelope {
            generation: None,
            yingqu_media: 2,
            library_id: id.clone(),
            asset_id: id.clone(),
            operation_id: id.clone(),
            device_id: id,
            kind: "library".into(),
            bytes: 1,
            sha256: "ab".repeat(32),
            container: "json".into(),
        };
        library.validate(&target, &envelope).unwrap();
        for (field, value) in [
            ("schema_version", serde_json::json!(3)),
            ("provider_type", serde_json::json!("s3")),
            ("mode", serde_json::json!("multi_writer")),
            ("account_id", serde_json::json!("other")),
            ("channel_id", serde_json::json!("other")),
            (
                "library_id",
                serde_json::json!(uuid::Uuid::new_v4().to_string()),
            ),
        ] {
            let mut changed = serde_json::to_value(&library).unwrap();
            changed[field] = value;
            assert!(
                serde_json::from_value::<Library>(changed)
                    .unwrap()
                    .validate(&target, &envelope)
                    .is_err(),
                "{field} must still be checked without a separate discovery scan"
            );
        }
    }
    #[test]
    fn library_consolidation_is_stable_across_partial_cleanup_and_keeps_other_assets() {
        let mut library = Library {
            previous_segment: None, direct_delete: None, archived_events: vec![],
            schema_version: 2,
            provider_type: ProviderType::Telegram,
            mode: "single_writer".into(),
            library_id: "library".into(),
            account_id: "1".into(),
            channel_id: "2".into(),
        };
        let old_asset = ArchivedEvent {
            revision: 10,
            body: b"other asset original bytes".to_vec(),
        };
        let edit = ArchivedEvent {
            revision: 20,
            body: b"deleted asset original edit".to_vec(),
        };
        let tombstone = ArchivedEvent {
            revision: 30,
            body: b"deleted asset tombstone".to_vec(),
        };
        library
            .merge_archive(vec![edit.clone(), old_asset.clone()])
            .unwrap();
        library
            .merge_archive(vec![tombstone.clone(), edit.clone()])
            .unwrap();
        let confirmed = serde_json::to_vec(&library).unwrap();
        // Partial cleanup must reuse the same content-addressed send identity.
        library.merge_archive(vec![tombstone]).unwrap();
        assert_eq!(serde_json::to_vec(&library).unwrap(), confirmed);
        assert_eq!(library.archived_events[0], old_asset);
        assert_eq!(library.archived_events[1], edit);
        assert_eq!(library.archived_events.len(), 3);
        let conflict = ArchivedEvent {
            revision: 20,
            body: b"changed edit".to_vec(),
        };
        assert!(library.merge_archive(vec![conflict]).is_err());
        assert_eq!(
            serde_json::to_vec(&library).unwrap(),
            confirmed,
            "a conflict must not erase the last confirmed archive"
        );
    }

    #[test]
    fn library_archive_preserves_exact_replay_after_original_event_messages_are_gone() {
        let target = Target {scope:Scope {space_id:uuid::Uuid::new_v4().to_string(),library_id:uuid::Uuid::new_v4().to_string(),provider_type:ProviderType::Telegram,protocol:2},account_id:"1".into(),channel_id:"2".into()};
        let asset = uuid::Uuid::new_v4().to_string();
        let mut change = AssetChange {recreate:false,resolved_conflicts:vec![],asset_id:asset.clone(),base:None,
            fields:std::collections::BTreeMap::from([("title".into(),serde_json::json!("原始标题")),("original_filename".into(),serde_json::json!("video.mp4"))]),field_bases:Default::default(),
            media:Some(MediaRef {scope:target.scope.clone(),location:MediaLocation::TelegramMessage {account_id:"1".into(),channel_id:"2".into(),message_id:10,document_id:100},bytes:24,sha256:"ab".repeat(32),container:"mp4".into()}),
            cover:ReferenceChange::Unchanged,transcript:ReferenceChange::Unchanged,delete:false,device_id:uuid::Uuid::new_v4().to_string(),operation_id:uuid::Uuid::new_v4().to_string(),conflicts:vec![]};
        let original = ((10_i64<<32)|1,Event::encode(&target,&change).unwrap());
        let first = telegram_replay::replay(&target,20_i64<<32,vec![original.clone()],None).unwrap();
        change.base = Some(first.assets[&asset].clone()); change.media = None; change.fields.clear(); change.delete = true; change.operation_id = uuid::Uuid::new_v4().to_string();
        let tombstone = ((20_i64<<32)|1,Event::encode(&target,&change).unwrap());
        let before = telegram_replay::replay(&target,30_i64<<32,vec![original.clone(),tombstone.clone()],Some(&first.cursor)).unwrap();
        let archive = Library {previous_segment: None, direct_delete: None, archived_events:vec![ArchivedEvent{revision:original.0,body:original.1.clone()},ArchivedEvent{revision:tombstone.0,body:tombstone.1}],schema_version:2,provider_type:ProviderType::Telegram,mode:"single_writer".into(),library_id:target.scope.library_id.clone(),account_id:"1".into(),channel_id:"2".into()};
        let saved: Library = serde_json::from_slice(&serde_json::to_vec(&archive).unwrap()).unwrap();
        let records = saved.archived_events.iter().map(|event|(event.revision,event.body.clone())).collect::<Vec<_>>();
        let mut with_live_copy = records.clone(); with_live_copy.push(original);
        assert_eq!(merge_events(with_live_copy).unwrap().len(),2,"archive confirmation is safe while originals still exist");
        let mut physical_high = 40;
        let mut archive_only = vec![];
        saved.append_archived_events(&target,40,&mut physical_high,&mut archive_only).unwrap();
        let restored = telegram_replay::replay(&target,physical_high,merge_events(archive_only).unwrap(),Some(&before.cursor)).unwrap();
        assert!(restored.assets[&asset].deleted);
        assert_eq!(restored.cursor.evidence,before.cursor.evidence);
        assert_eq!(restored.operations,before.operations);
        let fresh = telegram_replay::replay(&target,40_i64<<32,records.clone(),None).unwrap();
        assert!(fresh.assets[&asset].deleted,"a fresh database must not revive the deleted video");
        assert!(telegram_replay::replay(&target,40_i64<<32,records[1..].to_vec(),Some(&before.cursor)).is_err(),"a missing archived event remains an integrity failure");
        let mut edited = records[0].1.clone(); edited.push(b' ');
        assert!(merge_events(vec![records[0].clone(),(records[0].0,edited)]).is_err(),"conflicting live/archive evidence must not be silently replaced");
        let mut legacy = serde_json::to_value(archive).unwrap(); legacy.as_object_mut().unwrap().remove("archived_events");
        assert!(serde_json::from_value::<Library>(legacy).unwrap().archived_events.is_empty());
    }

    #[test]
    fn video_presentation_uses_the_assets_own_space_binding() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(r#"CREATE TABLE media_assets(asset_id TEXT,metadata_json TEXT,original_filename TEXT);
            CREATE TABLE backup_space_assets(space_id TEXT,remote_asset_id TEXT,local_asset_id TEXT);
            INSERT INTO media_assets VALUES('local-a','{"title":"抖音原始标题"}','原视频.mp4');
            INSERT INTO media_assets VALUES('local-b','{"title":"另一个空间"}','另一个视频.mov');
            INSERT INTO backup_space_assets VALUES('space-a','remote','local-a');
            INSERT INTO backup_space_assets VALUES('space-b','remote','local-b');"#).unwrap();
        let (metadata, filename) = asset_presentation(&db, "space-a", "remote")
            .unwrap()
            .unwrap();
        assert_eq!(metadata["title"], "抖音原始标题");
        assert_eq!(filename, "原视频.mp4");
        let (metadata, filename) = asset_presentation(&db, "space-b", "remote")
            .unwrap()
            .unwrap();
        assert_eq!(metadata["title"], "另一个空间");
        assert_eq!(filename, "另一个视频.mov");
        assert!(asset_presentation(&db, "unknown-space", "remote")
            .unwrap()
            .is_none());
        assert!(asset_presentation(&db, "space-a", "missing")
            .unwrap()
            .is_none());
    }
}
