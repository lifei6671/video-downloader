//! Telegram single-message downloads through grammers, using the existing task queue.
use crate::backup::{provider::ProviderFuture, telegram_session::rpc_error};
use anyhow::{bail, Context, Result};
use grammers_client::{
    client::DownloadIter,
    media::{Downloadable, Media as SdkMedia},
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
pub const PROTOCOL_REVISION: &str = "grammers-0.11-download-v1";
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Media {
    pub channel: u64,
    pub message: u32,
    pub document: i64,
    pub size: u64,
    pub mime: String,
    pub name: String,
    pub container: String,
}
pub(crate) trait Transport: Send {
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media>;
    fn next(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>>;
}
#[cfg(test)]
pub(crate) type Factory = std::sync::Arc<dyn Fn() -> Box<dyn Transport> + Send + Sync>;
struct OriginalDocument(tl::types::Document);
impl Downloadable for OriginalDocument {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputDocumentFileLocation {
                id: self.0.id,
                access_hash: self.0.access_hash,
                file_reference: self.0.file_reference.clone(),
                thumb_size: String::new(),
            }
            .into(),
        )
    }
    fn size(&self) -> Option<usize> {
        usize::try_from(self.0.size).ok()
    }
    fn dc_id(&self) -> Option<i32> {
        Some(self.0.dc_id)
    }
}
struct SdkTransport {
    client: Client,
    iter: Option<DownloadIter>,
}
impl Transport for SdkTransport {
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async move {
            let peer = if let Some(channel) = link.channel {
                let mut dialogs = self.client.iter_dialogs();
                let mut found = None;
                while let Some(dialog) = dialogs.next().await.map_err(rpc_error)? {
                    if matches!(dialog.peer(), Peer::Channel(_))
                        && dialog.peer_id().bare_id_unchecked() as u64 == channel
                    {
                        found = Some(dialog.peer_ref());
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
                if !matches!(peer, Peer::Channel(_)) {
                    bail!("UNSUPPORTED_CONTENT");
                }
                peer.to_ref()
                    .await
                    .map_err(|_| anyhow::anyhow!("TELEGRAM_CHANNEL_FORBIDDEN"))?
                    .context("TELEGRAM_CHANNEL_FORBIDDEN")?
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
            let Some(tl::enums::Document::Document(raw)) = doc.raw.document else {
                bail!("TELEGRAM_MEDIA_MISSING");
            };
            let result = media_info(peer.id.bare_id_unchecked() as u64, link.message, &raw)?;
            self.iter = Some(self.client.iter_download(&OriginalDocument(raw)));
            Ok(result)
        })
    }
    fn next(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> {
        Box::pin(async {
            self.iter.as_mut().context("TELEGRAM_MEDIA_MISSING")?.next().await.map_err(|e| {
            if matches!(&e, grammers_client::InvocationError::Rpc(r) if r.name == "FILE_REFERENCE_EXPIRED") { anyhow::anyhow!("TELEGRAM_FILE_REFERENCE_EXPIRED") } else if matches!(&e, grammers_client::InvocationError::Rpc(r) if r.name.starts_with("FILE_REFERENCE_")) { anyhow::anyhow!("TELEGRAM_RPC_ERROR") } else { rpc_error(e) }
        })
        })
    }
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
    })
}
fn preview(media: &Media, link: &MessageLink) -> Value {
    json!({"platform":"telegram","resolver":"grammers","kind":"video","video_id":format!("{}_{}",media.channel,media.message),
        "canonical_url":format!("https://t.me/c/{}/{}",media.channel,media.message),"title":media.name,"original_filename":media.name,"mime_type":media.mime,
        "author_profile":{"id":media.channel.to_string(),"nickname":null,"handle":public_peer(link),"kind":"channel","avatar_cached":false},
        "filesize":media.size,"duration_ms":null,"width":null,"height":null,"selected_format_id":"original","telegram_document_id":media.document,
        "formats":[{"format_id":"original","container":media.container,"codec":null,"width":null,"height":null,"filesize":media.size}]})
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
    core: &crate::desktop_core::Core,
    input: &str,
    expected: Option<&Value>,
    directory: Option<&Path>,
    progress: impl FnMut(Value),
    mut cancel: watch::Receiver<bool>,
) -> Result<Value> {
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
    let connection = if factory.is_some() {
        None
    } else {
        let ready = tokio::select! { biased; _=cancel.wait_for(|v|*v)=>bail!("CANCELED"), result=binding.session.resume(binding.namespace)=>result? };
        if !matches!(ready.phase, crate::backup::telegram_session::Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        Some(binding.session.download_connection()?)
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
        })
    };
    let (_stop, mut stop) = watch::channel(false);
    if let Some(connection) = &connection {
        stop = connection.cancellation();
    }
    let result = tokio::select! { biased;
        _=cancel.wait_for(|v|*v)=>Err(anyhow::anyhow!("CANCELED")),
        _=stop.wait_for(|v|*v)=>Err(anyhow::anyhow!("TELEGRAM_AUTH_REQUIRED")),
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
async fn transfer(
    source: &mut dyn Transport,
    link: &MessageLink,
    expected: Option<&Value>,
    directory: Option<&Path>,
    mut progress: impl FnMut(Value),
) -> Result<Value> {
    let mut media = timeout(IDLE_TIMEOUT, source.inspect(link))
        .await
        .context("TELEGRAM_TIMEOUT")??;
    let Some(expected) = expected else {
        return Ok(preview(&media, link));
    };
    check_expected(&media, expected)?;
    let directory = directory.context("IO_ERROR")?;
    let partial = directory.join(format!("video.{}.part", media.container));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)
        .await
        .context("IO_ERROR")?;
    let mut refreshed = false;
    let mut received = 0;
    let mut head = Vec::new();
    let mut samples = VecDeque::from([(Instant::now(), 0u64)]);
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        let next = timeout(IDLE_TIMEOUT, source.next())
            .await
            .context("TELEGRAM_TIMEOUT")?;
        let bytes = match next {
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
                samples = VecDeque::from([(Instant::now(), 0)]);
                progress(
                    json!({"bytes":0,"total_bytes":media.size,"speed_bps":null,"eta_seconds":null}),
                );
                continue;
            }
            other => other?,
        };
        let Some(bytes) = bytes else {
            break;
        };
        if received + bytes.len() as u64 > media.size {
            bail!("INCOMPLETE_MEDIA");
        }
        file.write_all(&bytes).await.context("IO_ERROR")?;
        file.flush().await.context("IO_ERROR")?;
        received += bytes.len() as u64;
        if head.len() < 12 {
            head.extend_from_slice(&bytes[..bytes.len().min(12 - head.len())]);
        }
        let now = Instant::now();
        if last.elapsed() >= Duration::from_millis(250) || received == media.size {
            samples.push_back((now, received));
            while samples.len() > 2 && now.duration_since(samples[1].0) > Duration::from_secs(3) {
                samples.pop_front();
            }
            let (at, count) = samples.front().context("INVALID_STATE")?;
            let speed =
                (received - count) as f64 / now.duration_since(*at).as_secs_f64().max(0.001);
            progress(
                json!({"bytes":received,"total_bytes":media.size,"speed_bps":speed,"eta_seconds":if speed>0.0 {Some((media.size-received) as f64/speed)} else {None}}),
            );
            last = now;
        }
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
    Ok(
        json!({"platform":"telegram","video_id":format!("{}_{}",media.channel,media.message),"container":media.container,"filename":filename,"bytes":received,"format_id":"original"}),
    )
}
#[cfg(test)]
#[path = "telegram_download_tests.rs"]
mod tests;
