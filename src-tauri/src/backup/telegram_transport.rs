//! grammers adapter for original document bytes. Temporary file references never escape this layer.
#[cfg(test)]
#[path = "telegram_rich_tests.rs"]
mod rich_tests;
#[cfg(test)]
#[path = "telegram_index_tests.rs"]
mod index_tests;
#[path = "telegram_message_map.rs"]
mod message_map;
#[path = "telegram_bundle.rs"]
mod bundle;
use super::{
    provider::{ProviderFuture, VideoAttributes},
    telegram_session::rpc_error,
    telegram_transfer::{Envelope, Io, RemoteDocument, Target, Transport},
};
use anyhow::{bail, Context, Result};
use grammers_client::{
    media::{Media, Uploaded},
    peer::Peer,
    session::types::PeerRef,
    tl, Client, InvocationError,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

fn document_attributes(name: String, video: Option<&VideoAttributes>) -> (String, Vec<tl::enums::DocumentAttribute>) {
    let mime = mime_guess::from_path(&name).first_or_octet_stream().to_string();
    let mut attributes = vec![tl::types::DocumentAttributeFilename { file_name: name }.into()];
    if let Some(video) = video {
        attributes.push(tl::types::DocumentAttributeVideo {
            round_message: false, supports_streaming: video.supports_streaming, nosound: false,
            duration: video.duration_ms as f64 / 1000.0, w: video.width as i32, h: video.height as i32,
            preload_prefix_size: None, video_start_ts: None, video_codec: None,
        }.into());
        (mime, attributes)
    } else { ("application/octet-stream".into(), attributes) }
}
fn uploaded_media(uploaded: Uploaded, video: Option<&VideoAttributes>, thumb: Option<tl::enums::InputFile>) -> Result<tl::enums::InputMedia> {
    let name = match &uploaded.raw {
        tl::enums::InputFile::File(f) => f.name.clone(),
        tl::enums::InputFile::Big(f) => f.name.clone(),
        _ => bail!("BACKUP_INVALID_MEDIA"),
    };
    let (mime_type, attributes) = document_attributes(name, video);
    Ok(tl::types::InputMediaUploadedDocument {
        nosound_video:video.is_some(),force_file:video.is_none(),spoiler:false,file:uploaded.raw,thumb,
        mime_type,attributes,stickers:None,video_cover:None,video_timestamp:None,ttl_seconds:None,
    }.into())
}
struct OriginalDocument<'a>(&'a tl::types::Document);
pub(super) struct PreparedUpload { file: Uploaded, thumbnail: Option<tl::enums::InputFile> }
impl grammers_client::media::Downloadable for OriginalDocument<'_> {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(tl::types::InputDocumentFileLocation {id:self.0.id,access_hash:self.0.access_hash,file_reference:self.0.file_reference.clone(),thumb_size:String::new()}.into())
    }
    fn size(&self) -> Option<usize> {usize::try_from(self.0.size).ok()}
    fn dc_id(&self) -> Option<i32> {Some(self.0.dc_id)}
}
fn message_documents(message:&grammers_client::message::Message) -> Result<Vec<(RemoteDocument,tl::types::Document)>> {
    if let Some(documents)=bundle::documents(message)? {return Ok(documents);}
    let (evidence,Media::Document(document))=document(message)? else {bail!("TELEGRAM_MEDIA_CHANGED");};
    let envelope:Envelope=serde_json::from_str(&evidence.caption).context("TELEGRAM_INDEX_INVALID")?;
    if envelope.kind!="library" {bail!("TELEGRAM_INDEX_INVALID");}
    let Some(tl::enums::Document::Document(raw))=document.raw.document else {bail!("TELEGRAM_MEDIA_MISSING");};
    Ok(vec![(evidence,raw)])
}
fn select_document(message:&grammers_client::message::Message, document_id:Option<i64>, envelope:Option<&Envelope>) -> Result<(RemoteDocument,tl::types::Document)> {
    let mut documents=message_documents(message)?;
    documents.retain(|(d,_)|document_id.is_none_or(|id|id==d.document_id) && envelope.is_none_or(|e|e.caption().ok().as_deref()==Some(d.caption.as_str())));
    if documents.len()!=1 && !(document_id.is_some() && envelope.is_none() && !documents.is_empty()) {bail!("TELEGRAM_MEDIA_MISSING");}
    documents.pop().context("TELEGRAM_MEDIA_MISSING")
}

// Shared by clones in one request; each fresh connection gets an empty cache.
type IndexBodyCache = std::sync::Mutex<std::collections::HashMap<RemoteDocument, Vec<u8>>>;
// Verification evidence lasts one execution attempt, including neither a retry nor maintenance.
struct ArchiveRead {
    target: Target,
    document: RemoteDocument,
    bytes: Vec<u8>,
}
tokio::task_local! {
    static DELETE_ARCHIVE_READ: std::cell::RefCell<Option<ArchiveRead>>;
    static PUBLISH_ARCHIVE: (Target, String);
}

pub(super) async fn with_delete_archive_read<F: Future>(first_publish: bool, work: F) -> F::Output {
    if first_publish {
        DELETE_ARCHIVE_READ.scope(Default::default(), work).await
    } else {
        work.await
    }
}

pub(super) async fn capture_publish_refresh<F: Future>(target: &Target, operation: &str, work: F) -> F::Output {
    PUBLISH_ARCHIVE.scope((target.clone(), operation.to_owned()), work).await
}

