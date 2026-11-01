//! grammers adapter for original document bytes. Temporary file references never escape this layer.
#[cfg(test)]
#[path = "telegram_rich_tests.rs"]
mod rich_tests;
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
    let mut attributes = vec![tl::types::DocumentAttributeFilename { file_name: name }.into()];
    if let Some(video) = video {
        attributes.push(tl::types::DocumentAttributeVideo {
            round_message: false, supports_streaming: video.supports_streaming, nosound: false,
            duration: video.duration_ms as f64 / 1000.0, w: video.width as i32, h: video.height as i32,
            preload_prefix_size: None, video_start_ts: None, video_codec: None,
        }.into());
        ("video/mp4".into(), attributes)
    } else { ("application/octet-stream".into(), attributes) }
}
fn uploaded_media(uploaded: Uploaded, video: Option<&VideoAttributes>) -> Result<tl::enums::InputMedia> {
    let name = match &uploaded.raw {
        tl::enums::InputFile::File(f) => f.name.clone(),
        tl::enums::InputFile::Big(f) => f.name.clone(),
        _ => bail!("BACKUP_INVALID_MEDIA"),
    };
    let (mime_type, attributes) = document_attributes(name, video);
    Ok(tl::types::InputMediaUploadedDocument {
        nosound_video:false,force_file:video.is_none(),spoiler:false,file:uploaded.raw,thumb:None,
        mime_type,attributes,stickers:None,video_cover:None,video_timestamp:None,ttl_seconds:None,
    }.into())
}
struct OriginalDocument(tl::types::Document);
impl grammers_client::media::Downloadable for OriginalDocument {
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

#[derive(Clone)]
pub(super) struct GrammersTransport {
    client: Client,
    peer: PeerRef,
    max_bytes: u64,
    can_post: bool,
    can_delete: bool,
    channel_title: String,
    preferred_message:Option<i32>,
}
pub(super) struct IndexDocument {pub revision:i64,pub document:RemoteDocument}
impl GrammersTransport {
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
            rows.push(serde_json::json!({"text_is_json":serde_json::from_str::<serde_json::Value>(message.text()).is_ok(),"text_is_heading":message.text()=="映取备份","rich_documents":count,"rich_part":part,"rich_blocks":blocks,"legacy_media":raw.media.is_some()}));
        }
        Ok(serde_json::json!(rows))
    }
    /// Read one fixed high-water prefix; gaps in unrelated message IDs are legal.
    pub(super) async fn indexes(&self, io: &Io) -> Result<(i64, Vec<IndexDocument>)> {
        self.documents(io, false).await
    }
    async fn documents(&self, io: &Io, media: bool) -> Result<(i64, Vec<IndexDocument>)> {
        let mut newest = self.client.iter_messages(self.peer).limit(1);
        let high = io
            .run(async { newest.next().await.map_err(rpc_error) })
            .await?
            .map_or(0, |m| m.id());
        let mut history = self
            .client
            .iter_messages(self.peer)
            .offset_id(high.checked_add(1).context("TELEGRAM_INDEX_INVALID")?);
        let mut result = vec![];
        while let Some(message) = io
            .run(async { history.next().await.map_err(rpc_error) })
            .await?
        {
            io.touch();
            if message.id() > high {
                continue;
            }
            let message=io.run(self.complete_message(message)).await?;
            if let Some(documents)=bundle::documents(&message)? {
                for (evidence,_) in documents {
                    let envelope:Envelope=serde_json::from_str(&evidence.caption).context("TELEGRAM_INDEX_INVALID")?;
                    if envelope.kind=="event" && !media || envelope.kind!="event" && media {
                        result.push(IndexDocument {revision:if envelope.kind=="event" {bundle::event_revision(&message,&envelope)?} else {i64::from(message.id())},document:evidence});
                    }
                }
                continue;
            }
            let Ok(raw) = serde_json::from_str::<serde_json::Value>(message.text()) else {
                continue;
            };
            if raw["yingqu_media"] != 2 || raw["kind"] != "library" || media
            {
                continue;
            }
            let envelope: Envelope =
                serde_json::from_value(raw).context("TELEGRAM_INDEX_INVALID")?;
            let evidence = document(&message)?.0;
            if evidence.bytes != envelope.bytes
                || !media && (envelope.container != "json" || evidence.bytes > 16 * 1024 * 1024)
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            result.push(IndexDocument {revision:i64::from(evidence.message_id),document:evidence});
        }
        result.sort_by_key(|d| d.revision);
        Ok((result.iter().map(|d|d.revision).max().unwrap_or(0).max(i64::from(high)), result))
    }
    pub(super) async fn index_body(&self, expected: RemoteDocument, io: &Io) -> Result<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let envelope: Envelope =
            serde_json::from_str(&expected.caption).context("TELEGRAM_INDEX_INVALID")?;
        for attempt in 0..2 {
            let result = async {
                let mut reader = io.run(self.download(expected.clone())).await?;
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
        premium: bool,
        io: &Io,
    ) -> Result<Self> {
        target.validate()?;
        let mut dialogs = client.iter_dialogs();
        let mut found = None;
        while let Some(dialog) = io
            .run(async { dialogs.next().await.map_err(rpc_error) })
            .await?
        {
            if let Peer::Channel(channel) = dialog.peer() {
                if channel.id().bare_id_unchecked().to_string() != target.channel_id {
                    continue;
                }
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
                    let tl::enums::ChatAdminRights::Rights(r) = r;
                    r
                });
                let can_post = channel.raw.creator || rights.is_some_and(|r| r.post_messages);
                let can_delete = channel.raw.creator || rights.is_some_and(|r| r.delete_messages);
                let peer = channel
                    .to_ref()
                    .await
                    .map_err(|_| anyhow::anyhow!("TELEGRAM_CHANNEL_FORBIDDEN"))?
                    .context("TELEGRAM_CHANNEL_FORBIDDEN")?;
                found = Some((peer, can_post, can_delete, channel.title().to_owned()));
                break;
            }
        }
        let (peer, can_post, can_delete, channel_title) = found.context("TELEGRAM_CHANNEL_FORBIDDEN")?;
        let mut history = client.iter_messages(peer).limit(1);
        io.run(async { history.next().await.map_err(rpc_error) })
            .await?;
        let config = io
            .run(async {
                client
                    .invoke(&tl::functions::help::GetAppConfig { hash: 0 })
                    .await
                    .map_err(rpc_error)
            })
            .await?;
        let tl::enums::help::AppConfig::Config(config) = config else {
            bail!("TELEGRAM_LIMIT_UNAVAILABLE");
        };
        let max_bytes = limit(&config.config, premium)?;
        Ok(Self {
            client,
            peer,
            max_bytes,
            can_post,
            can_delete,
            channel_title,
            preferred_message:None,
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
    type Uploaded = Uploaded;
    fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    fn can_upload(&self) -> bool {
        self.can_post
    }
    fn can_delete(&self) -> bool {
        self.can_delete
    }
    fn media<'a>(&'a self, io: &'a Io) -> ProviderFuture<'a, Vec<RemoteDocument>> {
        Box::pin(async move { Ok(self.documents(io, true).await?.1.into_iter().map(|d|d.document).collect()) })
    }
    fn remove(&self, expected: RemoteDocument) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            if !self.can_delete {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            match self.remove_bundle_attachment(&expected).await {
                Ok(true)=>return Ok(()),
                Err(e) if e.to_string()=="TELEGRAM_MEDIA_MISSING"=>return Ok(()),
                Err(e)=>return Err(e),
                Ok(false)=>{},
            }
            let envelope:Envelope=serde_json::from_str(&expected.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.kind!="library" {bail!("TELEGRAM_INDEX_INVALID");}
            let actual = match self.message(expected.message_id).await {
                Ok(message) => document(&message)?.0,
                Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
                Err(e) => return Err(e),
            };
            if actual != expected {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            self.client
                .delete_messages(self.peer, &[expected.message_id])
                .await
                .map_err(rpc_error)?;
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
    ) -> ProviderFuture<'a, Uploaded> {
        Box::pin(async move {
            self.client
                .upload_stream(
                    &mut reader,
                    usize::try_from(bytes).context("TELEGRAM_FILE_TOO_LARGE")?,
                    name,
                )
                .await
                .map_err(io_error)
        })
    }
    fn send<'a>(
        &'a self,
        uploaded: Uploaded,
        caption: String,
        random_id: i64,
        video: Option<VideoAttributes>,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<i32>> {
        Box::pin(async move {
            let envelope:Envelope=serde_json::from_str(&caption).context("BACKUP_INVALID_MEDIA")?;
            if envelope.kind!="library" {return self.send_bundle(uploaded,envelope,random_id,video,io).await;}
            let name = match &uploaded.raw {
                tl::enums::InputFile::File(f) => f.name.clone(),
                tl::enums::InputFile::Big(f) => f.name.clone(),
                _ => bail!("BACKUP_INVALID_MEDIA"),
            };
            let (mime_type, attributes) = document_attributes(name, video.as_ref());
            let media = tl::types::InputMediaUploadedDocument {
                nosound_video: false,
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
            let mut history = self.client.iter_messages(self.peer);
            let mut found = None;
            while let Some(message) = history.next().await.map_err(rpc_error)? {
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
            Ok(Box::new(DownloadReader {
                iter: Some(self.client.iter_download(&OriginalDocument(document))),
                next: None,
                buffer: vec![],
                offset: 0,
            }) as Box<dyn AsyncRead + Unpin + Send>)
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
