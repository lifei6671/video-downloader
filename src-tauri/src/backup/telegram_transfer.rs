//! Byte-preserving Telegram documents, durable send evidence and verified no-clobber recovery.
use super::{
    provider::{
        MediaLocation, MediaRef, MediaSource, ProviderFuture, ProviderType, Scope, TransferControl,
        UploadReceipt,
    },
    telegram_transfer_journal::{Entry, Journal, SendState},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex},
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, ReadBuf},
    sync::watch,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub scope: Scope,
    pub account_id: String,
    pub channel_id: String,
}
pub struct UploadRequest {
    pub login_id: String,
    pub target: Target,
    pub source: MediaSource,
}
#[derive(Clone, Debug)]
pub struct RestoreRequest {
    pub login_id: String,
    pub target: Target,
    pub reference: MediaRef,
    pub destination: PathBuf,
}
pub(super) enum Job {
    Upload(MediaSource),
    Restore {
        reference: MediaRef,
        destination: PathBuf,
    },
}
pub(super) enum Output {
    Uploaded(UploadReceipt),
    Restored(PathBuf),
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        if self.scope.provider_type != ProviderType::Telegram
            || self.scope.protocol != 2
            || self.scope.space_id.is_empty()
            || self.scope.library_id.is_empty()
            || !positive_id(&self.account_id)
            || !positive_id(&self.channel_id)
        {
            bail!("BACKUP_SCOPE_MISMATCH");
        }
        Ok(())
    }
    pub(super) fn message_id(&self, reference: &MediaRef) -> Result<i32> {
        self.validate()?;
        reference.validate_scope(&self.scope)?;
        match &reference.location {
            MediaLocation::TelegramMessage {
                account_id,
                channel_id,
                message_id,
                document_id,
            } if account_id == &self.account_id
                && channel_id == &self.channel_id
                && *message_id > 0 && *document_id != 0 =>
            {
                Ok(i32::try_from(*message_id).context("BACKUP_MEDIA_PROVIDER_MISMATCH")?)
            }
            _ => bail!("BACKUP_MEDIA_PROVIDER_MISMATCH"),
        }
    }
}
fn positive_id(value: &str) -> bool {
    value
        .parse::<i64>()
        .is_ok_and(|v| v > 0 && v.to_string() == value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub generation: Option<u64>,
    pub yingqu_media: u32,
    pub library_id: String,
    pub asset_id: String,
    pub operation_id: String,
    pub device_id: String,
    pub kind: String,
    pub bytes: u64,
    pub sha256: String,
    pub container: String,
}
impl Envelope {
    fn source(target: &Target, source: &MediaSource) -> Result<Self> {
        if !matches!(
            source.kind.as_str(),
            "video" | "cover" | "transcript" | "library" | "event"
        ) || source.operation_id.is_empty()
            || source.asset_id.is_empty()
            || source.device_id.is_empty()
            || source.bytes == 0
            || source.sha256.len() != 64
            || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || source.container.is_empty()
            || source.container.len() > 12
            || !source.container.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            bail!("BACKUP_INVALID_MEDIA");
        }
        Ok(Self {
            generation: source.recreate_revision,
            yingqu_media: 2,
            library_id: target.scope.library_id.clone(),
            asset_id: source.asset_id.clone(),
            operation_id: source.operation_id.clone(),
            device_id: source.device_id.clone(),
            kind: source.kind.clone(),
            bytes: source.bytes,
            sha256: source.sha256.clone(),
            container: source.container.clone(),
        })
    }
    pub(super) fn caption(&self) -> Result<String> {
        let text = serde_json::to_string(self)?;
        if text.chars().count() > 1024 {
            bail!("BACKUP_INVALID_MEDIA");
        }
        Ok(text)
    }
    fn random_id(&self) -> Result<i64> {
        let hash = Sha256::digest(self.caption()?.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&hash[..8]);
        let value = i64::from_le_bytes(bytes);
        Ok(if value == 0 { 1 } else { value })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct RemoteDocument {
    pub message_id: i32,
    pub document_id: i64,
    pub bytes: u64,
    pub caption: String,
}
impl RemoteDocument {
    pub(super) fn check(&self, envelope: &Envelope) -> Result<()> {
        if self.message_id <= 0
            || self.document_id == 0
            || self.bytes != envelope.bytes
            || self.caption != envelope.caption()?
        {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        Ok(())
    }
}
pub(super) trait Transport: Send + Sync {
    type Uploaded: Send;
    fn max_bytes(&self) -> u64;
    fn can_upload(&self) -> bool;
    fn can_delete(&self) -> bool {
        false
    }
    #[cfg(test)]
    fn media<'a>(&'a self, _io: &'a Io) -> ProviderFuture<'a, Vec<RemoteDocument>> {
        Box::pin(async { bail!("BACKUP_DELETE_UNSUPPORTED") })
    }
    fn remove(&self, _document: RemoteDocument) -> ProviderFuture<'_, ()> {
        Box::pin(async { bail!("BACKUP_DELETE_UNSUPPORTED") })
    }
    fn upload<'a>(
        &'a self,
        reader: &'a mut (dyn AsyncRead + Unpin + Send),
        bytes: u64,
        name: String,
    ) -> ProviderFuture<'a, Self::Uploaded>;
    fn send<'a>(
        &'a self,
        uploaded: Self::Uploaded,
        caption: String,
        random_id: i64,
        video: Option<super::provider::VideoAttributes>,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<i32>>;
    fn inspect(&self, message: i32) -> ProviderFuture<'_, RemoteDocument>;
    fn inspect_document(&self, message:i32, document_id:i64) -> ProviderFuture<'_,RemoteDocument> {
        Box::pin(async move {
            let document=self.inspect(message).await?;
            if document_id!=document.document_id {bail!("TELEGRAM_MEDIA_CHANGED");}
            Ok(document)
        })
    }
    fn inspect_envelope<'a>(&'a self,message:i32,envelope:&'a Envelope) -> ProviderFuture<'a,RemoteDocument> {
        Box::pin(async move {let document=self.inspect(message).await?;document.check(envelope)?;Ok(document)})
    }
    fn inspect_reference<'a>(&'a self,target:&'a Target,reference:&'a MediaRef)->ProviderFuture<'a,RemoteDocument> {
        Box::pin(async move {self.inspect_document(target.message_id(reference)?,reference.document_id().context("BACKUP_MEDIA_PROVIDER_MISMATCH")?).await})
    }
    fn find<'a>(
        &'a self,
        envelope: &'a Envelope,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<RemoteDocument>>;
    fn download(
        &self,
        document: RemoteDocument,
    ) -> ProviderFuture<'_, Box<dyn AsyncRead + Unpin + Send>>;
}