fn remember_archive_read(envelope: &Envelope, document: &RemoteDocument, bytes: &[u8]) {
    let _ = PUBLISH_ARCHIVE.try_with(|(target, operation)| {
        if envelope.kind == "library" && envelope.operation_id == *operation && envelope.library_id == target.scope.library_id {
            let _ = DELETE_ARCHIVE_READ.try_with(|slot| slot.replace(Some(ArchiveRead {
                target: target.clone(), document: document.clone(), bytes: bytes.to_vec(),
            })));
        }
    });
}

pub(super) async fn confirm_archive(transport: &impl Transport, target: &Target, document: &RemoteDocument, expected: &[u8], io: &Io) -> Result<()> {
    io.check()?;
    let published = DELETE_ARCHIVE_READ.try_with(|slot| slot.take()).ok().flatten();
    if let Some(read) = published {
        if read.target != *target || read.document != *document || read.bytes != expected {
            bail!("BACKUP_PUBLICATION_UNCONFIRMED");
        }
        let _check = super::delete_timing::setup_span("archive_identity_check");
        let current = io.run(transport.inspect_document(document.message_id, document.document_id)).await?;
        if current != *document { bail!("TELEGRAM_MEDIA_CHANGED"); }
    } else {
        let _download = super::delete_timing::setup_span("archive_body_download");
        let bytes = read_index_body(transport, &IndexBodyCache::default(), document.clone(), io, None).await?;
        if bytes != expected { bail!("BACKUP_PUBLICATION_UNCONFIRMED"); }
    }
    Ok(())
}

async fn read_index_body(
    transport: &impl Transport,
    cache: &IndexBodyCache,
    expected: RemoteDocument,
    io: &Io,
    mut scanned: Option<Box<dyn AsyncRead + Unpin + Send>>,
) -> Result<Vec<u8>> {
    io.check()?;
    if let Some(bytes) = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("TELEGRAM_INDEX_INVALID"))?
        .get(&expected)
        .cloned()
    {
        return Ok(bytes);
    }
    use sha2::{Digest, Sha256};
    let envelope: Envelope =
        serde_json::from_str(&expected.caption).context("TELEGRAM_INDEX_INVALID")?;
    for attempt in 0..2 {
        let result = async {
            let _body = super::delete_timing::refresh_span("index_download");
            let mut reader = match scanned.take() {
                Some(reader) => reader,
                None => io.run(transport.download(expected.clone())).await?,
            };
            let mut bytes = vec![];
            let mut buffer = vec![0; 65536];
            loop {
                let count = io
                    .run(async { reader.read(&mut buffer).await.map_err(anyhow::Error::from) })
                    .await?;
                if count == 0 {
                    break;
                }
                if bytes.len() as u64 + count as u64 > expected.bytes {
                    bail!("TELEGRAM_INDEX_INVALID");
                }
                bytes.extend_from_slice(&buffer[..count]);
                io.touch();
            }
            if bytes.len() as u64 != expected.bytes
                || format!("{:x}", Sha256::digest(&bytes)) != envelope.sha256
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            remember_archive_read(&envelope, &expected, &bytes);
            cache
                .lock()
                .map_err(|_| anyhow::anyhow!("TELEGRAM_INDEX_INVALID"))?
                .insert(expected.clone(), bytes.clone());
            Ok(bytes)
        }
        .await;
        match result {
            Err(e) if attempt == 0 && e.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" => {
                continue
            }
            other => return other,
        }
    }
    unreachable!("bounded index download always returns")
}

#[derive(Clone)]
pub(super) struct GrammersTransport {
    index_bodies: std::sync::Arc<IndexBodyCache>,
    client: Client,
    peer: PeerRef,
    max_bytes: u64,
    can_post: bool,
    can_delete: bool,
    channel_title: String,
    preferred_message:Option<i32>,
    new_delete_operation: Option<String>,
    bundle_heading: Option<String>,
    video_thumbnail: Option<Vec<u8>>,
}
#[derive(Clone)]
pub(super) struct IndexDocument {
    pub revision: i64,
    pub document: RemoteDocument,
    source: IndexSource,
}
impl IndexDocument {
    pub(super) fn verified_body(&self) -> Result<&[u8]> {
        match &self.source {
            IndexSource::Verified(bytes) => Ok(bytes),
            IndexSource::Scanned(_) => bail!("TELEGRAM_INDEX_INVALID"),
        }
    }
}
#[derive(Clone)]
enum IndexSource {
    Scanned(std::sync::Arc<tl::types::Document>),
    Verified(std::sync::Arc<Vec<u8>>),
}
#[derive(Default)]
pub(super) struct ChannelSnapshot {pub high:i64,pub indexes:Vec<IndexDocument>,pub media:Vec<RemoteDocument>}
impl ChannelSnapshot {
    fn order(&mut self) {
        self.indexes.sort_by_key(|index| index.revision);
        self.media.sort_by_key(|document| document.message_id);
        self.high = self.high.max(self.indexes.last().map_or(0, |index| index.revision));
    }
}

