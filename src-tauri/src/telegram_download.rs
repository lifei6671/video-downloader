//! Telegram single-message downloads through grammers, using the existing task queue.
use crate::backup::{provider::ProviderFuture, telegram_session::{rpc_error, retry_read}};
use anyhow::{bail, Context, Result};
use grammers_client::{
    media::{Media as SdkMedia, Photo, PhotoSize},
    peer::Peer,
    tl, Client,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::watch,
    time::timeout,
};
pub const PROTOCOL_REVISION: &str = "grammers-0.11-download-v3";
pub(crate) const ALIAS_IDENTITY_MISMATCH: &str = "TELEGRAM_ALIAS_IDENTITY_MISMATCH";
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
#[derive(Debug)]
pub(crate) struct MessageLink {
    pub channel: Option<u64>,
    pub message: u32,
    pub url: String,
}

fn positive(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) || text.starts_with('0') {
        return None;
    }
    text.parse::<u64>()
        .ok()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
}

pub fn is_telegram(input: &str) -> bool {
    crate::parse_cache::share_url(input)
        .and_then(|s| url::Url::parse(&s).ok())
        .is_some_and(|u| matches!(u.host_str(), Some("t.me" | "telegram.me")))
}

pub(crate) fn message_link(input: &str) -> Result<MessageLink> {
    // Extract just one link; keep the original query for validation, unlike cache lookup normalization.
    let start = input.find("https://").context("INVALID_INPUT")?;
    let end = input[start..]
        .find(char::is_whitespace)
        .map_or(input.len(), |n| start + n);
    let text = input[start..end].trim_end_matches(['。', '，', ')', '）']);
    if input[end..].contains("https://") || input[..start].contains("http://") {
        bail!("INVALID_INPUT");
    }
    let u = url::Url::parse(text).context("INVALID_INPUT")?;
    if u.scheme() != "https"
        || !matches!(u.host_str(), Some("t.me" | "telegram.me"))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || text.contains('%')
        || text.contains('\\')
    {
        bail!("INVALID_INPUT");
    }
    let parts: Vec<_> = u.path().trim_start_matches('/').split('/').collect();
    let (channel, msg) = match parts.as_slice() {
        ["c", channel, msg] => (Some(positive(channel).context("INVALID_INPUT")?), *msg),
        [name, msg]
            if (1..=32).contains(&name.len())
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !matches!(*name, "c" | "s" | "joinchat") =>
        {
            (None, *msg)
        }
        _ => bail!("INVALID_INPUT"),
    };
    let message =
        u32::try_from(positive(msg).context("INVALID_INPUT")?).context("INVALID_INPUT")?;
    if message > i32::MAX as u32 {
        bail!("INVALID_INPUT");
    }
    Ok(MessageLink {
        channel,
        message,
        url: format!("https://t.me{}", u.path()),
    })
}

pub fn validate_identity(id: &str, canonical: &str) -> bool {
    let Ok(link) = message_link(canonical) else {
        return false;
    };
    link.channel
        .is_some_and(|channel| id == format!("{channel}_{}", link.message))
}

pub(crate) fn public_peer(link: &MessageLink) -> Option<String> {
    (link.channel.is_none()).then(|| link.url.split('/').nth(3).unwrap().to_ascii_lowercase())
}