pub(super) struct Io {
    cancel: watch::Receiver<bool>,
    stop: watch::Receiver<bool>,
    control: Option<Arc<TransferControl>>,
    abort: watch::Sender<bool>,
    activity: Mutex<Instant>,
    idle: Duration,
}
impl Drop for Io {
    fn drop(&mut self) {
        self.abort.send_replace(true);
    }
}
impl Io {
    pub(super) fn new(
        cancel: watch::Receiver<bool>,
        stop: watch::Receiver<bool>,
        control: Option<Arc<TransferControl>>,
    ) -> Self {
        Self {
            cancel,
            stop,
            control,
            abort: watch::channel(false).0,
            activity: Mutex::new(Instant::now()),
            idle: Duration::from_secs(60),
        }
    }
    pub(super) fn abort_handle(&self) -> watch::Sender<bool> {
        self.abort.clone()
    }
    pub(super) fn check(&self) -> Result<()> {
        if *self.cancel.borrow()
            || *self.stop.borrow()
            || *self.abort.borrow()
            || self.stop.has_changed().is_err()
        {
            bail!("TELEGRAM_TRANSFER_CANCELED");
        }
        if let Some(control) = &self.control {
            (control.check)()?;
        }
        Ok(())
    }
    pub(super) fn touch(&self) {
        *self
            .activity
            .lock()
            .expect("telegram transfer activity lock poisoned") = Instant::now();
    }
    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        self.touch();
        let mut cancel = self.cancel.clone();
        let mut stop = self.stop.clone();
        let mut abort = self.abort.subscribe();
        tokio::pin!(work);
        let mut check = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {biased;
                _=cancel.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=stop.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=abort.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=check.tick()=>{self.check()?;if self.activity.lock().expect("telegram transfer activity lock poisoned").elapsed()>=self.idle {bail!("TELEGRAM_TRANSFER_TIMEOUT");}},
                result=&mut work=>{self.check()?;return result;}
            }
        }
    }
}
struct SourceReader<'a> {
    file: tokio::fs::File,
    io: &'a Io,
    hash: Sha256,
    bytes: u64,
    progress: Option<&'a AtomicU64>,
}
impl AsyncRead for SourceReader<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if let Err(error) = self.io.check() {
            return Poll::Ready(Err(std::io::Error::other(error)));
        }
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.file).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = &result {
            let bytes = &buffer.filled()[before..];
            self.bytes += bytes.len() as u64;
            self.hash.update(bytes);
            if let Some(progress) = self.progress { progress.store(self.bytes, Ordering::Relaxed); }
            self.io.touch();
        }
        result
    }
}
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn allowed(journal: &impl Journal, target: &Target) -> Result<()> {
    target.validate()?;
    let wait = journal.cooldown(&target.account_id)?.saturating_sub(now());
    if wait > 0 {
        bail!("TELEGRAM_FLOOD_WAIT:{wait}");
    }
    Ok(())
}
fn receipt(target: &Target, entry: &Entry, document: &RemoteDocument) -> UploadReceipt {
    UploadReceipt {
        reference: MediaRef {
            scope: target.scope.clone(),
            location: MediaLocation::TelegramMessage {
                account_id: target.account_id.clone(),
                channel_id: target.channel_id.clone(),
                message_id: i64::from(document.message_id),
                document_id: document.document_id,
            },
            bytes: entry.envelope.bytes,
            sha256: entry.envelope.sha256.clone(),
            container: entry.envelope.container.clone(),
        },
        operation_id: entry.envelope.operation_id.clone(),
        device_id: entry.envelope.device_id.clone(),
    }
}
async fn source_unchanged(source: &MediaSource, io: &Io) -> Result<()> {
    let path = source.path.clone();
    let expected = source.sha256.clone();
    let bytes = source.bytes;
    // Reuse the repository's cancelable file fingerprint; await the blocking worker even on cancellation.
    let cancel = io.cancel.clone();
    let stop = io.stop.clone();
    let abort = io.abort.subscribe();
    let control = io.control.clone();
    let result = tokio::task::spawn_blocking(move || {
        super::restore::regular(&path)?.context("FILE_MISSING")?;
        let (hash, signature) = super::repository::fingerprint_with_cancel(&path, || {
            *cancel.borrow()
                || *stop.borrow()
                || *abort.borrow()
                || stop.has_changed().is_err()
                || control.as_ref().is_some_and(|c| (c.check)().is_err())
        })?;
        if hash != expected || signature["size"].as_u64() != Some(bytes) {
            bail!("BACKUP_SOURCE_CHANGED");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?;
    io.check()?;
    result
}

/// Recognize ISO BMFF metadata, including a trailing index, without changing original bytes.
async fn mp4_has_metadata(path: &Path, io: &Io) -> Result<bool> {
    let mut file = tokio::fs::File::open(path).await.context("IO_ERROR")?;
    let total = file.metadata().await.context("IO_ERROR")?.len();
    let (mut offset, mut ftyp, mut moov, mut mdat) = (0u64, false, false, false);
    while total.saturating_sub(offset) >= 8 {
        io.check()?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .context("IO_ERROR")?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).await.context("IO_ERROR")?;
        let length = u32::from_be_bytes(header[..4].try_into()?) as u64;
        let (length, header_bytes) = if length == 1 {
            if total - offset < 16 {
                return Ok(false);
            }
            let mut extended = [0u8; 8];
            file.read_exact(&mut extended).await.context("IO_ERROR")?;
            (u64::from_be_bytes(extended), 16)
        } else if length == 0 {
            (total - offset, 8)
        } else {
            (length, 8)
        };
        if length < header_bytes || length > total - offset {
            return Ok(false);
        }
        match &header[4..] {
            b"ftyp" => ftyp = true,
            b"moov" => moov = ftyp,
            b"mdat" if length > header_bytes => mdat = true,
            _ => {}
        }
        offset += length;
        // Telegram's remote reader can seek to a trailing moov without downloading all media.
        if ftyp && moov && mdat {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) async fn prepare_video(
    core: &crate::desktop_core::Core,
    source: &mut MediaSource,
    io: &Io,
) -> Result<()> {
    if source.kind != "video" || source.video.is_some() {
        return Ok(());
    }
    io.check()?;
    if super::telegram_transfer_journal::CoreJournal(core)
        .get(&source.operation_id, &source.kind)?
        .is_some_and(|entry| entry.state != SendState::Prepared)
    {
        return Ok(());
    }
    let result = core
        .probe_video_metadata(&source.path, &source.operation_id, async {
            loop {
                if io.check().is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
    io.check()?;
    source.video = Some(
        super::provider::VideoAttributes::from_metadata(&result?)
            .context("VIDEO_METADATA_UNAVAILABLE")?,
    );
    io.touch();
    Ok(())
}
pub(super) async fn upload(
    transport: &impl Transport,
    journal: &impl Journal,
    target: &Target,
    source: MediaSource,
    io: &Io,
) -> Result<UploadReceipt> {
    upload_with_timing(transport, journal, target, source, io, Default::default()).await
}

pub(super) async fn upload_with_timing(
    transport: &impl Transport,
    journal: &impl Journal,
    target: &Target,
    source: MediaSource,
    io: &Io,
    timing: super::delete_timing::IndexTiming<'_>,
) -> Result<UploadReceipt> {
    allowed(journal, target)?;
    io.check()?;
    let envelope = Envelope::source(target, &source)?;
    let mut entry = journal.reserve(Entry {
        target: target.clone(),
        random_id: envelope.random_id()?,
        envelope,
        state: SendState::Prepared,
        document: None,
        last_error: None,
    })?;
    if let Some(control) = &source.control {
        (control.checkpoint)(super::provider::OpaqueState {scope:target.scope.clone(),value:serde_json::json!({"operation_id":source.operation_id,"asset_id":source.asset_id,"kind":source.kind,"device_id":source.device_id}).to_string()})?;
    }
    let result = async {
        if entry.state == SendState::Confirmed {
            let document = timing.run("receipt", async {
                let document = io
                    .run(
                        transport.inspect_envelope(
                            entry
                                .document
                                .as_ref()
                                .context("TELEGRAM_SEND_EVIDENCE_INVALID")?
                                .message_id,
                            &entry.envelope,
                        ),
                    )
                    .await?;
                document.check(&entry.envelope)?;
                if Some(&document) != entry.document.as_ref() {
                    bail!("TELEGRAM_MEDIA_CHANGED");
                }
                Ok(document)
            }).await?;
            return Ok(receipt(target, &entry, &document));
        }
        let mut sent_message = None;
        if entry.state == SendState::Prepared {
            if !transport.can_upload() {
                bail!("TELEGRAM_CHANNEL_FORBIDDEN");
            }
            if source.bytes > transport.max_bytes() {
                bail!("TELEGRAM_FILE_TOO_LARGE");
            }
            source_unchanged(&source, io).await?;
            let mut video = if source.kind == "video" {
                Some(source.video.clone().context("VIDEO_METADATA_UNAVAILABLE")?)
            } else { None };
            if let Some(video) = &mut video {
                video.supports_streaming = matches!(source.container.as_str(), "mp4" | "mov" | "m4v" | "3gp" | "3g2" | "mj2")
                    && io.run(mp4_has_metadata(&source.path, io)).await?;
            }
            let file = tokio::fs::File::open(&source.path)
                .await
                .context("IO_ERROR")?;
            let mut reader = SourceReader {
                file,
                io,
                hash: Sha256::new(),
                bytes: 0,
                progress: source.control.as_ref().map(|control| control.progress.as_ref()),
            };
            let uploaded = timing.run("upload", io.run(transport.upload(
                    &mut reader,
                    source.bytes,
                    format!("yingqu-{}.{}", source.kind, source.container),
                ))).await?;
            if reader.bytes != source.bytes
                || format!("{:x}", reader.hash.finalize()) != source.sha256
            {
                bail!("BACKUP_SOURCE_CHANGED");
            }
            if let Some(control) = &source.control {
                control.progress.store(source.bytes, Ordering::Relaxed);
            }
            source_unchanged(&source, io).await?;
            io.check()?;
            entry.state = SendState::Sending;
            journal.save(&entry)?;
            match timing.run("send", io.run(transport.send(uploaded, entry.envelope.caption()?, entry.random_id, video, io))).await
            {
                Ok(message) => sent_message = message,
                Err(error) => {
                    let code = error.to_string();
                    // These RPC replies explicitly reject publication; transport failures stay uncertain.
                    if matches!(
                        code.as_str(),
                        "TELEGRAM_CHANNEL_FORBIDDEN"
                            | "TELEGRAM_AUTH_REQUIRED"
                            | "TELEGRAM_FILE_TOO_LARGE"
                    ) || code
                        .strip_prefix("TELEGRAM_FLOOD_WAIT:")
                        .is_some_and(|v| v.parse::<u32>().is_ok())
                    {
                        entry.state = SendState::Prepared;
                    }
                    return Err(error);
                }
            }
        }
        // Sending also covers a response lost before a crash/cancel. Never retransmit without evidence.
        let document = timing.run("receipt", async {
            let document = if let Some(id) = sent_message {
                io.run(transport.inspect_envelope(id,&entry.envelope)).await?
            } else {
                io.run(transport.find(&entry.envelope, io))
                    .await?
                    .context("TELEGRAM_SEND_UNCONFIRMED")?
            };
            document.check(&entry.envelope)?;
            let readback = io.run(transport.inspect_envelope(document.message_id,&entry.envelope)).await?;
            readback.check(&entry.envelope)?;
            if readback != document {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            Ok(document)
        }).await?;
        io.check()?;
        entry.state = SendState::Confirmed;
        entry.document = Some(document.clone());
        entry.last_error = None;
        journal.save(&entry)?;
        Ok(receipt(target, &entry, &document))
    }
    .await;
    if let Err(error) = &result {
        entry.last_error = Some(error.to_string());
        journal.save(&entry)?;
        journal.failure(&target.account_id, &error.to_string())?;
    }
    result
}

/// Recover bytes into a same-directory managed temporary file, then commit without overwriting.
pub(super) async fn restore(
    transport: &impl Transport,
    journal: &impl Journal,
    target: &Target,
    reference: &MediaRef,
    destination: &Path,
    io: &Io,
) -> Result<PathBuf> {
    allowed(journal, target)?;
    target.message_id(reference)?;
    if reference.bytes == 0
        || reference.sha256.len() != 64
        || !reference.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("BACKUP_INVALID_MEDIA");
    }
    if destination.exists() {
        bail!("TELEGRAM_DESTINATION_EXISTS");
    }
    let result = async {
        let parent = destination.parent().context("IO_ERROR")?;
        let mut staged = tempfile::NamedTempFile::new_in(parent).context("IO_ERROR")?;
        for attempt in 0..2 {
            io.check()?;
            let document = io.run(transport.inspect_reference(target,reference)).await?;
            let envelope: Envelope =
                serde_json::from_str(&document.caption).context("TELEGRAM_MEDIA_CHANGED")?;
            if envelope.yingqu_media != 2
                || envelope.library_id != target.scope.library_id
                || envelope.bytes != reference.bytes
                || envelope.sha256 != reference.sha256
                || envelope.container != reference.container
            {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            document.check(&envelope)?;
            staged.as_file_mut().set_len(0).context("IO_ERROR")?;
            use std::io::{Seek, SeekFrom};
            staged.as_file_mut().seek(SeekFrom::Start(0))?;
            let mut output = tokio::fs::File::from_std(staged.as_file().try_clone()?);
            let download = async {
                let mut reader = io.run(transport.download(document)).await?;
                let mut hash = Sha256::new();
                let mut bytes = 0_u64;
                let mut buffer = vec![0_u8; 64 * 1024];
                loop {
                    let count = io
                        .run(async {
                            reader
                                .read(&mut buffer)
                                .await
                                .map_err(|e| anyhow::anyhow!(e))
                        })
                        .await?;
                    if count == 0 {
                        break;
                    }
                    io.touch();
                    bytes += count as u64;
                    if bytes > reference.bytes {
                        bail!("BACKUP_HASH_MISMATCH");
                    }
                    hash.update(&buffer[..count]);
                    output
                        .write_all(&buffer[..count])
                        .await
                        .context("IO_ERROR")?;
                }
                if bytes != reference.bytes || format!("{:x}", hash.finalize()) != reference.sha256
                {
                    bail!("BACKUP_HASH_MISMATCH");
                }
                output.sync_all().await.context("IO_ERROR")?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            drop(output);
            match download {
                Err(error)
                    if attempt == 0 && error.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" =>
                {
                    continue
                }
                other => other?,
            }
            io.check()?;
            staged.persist_noclobber(destination).map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    anyhow::anyhow!("TELEGRAM_DESTINATION_EXISTS")
                } else {
                    anyhow::anyhow!("IO_ERROR")
                }
            })?;
            return Ok(destination.to_owned());
        }
        bail!("TELEGRAM_FILE_REFERENCE_EXPIRED")
    }
    .await;
    if let Err(error) = &result {
        journal.failure(&target.account_id, &error.to_string())?;
    }
    result
}

#[cfg(test)]
#[path = "telegram_transfer_tests.rs"]
mod tests;