// The cached reference supplies only the address. Membership and permissions come from this reply.
async fn cached_channel(
    client: &Client, session: &super::telegram_vault::VaultSession, id: i64, io: &Io,
) -> Result<Option<grammers_client::peer::Channel>> {
    use grammers_session::{Session, types::{PeerId, PeerInfo}};
    let id = PeerId::channel(id).context("TELEGRAM_CHANNEL_FORBIDDEN")?;
    let Some(peer) = session.peer_ref(id).await? else { return Ok(None) };
    let _by_id = super::delete_timing::setup_span("channel_by_id");
    let reply = io.run(async {
        match client.invoke(&tl::functions::channels::GetChannels { id: vec![peer.into()] }).await {
            Ok(reply) => Ok(Some(reply)),
            Err(InvocationError::Rpc(e)) if matches!(e.name.as_str(), "CHANNEL_INVALID" | "PEER_ID_INVALID" | "ACCESS_HASH_INVALID") => Ok(None),
            Err(e) => Err(rpc_error(e)),
        }
    }).await?;
    let Some(reply) = reply else { return Ok(None) };
    let chats = match reply {
        tl::enums::messages::Chats::Chats(result) => result.chats,
        tl::enums::messages::Chats::Slice(result) => result.chats,
    };
    let Some(raw) = checked_channel_reply(id.bare_id_unchecked(), chats)? else { return Ok(None) };
    if raw.access_hash.is_some() {
        session.cache_peer(PeerInfo::from(&raw)).await?;
    }
    Ok(Some(grammers_client::peer::Channel::from_raw(client, raw.into())))
}

fn checked_channel_reply(id: i64, mut chats: Vec<tl::enums::Chat>) -> Result<Option<tl::types::Channel>> {
    if chats.is_empty() { return Ok(None); }
    if chats.len() != 1 { bail!("TELEGRAM_CHANNEL_FORBIDDEN"); }
    match chats.pop() {
        Some(tl::enums::Chat::Channel(channel)) if channel.id == id && channel.broadcast => {
            Ok((!channel.min).then_some(channel))
        }
        Some(tl::enums::Chat::Empty(empty)) if empty.id == id => Ok(None),
        _ => bail!("TELEGRAM_CHANNEL_FORBIDDEN"),
    }
}

async fn check_read_access(kind: &str, probe: impl Future<Output = Result<()>>) -> Result<()> {
    // Both deletion stages read the actual mapped index before any remote mutation.
    if matches!(kind, "publish_delete" | "remote_cleanup") {
        return Ok(());
    }
    let _access = super::delete_timing::setup_span("channel_read_probe");
    probe.await
}

impl GrammersTransport {
    pub(super) fn for_new_delete(&self, operation: &str) -> Self {
        let mut value = self.clone();
        value.new_delete_operation = Some(operation.to_owned());
        value
    }

    pub(super) fn with_thumbnail(&self, body: Vec<u8>) -> Self {
        let mut value = self.clone();
        value.video_thumbnail = Some(body);
        value
    }
    async fn upload_video_thumbnail(&self) -> Result<Option<tl::enums::InputFile>> {
        let Some(body) = self.video_thumbnail.as_ref() else {
            return Ok(None);
        };
        let mut reader = std::io::Cursor::new(body.as_slice());
        let uploaded = self
            .client
            .upload_stream(&mut reader, body.len(), "video-thumb.jpg".into())
            .await
            .map_err(io_error)?;
        Ok(Some(uploaded.raw))
    }

    pub(super) fn with_heading(&self, title: Option<&str>, filename: Option<&str>) -> Self {
        let mut value = self.clone();
        value.bundle_heading = bundle::display_heading(title, filename);
        value
    }
    pub(super) fn for_message(&self,message:Option<i32>)->Self {let mut value=self.clone();value.preferred_message=message;value}
    pub(super) fn channel_title(&self) -> &str { &self.channel_title }
    #[cfg(test)]
    pub(super) async fn streaming_video_flag(&self, id: i32) -> Result<bool> {
        let message = self.message(id).await?;
        let (_,raw)=message_documents(&message)?.into_iter().find(|(_,d)|d.attributes.iter().any(|a|matches!(a,tl::enums::DocumentAttribute::Video(_)))).context("TELEGRAM_MEDIA_MISSING")?;
        Ok(raw.mime_type == "video/mp4" && raw.attributes.iter().any(|a| matches!(a, tl::enums::DocumentAttribute::Video(v) if v.supports_streaming)))
    }
    #[cfg(test)]
    pub(super) async fn bundle_documents(&self,id:i32) -> Result<Vec<RemoteDocument>> {
        Ok(bundle::documents(&self.message(id).await?)?.context("TELEGRAM_INDEX_INVALID")?.into_iter().map(|(d,_)|d).collect())
    }
    #[cfg(test)]
    pub(super) async fn bundle_presentation(&self,id:i32)->Result<serde_json::Value> {
        let message=self.fetch_rich_message(self.message(id).await?).await?;
        let tl::enums::Message::Message(raw)=message.raw else {bail!("TELEGRAM_MEDIA_MISSING");};
        let Some(tl::enums::RichMessage::Message(rich))=raw.rich_message else {bail!("TELEGRAM_INDEX_INVALID");};
        let visible=rich.blocks.iter().filter_map(|b|match b {tl::enums::PageBlock::Video(v)=>Some(v.video_id),tl::enums::PageBlock::Document(d)=>Some(d.document_id),_=>None}).collect::<Vec<_>>();
        Ok(serde_json::json!({"visible_documents":visible,"documents":rich.documents.len(),"anchors":rich.blocks.iter().filter(|b|matches!(b,tl::enums::PageBlock::Anchor(_))).count(),"history_collapsed":rich.blocks.iter().any(|b|matches!(b,tl::enums::PageBlock::Details(d) if !d.open)),"legacy_media":raw.media.is_some()}))
    }
    #[cfg(test)]
    pub(super) async fn recent_rich_shapes(&self,io:&Io) -> Result<serde_json::Value> {
        let mut history=self.client.iter_messages(self.peer).limit(5);let mut rows=vec![];
        while let Some(message)=io.run(async {history.next().await.map_err(rpc_error)}).await? {
            let tl::enums::Message::Message(raw)=&message.raw else {continue;};
            let (count,part,blocks)=match &raw.rich_message {Some(tl::enums::RichMessage::Message(r))=>(r.documents.len(),r.part,r.blocks.len()),_ =>(0,false,0)};
            rows.push(serde_json::json!({"text_is_json":serde_json::from_str::<serde_json::Value>(message.text()).is_ok(),"text_chars":message.text().chars().count(),"rich_documents":count,"rich_part":part,"rich_blocks":blocks,"legacy_media":raw.media.is_some()}));
        }
        Ok(serde_json::json!(rows))
    }
    /// Read one fixed high-water prefix; gaps in unrelated message IDs are legal.
    pub(super) async fn indexes(&self, io: &Io) -> Result<(i64, Vec<IndexDocument>)> {
        let snapshot = self.scan(io).await?;
        Ok((snapshot.high, snapshot.indexes))
    }
    pub(super) async fn scan(&self, io: &Io) -> Result<ChannelSnapshot> {
        // History pages descend from the first response and continue below its last ID.
        let mut history = self.client.iter_messages(self.peer);
        let mut high = None;
        let mut snapshot = ChannelSnapshot::default();
        while let Some(message) = io
            .run(async { history.next().await.map_err(rpc_error) })
            .await?
        {
            super::delete_timing::record_history_message();
            io.touch();
            if message.id() > *high.get_or_insert(message.id()) {
                continue;
            }
            self.collect_message(message, io, &mut snapshot).await?;
        }
        snapshot.high = i64::from(high.unwrap_or(0));
        snapshot.order();
        Ok(snapshot)
    }