#[derive(Clone, Debug)]
pub(crate) struct Media {
    pub channel: u64,
    pub channel_title: Option<String>,
    pub message: u32,
    pub document: i64,
    pub size: u64,
    pub mime: String,
    pub name: String,
    pub container: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
}
impl PartialEq for Media {
    fn eq(&self, other: &Self) -> bool {
        // A channel rename cannot change the identity of an in-flight document.
        self.channel == other.channel && self.message == other.message
            && self.document == other.document && self.size == other.size
            && self.mime == other.mime && self.name == other.name
            && self.container == other.container && self.width == other.width
            && self.height == other.height && self.duration_ms == other.duration_ms
    }
}
#[derive(Debug)]
pub(crate) struct TransferResult { pub value: Value, pub cover: Option<Vec<u8>> }
pub(crate) struct ReceivedPart { offset: u64, bytes: Vec<u8> }
pub(crate) trait Transport: Send {
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media>;
    fn next(&mut self, offset: u64) -> ProviderFuture<'_, Option<ReceivedPart>>;
    fn cover(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> { Box::pin(async { Ok(None) }) }
}
#[cfg(test)]
pub(crate) type Factory = std::sync::Arc<dyn Fn() -> Box<dyn Transport> + Send + Sync>;
struct SdkTransport {
    client: Client,
    iter: Option<crate::telegram_transfer_engine::FileDownload>,
    budget: std::sync::Arc<crate::telegram_transfer_engine::Budget>,
    covers: Vec<PhotoSize>,
}
impl Transport for SdkTransport {
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async move {
            let (peer, channel_title) = if let Some(channel) = link.channel {
                let mut dialogs = self.client.iter_dialogs();
                let mut found = None;
                while let Some(dialog) = dialogs.next().await.map_err(rpc_error)? {
                    if matches!(dialog.peer(), Peer::Channel(_))
                        && dialog.peer_id().bare_id_unchecked() as u64 == channel
                    {
                        let Peer::Channel(channel) = dialog.peer() else { unreachable!("checked channel peer"); };
                        found = Some((dialog.peer_ref(), channel.title().trim().to_owned()));
                        break;
                    }
                }
                found.context("TELEGRAM_CHANNEL_FORBIDDEN")?
            } else {
                let name = public_peer(link).context("INVALID_INPUT")?;
                let peer = self
                    .client
                    .resolve_username(&name)
                    .await
                    .map_err(rpc_error)?
                    .context("TELEGRAM_CHANNEL_FORBIDDEN")?;
                let Peer::Channel(channel) = &peer else { bail!("UNSUPPORTED_CONTENT"); };
                let title = channel.title().trim().to_owned();
                let peer = peer.to_ref()
                    .await
                    .map_err(|_| anyhow::anyhow!("TELEGRAM_CHANNEL_FORBIDDEN"))?
                    .context("TELEGRAM_CHANNEL_FORBIDDEN")?;
                (peer, title)
            };
            let message = self
                .client
                .get_messages_by_id(peer, &[link.message as i32])
                .await
                .map_err(rpc_error)?
                .into_iter()
                .next()
                .flatten()
                .context("TELEGRAM_MEDIA_MISSING")?;
            // A grouped_id also appears on individual video messages. The requested
            // message remains the only download target; never expand the group.
            let media = message.media().context("TELEGRAM_MEDIA_MISSING")?;
            let doc = match media {
                SdkMedia::Document(doc) => doc,
                _ => bail!("UNSUPPORTED_CONTENT"),
            };
            self.covers = cover_candidates(&doc);
            let Some(tl::enums::Document::Document(raw)) = doc.raw.document else {
                bail!("TELEGRAM_MEDIA_MISSING");
            };
            let mut result = media_info(peer.id.bare_id_unchecked() as u64, link.message, &raw)?;
            result.channel_title = (!channel_title.is_empty()).then_some(channel_title);
            let client = self.client.clone();
            let message_id = link.message as i32;
            let refresh: crate::telegram_transfer_engine::Refresh = std::sync::Arc::new(move || {
                let client = client.clone();
                Box::pin(async move {
                    let message = retry_read(|| async { client.get_messages_by_id(peer, &[message_id]).await.map_err(rpc_error) }).await?
                        .into_iter().next().flatten().context("TELEGRAM_MEDIA_MISSING")?;
                    let Some(SdkMedia::Document(doc)) = message.media() else { bail!("TELEGRAM_MEDIA_CHANGED"); };
                    let Some(tl::enums::Document::Document(raw)) = doc.raw.document else { bail!("TELEGRAM_MEDIA_CHANGED"); };
                    Ok(raw)
                })
            });
            self.iter = Some(crate::telegram_transfer_engine::download_file(self.client.clone(), raw, refresh, self.budget.clone())?);
            Ok(result)
        })
    }
    fn next(&mut self, _: u64) -> ProviderFuture<'_, Option<ReceivedPart>> {
        Box::pin(async {
            Ok(self.iter.as_mut().context("TELEGRAM_MEDIA_MISSING")?.next().await?.map(|(chunk, bytes)| ReceivedPart {offset: chunk.offset, bytes}))
        })
    }
    fn cover(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> {
        Box::pin(async {
            let client = &self.client;
            let budget = &self.budget;
            first_available_cover(&self.covers, move |candidate| Box::pin(async move {
                let mut download = client.iter_download(candidate);
                let mut body = vec![];
                while let Some(chunk) = crate::telegram_transfer_engine::cover_chunk(budget, download.next()).await? {
                    anyhow::ensure!(body.len() + chunk.len() <= crate::telegram_cover::MAX_BYTES, "COVER_UNAVAILABLE");
                    body.extend_from_slice(&chunk);
                }
                Ok(body)
            })).await
        })
    }
}

