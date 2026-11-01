//! Telegram documents and append-only original edits behind the existing backup contract.
use super::{
    provider::*,
    telegram_replay::{self, Cursor, Event, Projection},
    telegram_transfer::{self, Envelope, Io, Target, Transport},
    telegram_transfer_journal::{CoreJournal, Journal},
    telegram_transport::GrammersTransport,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
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

pub struct TelegramProvider {
    core: Weak<Core>,
    target: Target,
    capabilities: ProviderCapabilities,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    schema_version: u32,
    provider_type: ProviderType,
    mode: String,
    library_id: String,
    account_id: String,
    channel_id: String,
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
        control: Option<Arc<TransferControl>>,
        work: impl for<'a> FnOnce(&'a Core, &'a GrammersTransport, &'a Io) -> ProviderFuture<'a, T>,
    ) -> Result<T> {
        let core = self.core.upgrade().context("CANCELED")?;
        let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let view = binding.session
            .ready_for_backup(binding.namespace)
            .await?;
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
                control,
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
    /// Read-only discovery does not publish a descriptor or exercise deletion rights.
    pub(super) async fn discover(
        target: &Target,
        transport: &GrammersTransport,
        io: &Io,
    ) -> Result<Option<String>> {
        let (_, documents) = transport.indexes(io).await?;
        let mut library = None;
        for index in documents {
            let document=index.document;
            let envelope: Envelope =
                serde_json::from_str(&document.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.kind != "library" {
                continue;
            }
            let body = transport.index_body(document, io).await?;
            let value: Library = serde_json::from_slice(&body).context("TELEGRAM_INDEX_INVALID")?;
            uuid::Uuid::parse_str(&value.library_id).context("TELEGRAM_INDEX_INVALID")?;
            if value.schema_version != 2
                || value.provider_type != ProviderType::Telegram
                || value.mode != "single_writer"
                || value.account_id != target.account_id
                || value.channel_id != target.channel_id
                || envelope.library_id != value.library_id
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            if library.as_ref().is_some_and(|old| old != &value.library_id) {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            library = Some(value.library_id);
        }
        Ok(library)
    }
    async fn projection(&self, previous: Option<Cursor>) -> Result<Projection> {
        let target = self.target.clone();
        self.request(None, move |core, transport, io| {
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
        if Self::discover(target, transport, io).await?.as_deref() != Some(&target.scope.library_id)
        {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        let (high, documents) = transport.indexes(io).await?;
        let mut events = vec![];
        for index in documents {
            let document=index.document;
            let envelope: Envelope =
                serde_json::from_str(&document.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.library_id != target.scope.library_id {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            if envelope.kind == "event" {
                let id = index.revision;
                let body = transport.index_body(document, io).await?;
                let event = Event::decode(target, &body)?;
                if event.change.operation_id != envelope.operation_id
                    || event.change.asset_id != envelope.asset_id
                    || event.change.device_id != envelope.device_id
                {
                    bail!("TELEGRAM_INDEX_INVALID");
                }
                events.push((id, body));
            }
        }
        let projected = telegram_replay::replay(target, high, events, previous)?;
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
        telegram_transfer::upload(transport, &CoreJournal(core), target, source, io).await?;
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
            self.request(None, move |core, transport, io| {
                Box::pin(async move {
                    match Self::discover(&target, transport, io).await? {
                        Some(id) if id == target.scope.library_id => {}
                        Some(_) => bail!("BACKUP_LIBRARY_MISMATCH"),
                        None if create => {
                            let body = serde_json::to_vec(&Library {
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
            let projected = self.projection(self.cursor(checkpoint)?).await?;
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
                .projection(self.saved_cursor()?)
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
            let hash = format!("{:x}", Sha256::digest(&body));
            self.request(None, move |core, transport, io| {
                Box::pin(async move {
                    if change.delete && !transport.can_delete() {
                        bail!("BACKUP_DELETE_UNSUPPORTED");
                    }
                    let mut projected =
                        Self::project(core, &target, transport, io, previous.as_ref()).await?;
                    if let Some(old) = projected.operations.get(&change.operation_id) {
                        if old != &hash {
                            bail!("TELEGRAM_OPERATION_MISMATCH");
                        }
                    } else {
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
                        super::sync_engine::merge(&change, projected.assets.get(&change.asset_id))?;
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
                        let destination=transport.for_message(destination);
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
                        )
                        .await?;
                        projected =
                            Self::project(core, &target, transport, io, Some(&projected.cursor))
                                .await?;
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
            self.request(source.control.clone(), move |core, transport, io| {
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
                    let destination=transport.for_message(destination);
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
            self.request(None, move |core, transport, io| {
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
                let request=provider.request(None,move |_,transport,io|Box::pin(async move {
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
            self.request(None, move |_, transport, io| {
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
            self.request(None, move |core, transport, io| {
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
    ) -> ProviderFuture<'_, ()> {
        let asset = asset_id.to_owned();
        let operation = operation_id.to_owned();
        let target = self.target.clone();
        Box::pin(async move {
            uuid::Uuid::parse_str(&asset).context("BACKUP_INVALID_RECORD")?;
            uuid::Uuid::parse_str(&operation).context("BACKUP_INVALID_OPERATION")?;
            self.request(None, move |core, transport, io| {
                Box::pin(async move {
                    super::telegram_lifecycle::reconcile(transport, &target, &asset, io, || {
                        Box::pin(Self::project(core, &target, transport, io, None))
                    })
                    .await
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