    async fn collect_message(
        &self,
        message: grammers_client::message::Message,
        io: &Io,
        snapshot: &mut ChannelSnapshot,
    ) -> Result<()> {
        let message = io.run(self.complete_message(message)).await?;
        if let Some(documents) = bundle::documents(&message)? {
            for (evidence, original) in documents {
                let envelope: Envelope = serde_json::from_str(&evidence.caption)
                    .context("TELEGRAM_INDEX_INVALID")?;
                if matches!(envelope.kind.as_str(), "event" | "library") {
                    snapshot.indexes.push(IndexDocument {
                        revision: if envelope.kind == "event" {
                            bundle::event_revision(&message, &envelope)?
                        } else {
                            i64::from(message.id())
                        },
                        document: evidence,
                        source: IndexSource::Scanned(original.into()),
                    });
                } else {
                    snapshot.media.push(evidence);
                }
            }
            return Ok(());
        }
        let Ok(raw) = serde_json::from_str::<serde_json::Value>(message.text()) else {
            return Ok(());
        };
        if raw["yingqu_media"] != 2 || raw["kind"] != "library" {
            return Ok(());
        }
        let envelope: Envelope =
            serde_json::from_value(raw).context("TELEGRAM_INDEX_INVALID")?;
        let (evidence, Media::Document(attachment)) = document(&message)? else {
            bail!("TELEGRAM_MEDIA_CHANGED");
        };
        let Some(tl::enums::Document::Document(original)) = attachment.raw.document else {
            bail!("TELEGRAM_MEDIA_MISSING");
        };
        if evidence.bytes != envelope.bytes
            || (envelope.container != "json" || evidence.bytes > 16 * 1024 * 1024)
        {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        snapshot.indexes.push(IndexDocument {
            revision: i64::from(evidence.message_id),
            document: evidence,
            source: IndexSource::Scanned(original.into()),
        });
        Ok(())
    }

    pub(super) async fn index_body(&self, index: &IndexDocument, io: &Io) -> Result<Vec<u8>> {
        io.check()?;
        let reader = match &index.source {
            IndexSource::Verified(body) => return Ok(body.as_ref().clone()),
            IndexSource::Scanned(document) => self.original_reader(document),
        };
        read_index_body(self, &self.index_bodies, index.document.clone(), io, Some(reader)).await
    }

    fn original_reader(&self, document: &tl::types::Document) -> Box<dyn AsyncRead + Unpin + Send> {
        Box::new(DownloadReader {
            iter: Some(self.client.iter_download(&OriginalDocument(document))),
            next: None,
            buffer: vec![],
            offset: 0,
        })
    }
    pub(super) async fn media_reader(
        &self,
        target: &Target,
        reference: &super::provider::MediaRef,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>> {
        let doc = self.inspect_reference(target,reference).await?;
        let envelope: Envelope =
            serde_json::from_str(&doc.caption).context("TELEGRAM_MEDIA_CHANGED")?;
        doc.check(&envelope)?;
        if matches!(&reference.location,super::provider::MediaLocation::TelegramMessage {document_id:id,..} if *id!=doc.document_id)
        {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        if envelope.yingqu_media != 2
            || envelope.library_id != target.scope.library_id
            || envelope.bytes != reference.bytes
            || envelope.sha256 != reference.sha256
            || envelope.container != reference.container
        {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        self.download(doc).await
    }
    pub(super) async fn connect(
        client: Client,
        target: &Target,
        session: Option<&super::telegram_vault::VaultSession>,
        request_kind: &str,
        upload_limit: impl Future<Output = Result<u64>>,
        io: &Io,
    ) -> Result<Self> {
        target.validate()?;
        let lookup = super::delete_timing::setup_span("channel_lookup");
        let id = target.channel_id.parse::<i64>()?;
        let cached = if let Some(session) = session {
            cached_channel(&client, session, id, io).await?
        } else {
            None
        };
        let channel = match cached {
            Some(channel) => channel,
            None => {
                let _scan = super::delete_timing::setup_span("channel_dialogs");
                let mut dialogs = client.iter_dialogs();
                loop {
                    let dialog = io.run(async { dialogs.next().await.map_err(rpc_error) }).await?
                        .context("TELEGRAM_CHANNEL_FORBIDDEN")?;
                    if let Peer::Channel(channel) = dialog.peer() {
                        if channel.raw.id == id { break channel.clone(); }
                    }
                }
            }
        };
        let has_public_alias = channel.raw.usernames.as_ref().is_some_and(|names| {
            names.iter().any(|name| {
                let tl::enums::Username::Username(name) = name;
                name.active
            })
        });
        if channel.raw.left || channel.username().is_some() || has_public_alias {
            bail!("TELEGRAM_CHANNEL_FORBIDDEN");
        }
        let rights = channel.raw.admin_rights.as_ref().map(|r| {
            let tl::enums::ChatAdminRights::Rights(rights) = r;
            rights
        });
        let can_post = channel.raw.creator || rights.is_some_and(|r| r.post_messages);
        let can_delete = channel.raw.creator || rights.is_some_and(|r| r.delete_messages);
        let peer = channel.to_ref().await.map_err(|_| anyhow::anyhow!("TELEGRAM_CHANNEL_FORBIDDEN"))?
            .context("TELEGRAM_CHANNEL_FORBIDDEN")?;
        let channel_title = channel.title().to_owned();
        drop(lookup);
        check_read_access(request_kind, async {
            let mut history = client.iter_messages(peer).limit(1);
            io.run(async { history.next().await.map_err(rpc_error) }).await?;
            Ok(())
        }).await?;
        let max_bytes = upload_limit.await?;
        Ok(Self {
            index_bodies: std::sync::Arc::default(),
            client,
            peer,
            max_bytes,
            can_post,
            can_delete,
            channel_title,
            preferred_message:None,
            new_delete_operation: None,
            bundle_heading: None,
            video_thumbnail: None,
        })
    }
    async fn message(&self, id: i32) -> Result<grammers_client::message::Message> {
        let message = self
            .client
            .get_messages_by_id(self.peer, &[id])
            .await
            .map_err(rpc_error)?
            .into_iter()
            .next()
            .flatten()
            .context("TELEGRAM_MEDIA_MISSING")?;
        check_presence(&message.raw)?;
        self.complete_message(message).await
    }
    async fn complete_message(&self,message:grammers_client::message::Message)->Result<grammers_client::message::Message> {
        let tl::enums::Message::Message(original)=&message.raw else {return Ok(message);};
        if !matches!(&original.rich_message,Some(tl::enums::RichMessage::Message(r)) if r.part) {return Ok(message);}
        self.fetch_rich_message(message).await
    }
    async fn fetch_rich_message(&self,mut message:grammers_client::message::Message)->Result<grammers_client::message::Message> {
        let tl::enums::Message::Message(original)=&message.raw else {bail!("TELEGRAM_MEDIA_MISSING");};
        let id=original.id;let peer=original.peer_id.clone();
        let response=self.client.invoke(&tl::functions::messages::GetRichMessage {peer:self.peer.into(),id}).await.map_err(rpc_error)?;
        let messages=match response {tl::enums::messages::Messages::Messages(v)=>v.messages,tl::enums::messages::Messages::Slice(v)=>v.messages,tl::enums::messages::Messages::ChannelMessages(v)=>v.messages,_=>bail!("TELEGRAM_INDEX_INVALID")};
        let raw=messages.into_iter().find(|m|matches!(m,tl::enums::Message::Message(m) if m.id==id&&m.peer_id==peer)).context("TELEGRAM_MEDIA_MISSING")?;
        if !matches!(&raw,tl::enums::Message::Message(m) if matches!(&m.rich_message,Some(tl::enums::RichMessage::Message(r)) if !r.part)) {bail!("TELEGRAM_INDEX_INVALID");}
        message.raw=raw;Ok(message)
    }
}
fn check_presence(message: &tl::enums::Message) -> Result<()> {
    // grammers 0.11 can return Some(MessageEmpty) for a deleted channel message.
    if matches!(message, tl::enums::Message::Empty(_)) {
        bail!("TELEGRAM_MEDIA_MISSING");
    }
    Ok(())
}
pub(super) async fn fetch_upload_limit(client: &Client, premium: bool, io: &Io) -> Result<u64> {
    let config = io.run(async {
        client.invoke(&tl::functions::help::GetAppConfig { hash: 0 }).await.map_err(rpc_error)
    }).await?;
    let tl::enums::help::AppConfig::Config(config) = config else {
        bail!("TELEGRAM_LIMIT_UNAVAILABLE");
    };
    limit(&config.config, premium)
}

fn limit(config: &tl::enums::Jsonvalue, premium: bool) -> Result<u64> {
    let tl::enums::Jsonvalue::JsonObject(object) = config else {
        bail!("TELEGRAM_LIMIT_UNAVAILABLE");
    };
    let key = if premium {
        "upload_max_fileparts_premium"
    } else {
        "upload_max_fileparts_default"
    };
    let parts = object
        .value
        .iter()
        .find_map(|pair| {
            let tl::enums::JsonobjectValue::JsonObjectValue(pair) = pair;
            if pair.key != key {
                return None;
            }
            let tl::enums::Jsonvalue::JsonNumber(number) = &pair.value else {
                return None;
            };
            Some(number.value)
        })
        .context("TELEGRAM_LIMIT_UNAVAILABLE")?;
    if !parts.is_finite() || parts <= 0.0 || parts.fract() != 0.0 || parts > i32::MAX as f64 {
        bail!("TELEGRAM_LIMIT_UNAVAILABLE");
    }
    (parts as u64)
        .checked_mul(512 * 1024)
        .context("TELEGRAM_LIMIT_UNAVAILABLE")
}
fn document(message: &grammers_client::message::Message) -> Result<(RemoteDocument, Media)> {
    let media = message.media().context("TELEGRAM_MEDIA_MISSING")?;
    let Media::Document(document) = &media else {
        bail!("TELEGRAM_MEDIA_CHANGED");
    };
    let tl::enums::Document::Document(raw) = document
        .raw
        .document
        .as_ref()
        .context("TELEGRAM_MEDIA_MISSING")?
    else {
        bail!("TELEGRAM_MEDIA_MISSING");
    };
    Ok((
        RemoteDocument {
            message_id: message.id(),
            document_id: raw.id,
            bytes: u64::try_from(raw.size).context("TELEGRAM_MEDIA_CHANGED")?,
            caption: message.text().into(),
        },
        media,
    ))
}
impl Transport for GrammersTransport {
    type Uploaded = PreparedUpload;
    fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    fn can_upload(&self) -> bool {
        self.can_post
    }
    fn can_delete(&self) -> bool {
        self.can_delete
    }
    #[cfg(test)]
    fn media<'a>(&'a self, io: &'a Io) -> ProviderFuture<'a, Vec<RemoteDocument>> {
        Box::pin(async move { Ok(self.scan(io).await?.media) })
    }
    fn remove(&self, expected: RemoteDocument) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            if !self.can_delete {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            let read = super::delete_timing::setup_span("read_old_index");
            let message = match self.message(expected.message_id).await {
                Ok(message) => message,
                Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
                Err(e) => return Err(e),
            };
            match self.remove_bundle_attachment(&expected, &message).await {
                Ok(true) => return Ok(()),
                Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
                Err(e) => return Err(e),
                Ok(false) => {}
            }
            let envelope:Envelope=serde_json::from_str(&expected.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.kind!="library" {bail!("TELEGRAM_INDEX_INVALID");}
            let actual = document(&message)?.0;
            if actual != expected {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            drop(read);
            let send = super::delete_timing::setup_span("delete_old_index");
            self.client
                .delete_messages(self.peer, &[expected.message_id])
                .await
                .map_err(rpc_error)?;
            drop(send);
            let _confirm = super::delete_timing::setup_span("confirm_old_index_gone");
            match self.message(expected.message_id).await {
                Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => Ok(()),
                Err(e) => Err(e),
                Ok(_) => bail!("BACKUP_CLEANUP_PENDING"),
            }
        })
    }
    fn upload<'a>(
        &'a self,
        mut reader: &'a mut (dyn AsyncRead + Unpin + Send),
        bytes: u64,
        name: String,
    ) -> ProviderFuture<'a, PreparedUpload> {
        Box::pin(async move {
            let file = self.client
                .upload_stream(
                    &mut reader,
                    usize::try_from(bytes).context("TELEGRAM_FILE_TOO_LARGE")?,
                    name,
                )
                .await
                .map_err(io_error)?;
            let thumbnail = self.upload_video_thumbnail().await?;
            Ok(PreparedUpload { file, thumbnail })
        })
    }
    fn send<'a>(
        &'a self,
        uploaded: PreparedUpload,
        caption: String,
        random_id: i64,
        video: Option<VideoAttributes>,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<i32>> {
        Box::pin(async move {
            let envelope:Envelope=serde_json::from_str(&caption).context("BACKUP_INVALID_MEDIA")?;
            if envelope.kind!="library" {return self.send_bundle(uploaded,envelope,random_id,video,io).await;}
            let uploaded = uploaded.file;
            let name = match &uploaded.raw {
                tl::enums::InputFile::File(f) => f.name.clone(),
                tl::enums::InputFile::Big(f) => f.name.clone(),
                _ => bail!("BACKUP_INVALID_MEDIA"),
            };
            let (mime_type, attributes) = document_attributes(name, video.as_ref());
            let media = tl::types::InputMediaUploadedDocument {
                nosound_video: video.is_some(),
                force_file: video.is_none(),
                spoiler: false,
                file: uploaded.raw,
                thumb: None,
                mime_type,
                attributes,
                stickers: None,
                video_cover: None,
                video_timestamp: None,
                ttl_seconds: None,
            };
            let updates = self
                .client
                .invoke(&tl::functions::messages::SendMedia {
                    silent: true,
                    background: false,
                    clear_draft: false,
                    noforwards: false,
                    update_stickersets_order: false,
                    invert_media: false,
                    allow_paid_floodskip: false,
                    peer: self.peer.into(),
                    reply_to: None,
                    media: media.into(),
                    message: caption,
                    random_id,
                    reply_markup: None,
                    entities: None,
                    schedule_date: None,
                    schedule_repeat_period: None,
                    send_as: None,
                    quick_reply_shortcut: None,
                    effect: None,
                    allow_paid_stars: None,
                    suggested_post: None,
                })
                .await
                .map_err(rpc_error)?;
            Ok(sent_id(updates, random_id))
        })
    }
    fn inspect(&self, id: i32) -> ProviderFuture<'_, RemoteDocument> {
        Box::pin(async move { Ok(select_document(&self.message(id).await?,None,None)?.0) })
    }
    fn inspect_document(&self, id:i32, document_id:i64) -> ProviderFuture<'_,RemoteDocument> {
        Box::pin(async move {Ok(select_document(&self.message(id).await?,Some(document_id),None)?.0)})
    }
    fn inspect_envelope<'a>(&'a self,id:i32,envelope:&'a Envelope) -> ProviderFuture<'a,RemoteDocument> {
        Box::pin(async move {Ok(select_document(&self.message(id).await?,None,Some(envelope))?.0)})
    }
    fn find<'a>(
        &'a self,
        envelope: &'a Envelope,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<RemoteDocument>> {
        Box::pin(async move {
            let caption = envelope.caption()?;
            let _history = super::delete_timing::setup_span("history");
            super::delete_timing::selected_full_scan("receipt_reconciliation", None);
            let mut history = self.client.iter_messages(self.peer);
            let mut found = None;
            while let Some(message) = history.next().await.map_err(rpc_error)? {
                super::delete_timing::record_history_message();
                io.check()?;
                io.touch();
                let message=io.run(self.complete_message(message)).await?;
                let evidence=if let Some(documents)=bundle::documents(&message)? {
                    let Some((evidence,_))=documents.into_iter().find(|(d,_)|d.caption==caption) else {continue;};
                    evidence
                } else {
                    if envelope.kind!="library" || message.text()!=caption {continue;}
                    document(&message)?.0
                };
                if found.is_some() {
                    bail!("TELEGRAM_DUPLICATE_OPERATION");
                }
                found = Some(evidence);
            }
            Ok(found)
        })
    }
    fn download(
        &self,
        expected: RemoteDocument,
    ) -> ProviderFuture<'_, Box<dyn AsyncRead + Unpin + Send>> {
        Box::pin(async move {
            // Always fetch the original message anew: file_reference is an expiring session detail.
            let (actual, document) = select_document(&self.message(expected.message_id).await?,Some(expected.document_id),None)?;
            if actual != expected {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            Ok(self.original_reader(&document))
        })
    }
}
fn sent_id(updates: tl::enums::Updates, random_id: i64) -> Option<i32> {
    let updates = match updates {
        tl::enums::Updates::Updates(value) => value.updates,
        tl::enums::Updates::Combined(value) => value.updates,
        tl::enums::Updates::UpdateShortSentMessage(value) => return Some(value.id),
        _ => return None,
    };
    updates.into_iter().find_map(|update| match update {
        tl::enums::Update::MessageId(value) if value.random_id == random_id => Some(value.id),
        _ => None,
    })
}
fn io_error(error: std::io::Error) -> anyhow::Error {
    let code = error.to_string();
    if let Some(inner) = error.into_inner() {
        if let Ok(error) = inner.downcast::<InvocationError>() {
            return rpc_error(*error);
        }
    }
    // The reader's own validation/cancellation error is already a fixed application code.
    if matches!(
        code.as_str(),
        "CANCELED"
            | "TELEGRAM_TRANSFER_CANCELED"
            | "BACKUP_SOURCE_CHANGED"
            | "BACKUP_SPACE_INACTIVE"
    ) {
        return anyhow::anyhow!(code);
    }
    anyhow::anyhow!("TELEGRAM_TRANSFER_IO")
}
type NextChunk = Pin<
    Box<
        dyn Future<
                Output = (
                    grammers_client::client::DownloadIter,
                    std::result::Result<Option<Vec<u8>>, InvocationError>,
                ),
            > + Send,
    >,