async fn first_available_cover<'a>(candidates: &'a [PhotoSize], read: impl Fn(&'a PhotoSize) -> ProviderFuture<'a, Vec<u8>>) -> Result<Option<Vec<u8>>> {
    for candidate in candidates {
        if candidate.size() == 0 || candidate.size() > crate::telegram_cover::MAX_BYTES { continue; }
        match read(candidate).await {
            Ok(body) if crate::telegram_cover::valid(&body) => return Ok(Some(body)),
            Err(error) if matches!(error.to_string().as_str(), "TELEGRAM_AUTH_REQUIRED" | "CANCELED") => return Err(error),
            _ => {},
        }
    }
    Ok(None)
}

fn cover_candidates(doc: &grammers_client::media::Document) -> Vec<PhotoSize> {
    fn ranked(mut values: Vec<PhotoSize>) -> Vec<PhotoSize> {
        values.retain(|size| !matches!(size, PhotoSize::Empty(_) | PhotoSize::Path(_)));
        values.sort_by_key(|size| std::cmp::Reverse(size.size()));
        values
    }
    let mut covers = doc.raw.video_cover.clone().map(|photo| ranked(Photo::from_raw(photo).thumbs())).unwrap_or_default();
    covers.extend(ranked(doc.thumbs()));
    covers
}
fn media_info(channel: u64, message: u32, raw: &tl::types::Document) -> Result<Media> {
    let container = match raw.mime_type.as_str() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        _ => bail!("INVALID_MEDIA_CONTENT"),
    };
    let size = u64::try_from(raw.size)
        .ok()
        .filter(|n| *n >= 12)
        .context("INVALID_MEDIA_CONTENT")?;
    let name = raw
        .attributes
        .iter()
        .find_map(|a| match a {
            tl::enums::DocumentAttribute::Filename(f) => Some(f.file_name.as_str()),
            _ => None,
        })
        .unwrap_or("video");
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches([' ', '.']);
    Ok(Media {
        channel,
        channel_title: None,
        message,
        document: raw.id,
        size,
        mime: raw.mime_type.clone(),
        name: if name.is_empty() {
            format!("video.{container}")
        } else {
            name.into()
        },
        container: container.into(),
        width: raw.attributes.iter().find_map(|a| match a { tl::enums::DocumentAttribute::Video(v) => u32::try_from(v.w).ok().filter(|n| *n > 0), _ => None }),
        height: raw.attributes.iter().find_map(|a| match a { tl::enums::DocumentAttribute::Video(v) => u32::try_from(v.h).ok().filter(|n| *n > 0), _ => None }),
        duration_ms: raw.attributes.iter().find_map(|a| match a { tl::enums::DocumentAttribute::Video(v) if v.duration.is_finite() && v.duration > 0.0 && v.duration < u64::MAX as f64 / 1000.0 => Some((v.duration * 1000.0).round().max(1.0) as u64), _ => None }),
    })
}
fn preview(media: &Media, link: &MessageLink) -> Value {
    json!({"platform":"telegram","resolver":"grammers","kind":"video","video_id":format!("{}_{}",media.channel,media.message),
        "canonical_url":format!("https://t.me/c/{}/{}",media.channel,media.message),"title":media.name,"original_filename":media.name,"mime_type":media.mime,
        "author":media.channel_title,"author_profile":{"id":media.channel.to_string(),"nickname":media.channel_title,"handle":public_peer(link),"kind":"channel","avatar_cached":false},
        "filesize":media.size,"duration_ms":media.duration_ms,"width":media.width,"height":media.height,"selected_format_id":"original","telegram_document_id":media.document,
        "formats":[{"format_id":"original","container":media.container,"codec":null,"width":media.width,"height":media.height,"filesize":media.size}]})
}
pub(crate) fn merge_attributes(video: &mut Value, metadata: &Value) {
    for field in ["width", "height", "duration_ms"] {
        if metadata[field].as_u64().is_some_and(|n| n > 0) { video[field] = metadata[field].clone(); }
    }
    merge_channel_title(video, metadata);
    let width = video["width"].clone(); let height = video["height"].clone();
    if let Some(formats) = video["formats"].as_array_mut() {
        for format in formats.iter_mut().filter(|format| format["format_id"] == "original") { format["width"] = width.clone(); format["height"] = height.clone(); }
    }
}
pub(crate) fn fill_missing_attributes(video: &mut Value, metadata: &Value) {
    let mut missing = serde_json::Map::new();
    for field in ["width", "height", "duration_ms"] {
        if video[field].as_u64().is_none_or(|n| n == 0) {
            missing.insert(field.into(), metadata[field].clone());
        }
    }
    merge_attributes(video, &Value::Object(missing));
    merge_channel_title(video, metadata);
}
fn merge_channel_title(video: &mut Value, metadata: &Value) {
    if video["platform"] != "telegram" || video["video_id"] != metadata["video_id"] { return; }
    let Some(channel) = video["video_id"].as_str().and_then(|id| id.split_once('_')).map(|v| v.0) else { return; };
    if metadata["author_profile"]["id"].as_str() != Some(channel) { return; }
    let Some(title) = metadata["author_profile"]["nickname"].as_str().map(str::trim).filter(|s| !s.is_empty()) else { return; };
    if video["author_profile"]["id"].as_str() != Some(channel) {
        video["author_profile"] = metadata["author_profile"].clone();
    } else {
        video["author_profile"]["nickname"] = title.into();
        if metadata["author_profile"]["handle"].as_str().is_some_and(|s| !s.is_empty()) {
            video["author_profile"]["handle"] = metadata["author_profile"]["handle"].clone();
        }
    }
    video["author"] = title.into();
}
fn check_expected(media: &Media, expected: &Value) -> Result<()> {
    if expected["video_id"] != format!("{}_{}", media.channel, media.message) {
        bail!(ALIAS_IDENTITY_MISMATCH);
    }
    if expected["selected_format_id"] != "original" {
        bail!("FORMAT_UNAVAILABLE");
    }
    if expected["filesize"] != media.size
        || expected["mime_type"] != media.mime
        || expected["original_filename"] != media.name
        || expected["telegram_document_id"]
            .as_i64()
            .is_some_and(|id| id != media.document)
    {
        bail!("TELEGRAM_MEDIA_CHANGED");
    }
    Ok(())
}
pub(crate) async fn operation(
    core: &std::sync::Arc<crate::desktop_core::Core>,
    task_id: &str,
    input: &str,
    expected: Option<&Value>,
    directory: Option<&Path>,
    mut progress: impl FnMut(Value),
    mut cancel: watch::Receiver<bool>,
) -> Result<TransferResult> {
    let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download);
    loop {
        let current = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download);
        if !std::sync::Arc::ptr_eq(&binding.session, &current.session) { bail!("TELEGRAM_ACCOUNT_MISMATCH"); }
        if binding.session.reconnect.waiting() {
            progress(json!({"reconnecting":true}));
            crate::telegram_accounts::recover_network(core, crate::telegram_accounts::Purpose::Download, cancel.clone()).await?;
        }
        let mut network = binding.session.network_cancellation();
        core.wait_telegram_slot(task_id, cancel.clone()).await?;
        progress(json!({"reconnecting":false}));
        let result = if binding.session.network_change_pending() {
            Err(anyhow::anyhow!("TELEGRAM_NETWORK_CHANGED"))
        } else {
            Box::pin(operation_once(core, input, expected, directory, &mut progress, cancel.clone())).await
        };
        let reason = result.as_ref().err().map(ToString::to_string);
        if reason.as_deref().is_some_and(crate::telegram_reconnect::network_error) {
            binding.session.reconnect.disconnected();
            progress(json!({"reconnecting":true}));
            crate::telegram_accounts::recover_network(core, crate::telegram_accounts::Purpose::Download, cancel.clone()).await?;
            continue;
        }
        if !reason.as_deref().is_some_and(|reason| reason == "TELEGRAM_NETWORK_CHANGED"
            || reason == "TELEGRAM_BUSY" && (network.has_changed().unwrap_or(false) || binding.session.network_change_pending())) { return result; }
        tokio::select! { biased;
            _=cancel.wait_for(|v|*v)=>bail!("CANCELED"),
            _=network.changed()=>{},
            _=tokio::time::sleep(Duration::from_millis(500))=>{},
        }
    }
}
async fn operation_once(
    core: &crate::desktop_core::Core,
    input: &str,
    expected: Option<&Value>,
    directory: Option<&Path>,
    progress: impl FnMut(Value),
    mut cancel: watch::Receiver<bool>,
) -> Result<TransferResult> {
    let link = message_link(input)?;
    #[cfg(test)]
    let factory = core
        .telegram_source
        .lock()
        .expect("telegram test factory lock poisoned")
        .clone();
    #[cfg(not(test))]
    let factory: Option<fn() -> Box<dyn Transport>> = None;
    let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download);
    let mut network_stop = binding.session.network_cancellation();
    let connection = if factory.is_some() {
        None
    } else {
        let ready = tokio::select! { biased;
            _=cancel.wait_for(|v|*v)=>bail!("CANCELED"),
            _=network_stop.changed()=>bail!("TELEGRAM_NETWORK_CHANGED"),
            result=binding.session.resume(binding.namespace)=>result?,
        };
        if !matches!(ready.phase, crate::backup::telegram_session::Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        Some(if expected.is_none() {
            binding.session.metadata_connection()?
        } else {
            binding.session.download_connection()?
        })
    };
    let authorization_id = binding.session.status().operation_id;
    let mut source: Box<dyn Transport> = if let Some(factory) = factory {
        factory()
    } else {
        Box::new(SdkTransport {
            client: connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?
                .client(),
            iter: None,
            budget: crate::telegram_transfer_engine::Budget::account(&binding.session.status().account.context("TELEGRAM_AUTH_REQUIRED")?.id),
            covers: vec![],
        })
    };
    let (_stop, mut stop) = watch::channel(false);
    let (_connection_stop, mut connection_stop) = watch::channel(false);
    if let Some(connection) = &connection {
        stop = connection.cancellation();
        connection_stop = connection.stopped();
    }
    let result = tokio::select! { biased;
        _=cancel.wait_for(|v|*v)=>Err(anyhow::anyhow!("CANCELED")),
        _=stop.wait_for(|v|*v)=>Err(anyhow::anyhow!("TELEGRAM_AUTH_REQUIRED")),
        _=network_stop.changed()=>Err(anyhow::anyhow!("TELEGRAM_NETWORK_CHANGED")),
        _=connection_stop.wait_for(|v|*v)=>Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR")),
        result=timeout(Duration::from_secs(6*60*60), transfer(source.as_mut(), &link, expected, directory, progress))=>result.context("TIMEOUT").and_then(|v|v),
    };
    drop(source);
    if let Some(connection) = connection {
        connection.close().await;
    }
    if result.as_ref().is_err_and(|error| error.to_string() == "TELEGRAM_AUTH_REQUIRED") {
        if let Some(id) = authorization_id { binding.session.authorization_failed(&id).await; }
    }
    result
}
struct DownloadProgress {
    samples: VecDeque<(Instant, u64)>,
    last: Instant,
}
impl DownloadProgress {
    fn new(now: Instant) -> Self {
        Self {
            samples: VecDeque::from([(now, 0)]),
            last: now,
        }
    }
    fn sample(&mut self, now: Instant, received: u64, total: u64) -> Option<Value> {
        if now.duration_since(self.last) < Duration::from_secs(1) && received != total {
            return None;
        }
        self.samples.push_back((now, received));
        while self.samples.len() > 2
            && now.duration_since(self.samples[1].0) >= Duration::from_secs(5)
        {
            self.samples.pop_front();
        }
        let (at, count) = self.samples.front().expect("progress origin");
        let speed = (received - count) as f64 / now.duration_since(*at).as_secs_f64().max(0.001);
        self.last = now;
        Some(
            json!({"bytes":received,"total_bytes":total,"speed_bps":speed,"eta_seconds":if speed>0.0 {Some(total.saturating_sub(received) as f64/speed)} else {None}}),
        )
    }
}