>;
struct DownloadReader {
    iter: Option<grammers_client::client::DownloadIter>,
    next: Option<NextChunk>,
    buffer: Vec<u8>,
    offset: usize,
}
impl AsyncRead for DownloadReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if self.offset < self.buffer.len() {
                let count = output.remaining().min(self.buffer.len() - self.offset);
                output.put_slice(&self.buffer[self.offset..self.offset + count]);
                self.offset += count;
                return Poll::Ready(Ok(()));
            }
            if self.next.is_none() {
                let Some(mut iter) = self.iter.take() else {
                    return Poll::Ready(Ok(()));
                };
                self.next = Some(Box::pin(async move {
                    let chunk = iter.next().await;
                    (iter, chunk)
                }));
            }
            let Poll::Ready((iter, result)) = self
                .next
                .as_mut()
                .expect("download future set above")
                .as_mut()
                .poll(cx)
            else {
                return Poll::Pending;
            };
            self.next = None;
            match result {
                Ok(Some(chunk)) => {
                    self.iter = Some(iter);
                    self.buffer = chunk;
                    self.offset = 0;
                }
                Ok(None) => return Poll::Ready(Ok(())),
                Err(error) => return Poll::Ready(Err(std::io::Error::other(rpc_error(error)))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_delete_publication_and_cleanup_skip_the_preflight_history_read() {
        for kind in ["publish_delete", "remote_cleanup"] {
            check_read_access(kind, async { panic!("redundant history probe for {kind}") }).await.unwrap();
        }
        for kind in ["publish_change", "open_library", "pull_changes", "upload_media", "delete_media", "unknown"] {
            let denied = check_read_access(kind, async { bail!("TELEGRAM_CHANNEL_FORBIDDEN") }).await.unwrap_err();
            assert_eq!(denied.to_string(), "TELEGRAM_CHANNEL_FORBIDDEN");
        }
    }
    #[test]
    fn uploaded_video_includes_custom_thumbnail_without_forcing_document_mode() {
        let file=tl::types::InputFile {id:1,parts:1,name:"original.mov".into(),md5_checksum:String::new()}.into();
        let thumbnail:tl::enums::InputFile=tl::types::InputFile {id:2,parts:1,name:"video-thumb.jpg".into(),md5_checksum:String::new()}.into();
        let video=VideoAttributes {width:1080,height:1920,duration_ms:2500,supports_streaming:true};
        let tl::enums::InputMedia::UploadedDocument(media)=uploaded_media(Uploaded::from_raw(file),Some(&video),Some(thumbnail.clone())).unwrap() else {panic!("expected uploaded video document")};
        assert_eq!(media.thumb,Some(thumbnail));assert!(!media.force_file);
        assert!(media.attributes.iter().any(|attr|matches!(attr,tl::enums::DocumentAttribute::Video(attr) if attr.supports_streaming&&attr.w==1080&&attr.h==1920)));
    }
    #[test]
    fn telegram_video_presentation_keeps_filename_and_original_document() {
        let metadata = serde_json::json!({"duration_ms":2500,"width":1920,"height":1080});
        let video = VideoAttributes::from_metadata(&metadata).unwrap();
        let (mime, attributes) = document_attributes("原视频.mp4".into(), Some(&video));
        assert_eq!(mime, "video/mp4");
        assert!(attributes.iter().any(|v| matches!(v, tl::enums::DocumentAttribute::Filename(v) if v.file_name == "原视频.mp4")));
        assert!(attributes.iter().any(|v| matches!(v, tl::enums::DocumentAttribute::Video(v) if v.duration == 2.5 && v.w == 1920 && v.h == 1080 && !v.supports_streaming)));
        let mut streaming = video.clone();
        streaming.supports_streaming = true;
        let (_, attributes) = document_attributes("原视频.mp4".into(), Some(&streaming));
        assert!(attributes.iter().any(|v| matches!(v, tl::enums::DocumentAttribute::Video(v) if v.supports_streaming)));
        let (mime, attributes) = document_attributes("cover.png".into(), None);
        assert_eq!(mime, "application/octet-stream");
        assert_eq!(attributes.len(), 1);
        assert!(VideoAttributes::from_metadata(&serde_json::json!({"duration_ms":1,"width":null,"height":1})).is_none());
    }
    #[test]
    fn video_presentation_uses_the_original_container_mime_and_disables_force_file() {
        let video = VideoAttributes::from_metadata(
            &serde_json::json!({"duration_ms":2500,"width":1280,"height":720}),
        )
        .unwrap();
        for (name, mime) in [
            ("clip.mp4", "video/mp4"),
            ("clip.mov", "video/quicktime"),
            ("clip.webm", "video/webm"),
            ("clip.mkv", "video/x-matroska"),
            ("clip.avi", "video/x-msvideo"),
        ] {
            let uploaded = Uploaded::from_raw(
                tl::types::InputFileBig {
                    id: 1,
                    parts: 1,
                    name: name.into(),
                }
                .into(),
            );
            let tl::enums::InputMedia::UploadedDocument(media) =
                uploaded_media(uploaded, Some(&video), None).unwrap()
            else {
                panic!("wrong media")
            };
            assert_eq!(media.mime_type, mime);
            assert!(!media.force_file);
            assert!(media.nosound_video);
            assert!(media.attributes.iter().any(|a|matches!(a,tl::enums::DocumentAttribute::Video(v) if v.w==1280 && v.h==720 && v.duration==2.5)));
            assert!(media
                .attributes
                .iter()
                .any(|a| matches!(a,tl::enums::DocumentAttribute::Filename(v) if v.file_name==name)));
        }
        let uploaded = Uploaded::from_raw(
            tl::types::InputFileBig {
                id: 1,
                parts: 1,
                name: "transcript.json".into(),
            }
            .into(),
        );
        let tl::enums::InputMedia::UploadedDocument(media) = uploaded_media(uploaded, None, None).unwrap()
        else {
            panic!("wrong media")
        };
        assert!(media.force_file);
        assert!(!media.nosound_video);
        assert!(!media
            .attributes
            .iter()
            .any(|a| matches!(a, tl::enums::DocumentAttribute::Video(_))));
    }
    #[test]
    fn telegram_t4_empty_channel_message_is_missing_after_delete() {
        let empty = tl::types::MessageEmpty {
            id: 42,
            peer_id: Some(tl::types::PeerChannel { channel_id: 123 }.into()),
        }
        .into();
        assert_eq!(
            check_presence(&empty).unwrap_err().to_string(),
            "TELEGRAM_MEDIA_MISSING"
        );
    }
    #[test]
    fn telegram_transport_uses_account_limit_without_guessing_missing_values() {
        let config = |parts: f64| {
            tl::enums::Jsonvalue::JsonObject(tl::types::JsonObject {
                value: vec![tl::types::JsonObjectValue {
                    key: "upload_max_fileparts_default".into(),
                    value: tl::types::JsonNumber { value: parts }.into(),
                }
                .into()],
            })
        };
        assert_eq!(limit(&config(4000.0), false).unwrap(), 4000 * 512 * 1024);
        assert!(limit(&config(4000.0), true).is_err());
        for value in [0.0, -1.0, 1.5, f64::INFINITY, f64::NAN] {
            assert!(limit(&config(value), false).is_err());
        }
    }
    #[test]
    fn telegram_transport_sent_message_id_matches_the_durable_random_id() {
        let updates = || {
            tl::types::Updates {
                updates: vec![tl::types::UpdateMessageId {
                    id: 42,
                    random_id: 123,
                }
                .into()],
                users: vec![],
                chats: vec![],
                date: 0,
                seq: 0,
            }
            .into()
        };
        assert_eq!(sent_id(updates(), 123), Some(42));
        assert_eq!(sent_id(updates(), 999), None);
    }
}