async fn transfer(
    source: &mut dyn Transport,
    link: &MessageLink,
    expected: Option<&Value>,
    directory: Option<&Path>,
    mut progress: impl FnMut(Value),
) -> Result<TransferResult> {
    let mut media = timeout(IDLE_TIMEOUT, source.inspect(link))
        .await
        .context("TELEGRAM_TIMEOUT")??;
    if let Some(expected) = expected { check_expected(&media, expected)?; }
    let cover = match timeout(Duration::from_secs(20), source.cover()).await {
        Ok(Ok(cover)) => cover,
        Ok(Err(error)) if matches!(error.to_string().as_str(), "TELEGRAM_AUTH_REQUIRED" | "CANCELED") => return Err(error),
        _ => None,
    };
    if expected.is_none() {
        return Ok(TransferResult { value: preview(&media, link), cover });
    };
    let directory = directory.context("IO_ERROR")?;
    // Acquire exclusive creation and cleanup ownership together, without a
    // cancellation point between them. Each attempt owns its own staging path.
    let staged = tempfile::Builder::new()
        .prefix("video-")
        .suffix(&format!(".{}.part", media.container))
        .tempfile_in(directory)
        .context("IO_ERROR")?;
    let (file, partial) = staged.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut refreshed = false;
    let mut received = 0;
    let mut head = Vec::new();
    let mut telemetry = DownloadProgress::new(Instant::now());
    loop {
        let next = {
            let next = timeout(IDLE_TIMEOUT, source.next(received));
            tokio::pin!(next);
            loop {
                tokio::select! {
                    result = &mut next => break result.context("TELEGRAM_TIMEOUT")?,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        if let Some(value) = telemetry.sample(Instant::now(), received, media.size) { progress(value); }
                    }
                }
            }
        };
        let part = match next {
            Err(e) if e.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" && !refreshed => {
                let fresh = timeout(IDLE_TIMEOUT, source.inspect(link))
                    .await
                    .context("TELEGRAM_TIMEOUT")??;
                if fresh != media {
                    bail!("TELEGRAM_MEDIA_CHANGED");
                }
                media = fresh;
                refreshed = true;
                file.set_len(0).await.context("IO_ERROR")?;
                file.rewind().await.context("IO_ERROR")?;
                received = 0;
                head.clear();
                telemetry = DownloadProgress::new(Instant::now());
                progress(
                    json!({"bytes":0,"total_bytes":media.size,"speed_bps":null,"eta_seconds":null}),
                );
                continue;
            }
            other => other?,
        };
        let Some(ReceivedPart {offset, bytes}) = part else {
            break;
        };
        if bytes.is_empty() || received + bytes.len() as u64 > media.size || offset.checked_add(bytes.len() as u64).is_none_or(|end|end > media.size) {
            bail!("INCOMPLETE_MEDIA");
        }
        file.seek(std::io::SeekFrom::Start(offset)).await.context("IO_ERROR")?;
        file.write_all(&bytes).await.context("IO_ERROR")?;
        received += bytes.len() as u64;
        if offset == 0 {
            head.clear();
            head.extend_from_slice(&bytes[..bytes.len().min(12 - head.len())]);
        }
        if let Some(value) = telemetry.sample(Instant::now(), received, media.size) { progress(value); }
    }
    if received != media.size {
        bail!("INCOMPLETE_MEDIA");
    }
    if !crate::files::video_header_valid(&media.container, &head) {
        bail!("INVALID_MEDIA_CONTENT");
    }
    file.sync_all().await.context("IO_ERROR")?;
    drop(file);
    let filename = format!("video.{}", media.container);
    // Task-owned staging; the final user-visible move remains create-if-absent in files.rs.
    tokio::fs::rename(&partial, directory.join(&filename))
        .await
        .context("IO_ERROR")?;
    let display = preview(&media, link);
    Ok(TransferResult { value: json!({"platform":"telegram","video_id":format!("{}_{}",media.channel,media.message),"container":media.container,"filename":filename,"bytes":received,"format_id":"original","width":media.width,"height":media.height,"duration_ms":media.duration_ms,"author":display["author"],"author_profile":display["author_profile"]}), cover })
}
#[cfg(test)]
#[path = "telegram_download_tests.rs"]
mod tests;
