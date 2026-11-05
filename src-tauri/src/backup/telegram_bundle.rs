//! Asset-scoped rich messages. New edits append immutable events to the original video message.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    document_id: i64,
    envelope: Envelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_sequence: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    #[serde(skip)]
    legacy: bool,
    yingqu_bundle: u32,
    heading: String,
    attachments: Vec<Attachment>,
}
struct Loaded {
    bundle: Bundle,
    documents: Vec<tl::types::Document>,
}
pub(super) fn display_heading(title: Option<&str>, filename: Option<&str>) -> Option<String> {
    [title, filename]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|value| value.chars().take(300).collect())
}
impl Bundle {
    fn validate(&self) -> Result<()> {
        if !(self.yingqu_bundle == 1 || self.legacy && self.yingqu_bundle == 2)
            || self.attachments.is_empty()
            || self.attachments.len() > 256
            || self.heading.chars().count() > 300
        {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        let first = &self.attachments[0].envelope;
        let mut kinds = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        let mut owners = BTreeMap::new();
        let mut documents = BTreeMap::new();
        let mut legacy_event = false;
        for a in &self.attachments {
            let e = &a.envelope;
            let content = (&e.sha256, e.bytes, &e.container);
            if documents
                .insert(a.document_id, content)
                .is_some_and(|old| old != content)
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            if !matches!(e.kind.as_str(), "video" | "cover" | "transcript" | "event")
                || e.yingqu_media != 2
                || e.library_id != first.library_id
                || e.asset_id != first.asset_id
                || a.document_id == 0
                || !kinds.insert((&e.operation_id, &e.kind))
                || owners
                    .insert(&e.operation_id, &e.device_id)
                    .is_some_and(|old| old != &e.device_id)
                || e.kind != "event" && a.event_sequence.is_some()
                || e.kind == "event"
                    && match a.event_sequence {
                        Some(s) => s == 0 || !sequences.insert(s),
                        None => !self.legacy || std::mem::replace(&mut legacy_event, true),
                    }
                || e.bytes == 0
                || e.sha256.len() != 64
                || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || e.container.is_empty()
                || e.container.len() > 12
                || !e.container.bytes().all(|b| b.is_ascii_alphanumeric())
                || e.kind == "event" && (e.container != "json" || e.bytes > 16 * 1024 * 1024)
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            e.caption()?;
        }
        Ok(())
    }
    fn caption(&self) -> Result<String> {
        self.validate()?;
        let text = serde_json::to_string(self)?;
        if text.len() > 144 * 1024 {
            bail!("BACKUP_METADATA_TOO_LARGE");
        }
        Ok(text)
    }
    fn matches(&self, e: &Envelope) -> bool {
        self.attachments.iter().any(|a| {
            a.envelope.library_id == e.library_id
                && a.envelope.asset_id == e.asset_id
                && a.envelope.operation_id == e.operation_id
                && a.envelope.device_id == e.device_id
        })
    }
}
fn read(message: &grammers_client::message::Message) -> Result<Option<Loaded>> {
    let tl::enums::Message::Message(raw) = &message.raw else {
        return Ok(None);
    };
    let Some(tl::enums::RichMessage::Message(rich)) = &raw.rich_message else {
        return Ok(None);
    };
    let Some(bundle) = manifest(rich)? else {
        return Ok(None);
    };
    Ok(Some(load(bundle, rich, raw.grouped_id.is_some())?))
}
const ANCHOR_PREFIX: &str = "yingqu_asset_v1_";
fn manifest(rich: &tl::types::RichMessage) -> Result<Option<Bundle>> {
    let mut parts = BTreeMap::new();
    let mut total = None;
    let mut legacy = None;
    let mut single = None;
    for block in &rich.blocks {
        let tl::enums::PageBlock::Anchor(anchor) = block else {
            continue;
        };
        let (encoded, old) = if let Some(encoded) = anchor.name.strip_prefix(ANCHOR_PREFIX) { (encoded, false) }
            else if let Some(encoded) = anchor.name.strip_prefix("yingqu_bundle_v1_") { (encoded, true) }
            else {
            continue;
        };
        if legacy.is_some_and(|previous| previous != old) { bail!("TELEGRAM_INDEX_INVALID"); }
        legacy = Some(old);
        if encoded.len() > 4096 {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        if let Some(part) = encoded.strip_prefix("part_") {
            let mut values = part.splitn(3, '_');
            let index = values
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .context("TELEGRAM_INDEX_INVALID")?;
            let count = values
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .context("TELEGRAM_INDEX_INVALID")?;
            let body = values.next().context("TELEGRAM_INDEX_INVALID")?;
            if count == 0
                || count > 64
                || index >= count
                || total.is_some_and(|t| t != count)
                || parts.insert(index, body).is_some()
            {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            total = Some(count);
            continue;
        }
        if !old || single.replace(encoded).is_some() { bail!("TELEGRAM_INDEX_INVALID"); }
    }
    let encoded = if let Some(total) = total {
        if single.is_some() { bail!("TELEGRAM_INDEX_INVALID"); }
        if parts.len() != total {
            bail!("TELEGRAM_INDEX_INVALID");
        }
        Some((0..total)
            .map(|i| parts.get(&i).copied().context("TELEGRAM_INDEX_INVALID"))
            .collect::<Result<String>>()?)
    } else { single.map(str::to_owned) };
    if let Some(encoded) = encoded {
        let mut bundle: Bundle = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(encoded)
                .context("TELEGRAM_INDEX_INVALID")?,
        )
        .context("TELEGRAM_INDEX_INVALID")?;
        bundle.legacy = legacy.unwrap_or(false);
        bundle.validate()?;
        return Ok(Some(bundle));
    }
    Ok(None)
}
fn load(bundle: Bundle, rich: &tl::types::RichMessage, grouped: bool) -> Result<Loaded> {
    bundle.validate()?;
    if rich.part
        || grouped
        || rich.documents.len()
            != bundle
                .attachments
                .iter()
                .map(|a| a.document_id)
                .collect::<BTreeSet<_>>()
                .len()
    {
        bail!("TELEGRAM_INDEX_INVALID");
    }
    let mut documents = vec![];
    for a in &bundle.attachments {
        let d = rich
            .documents
            .iter()
            .find_map(|d| match d {
                tl::enums::Document::Document(d) if d.id == a.document_id => Some(d.clone()),
                _ => None,
            })
            .context("TELEGRAM_MEDIA_MISSING")?;
        if u64::try_from(d.size).ok() != Some(a.envelope.bytes) {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        if !documents
            .iter()
            .any(|old: &tl::types::Document| old.id == d.id)
        {
            documents.push(d);
        }
    }
    Ok(Loaded { bundle, documents })
}

pub(super) fn documents(
    message: &grammers_client::message::Message,
) -> Result<Option<Vec<(RemoteDocument, tl::types::Document)>>> {
    read(message)?
        .map(|loaded| {
            loaded
                .bundle
                .attachments
                .iter()
                .map(|a| {
                    let d = loaded
                        .documents
                        .iter()
                        .find(|d| d.id == a.document_id)
                        .context("TELEGRAM_MEDIA_MISSING")?
                        .clone();
                    Ok((
                        RemoteDocument {
                            message_id: message.id(),
                            document_id: d.id,
                            bytes: a.envelope.bytes,
                            caption: a.envelope.caption()?,
                        },
                        d,
                    ))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()
}
/// Every event uses (physical message ID, sequence); edits keep the original video.
pub(super) fn event_revision(
    message: &grammers_client::message::Message,
    envelope: &Envelope,
) -> Result<i64> {
    let loaded = read(message)?.context("TELEGRAM_INDEX_INVALID")?;
    let a = loaded
        .bundle
        .attachments
        .iter()
        .find(|a| &a.envelope == envelope)
        .context("TELEGRAM_INDEX_INVALID")?;
    match a.event_sequence {
        Some(sequence) => Ok(revision(message.id(), sequence)),
        None if loaded.bundle.legacy => Ok(i64::from(message.id())),
        _ => bail!("TELEGRAM_INDEX_INVALID"),
    }
}
fn revision(message: i32, sequence: u32) -> i64 {
    (i64::from(message) << 32) | i64::from(sequence)
}
fn page_caption(text: &str) -> tl::enums::PageCaption {
    tl::types::PageCaption {
        text: tl::types::TextPlain { text: text.into() }.into(),
        credit: tl::types::TextPlain {
            text: String::new(),
        }
        .into(),
    }
    .into()
}
fn rich_input(loaded: &Loaded) -> Result<tl::enums::InputRichMessage> {
    let encoded = URL_SAFE_NO_PAD.encode(loaded.bundle.caption()?.as_bytes());
    let chunks = encoded.as_bytes().chunks(3072).collect::<Vec<_>>();
    if chunks.len() > 64 {
        bail!("BACKUP_METADATA_TOO_LARGE");
    }
    let mut blocks = chunks
        .iter()
        .enumerate()
        .map(|(i, body)| {
            let body = std::str::from_utf8(body).expect("base64 is ASCII");
            tl::types::PageBlockAnchor {
                name: format!("{ANCHOR_PREFIX}part_{i}_{}_{}", chunks.len(), body),
            }
            .into()
        })
        .collect::<Vec<tl::enums::PageBlock>>();
    blocks.push(
        tl::types::PageBlockParagraph {
            text: tl::types::TextPlain {
                text: loaded.bundle.heading.clone(),
            }
            .into(),
        }
        .into(),
    );
    let mut latest = BTreeMap::new();
    for (i, a) in loaded.bundle.attachments.iter().enumerate() {
        latest.insert(&a.envelope.kind, i);
    }
    let mut history = vec![];
    for (i, a) in loaded.bundle.attachments.iter().enumerate() {
        let label = match a.envelope.kind.as_str() {
            "video" => "原视频",
            "cover" => "原始封面",
            "transcript" => "文案",
            _ => "备份元数据",
        };
        let video = loaded
            .documents
            .iter()
            .find(|d| d.id == a.document_id)
            .is_some_and(|d| {
                d.attributes
                    .iter()
                    .any(|v| matches!(v, tl::enums::DocumentAttribute::Video(_)))
            });
        let block = if a.envelope.kind == "video" && video {
            tl::types::PageBlockVideo {
                autoplay: false,
                r#loop: false,
                spoiler: false,
                video_id: a.document_id,
                caption: page_caption(label),
            }
            .into()
        } else {
            tl::types::PageBlockDocument {
                document_id: a.document_id,
                caption: page_caption(label),
            }
            .into()
        };
        if latest[&a.envelope.kind] == i {
            blocks.push(block);
        } else {
            history.push(block);
        }
    }
    if !history.is_empty() {
        blocks.push(
            tl::types::PageBlockDetails {
                open: false,
                blocks: history,
                title: tl::types::TextPlain {
                    text: "历史版本".into(),
                }
                .into(),
            }
            .into(),
        );
    }
    Ok(tl::types::InputRichMessage {
        rtl: false,
        noautolink: true,
        blocks,
        photos: None,
        documents: Some(
            loaded
                .documents
                .iter()
                .map(|d| {
                    tl::types::InputDocument {
                        id: d.id,
                        access_hash: d.access_hash,
                        file_reference: d.file_reference.clone(),
                    }
                    .into()
                })
                .collect(),
        ),
        users: None,
    }
    .into())
}
fn owned_message_matches(loaded: &Loaded, expected: &[RemoteDocument]) -> bool {
    !loaded.bundle.legacy
        && !expected.is_empty()
        && loaded.bundle.attachments.len() == expected.len()
        && loaded.bundle.attachments.iter().all(|attachment| {
            expected.iter().any(|document| {
                document.document_id == attachment.document_id
                    && document.bytes == attachment.envelope.bytes
                    && attachment.envelope.caption().ok().as_deref()
                        == Some(document.caption.as_str())
            })
        })
}

impl crate::backup::telegram_lifecycle::OwnedMessageRemoval for GrammersTransport {
    async fn read_owned_ids(&self, groups: &[(i32, Vec<RemoteDocument>)], io: &Io) -> Result<Vec<i32>> {
        let ids: Vec<_> = groups.iter().map(|(id, _)| *id).collect();
        let messages = self.client.get_messages_by_id(self.peer, &ids).await.map_err(rpc_error)?;
        let mut verified = Vec::new();
        // grammers returns one slot for each requested ID, in request order.
        for ((id, expected), message) in groups.iter().zip(messages) {
            let Some(message) = message else { continue };
            if matches!(message.raw, tl::enums::Message::Empty(_)) {
                continue;
            }
            let complete = match io.run(self.complete_message(message)).await {
                Ok(message) => message,
                // A partial rich message can disappear between the batch read and its follow-up.
                Err(error) if error.to_string() == "TELEGRAM_MEDIA_MISSING" => continue,
                Err(error) => return Err(error),
            };
            if complete.id() != *id || expected.iter().any(|d| d.message_id != *id) {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            let matches = match read(&complete)? {
                Some(loaded) => owned_message_matches(&loaded, expected),
                None if matches!(&complete.raw, tl::enums::Message::Message(m) if m.rich_message.is_none() && m.grouped_id.is_none()) => {
                    let [old] = expected.as_slice() else { bail!("TELEGRAM_MEDIA_CHANGED"); };
                    let envelope: Envelope = serde_json::from_str(&old.caption)?;
                    envelope.kind == "library" && document(&complete)?.0 == *old
                }
                None => false,
            };
            if !matches {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            verified.push(*id);
        }
        Ok(verified)
    }

    async fn delete_verified_ids(&self, ids: &[i32]) -> Result<()> {
        self.client.delete_messages(self.peer, ids).await.map_err(rpc_error)?;
        Ok(())
    }

    async fn confirm_ids_gone(&self, ids: &[i32]) -> Result<()> {
        let messages = self.client.get_messages_by_id(self.peer, ids).await.map_err(rpc_error)?;
        if messages.into_iter().flatten().any(|m| !matches!(m.raw, tl::enums::Message::Empty(_))) {
            bail!("BACKUP_CLEANUP_PENDING");
        }
        Ok(())
    }
}

impl GrammersTransport {
    async fn operation_bundle(
        &self,
        envelope: &Envelope,
        io: &Io,
    ) -> Result<Option<(i32, Loaded)>> {
        let mut history = self.client.iter_messages(self.peer);
        let mut found = None;
        while let Some(message) = io
            .run(async { history.next().await.map_err(rpc_error) })
            .await?
        {
            io.touch();
            let message = io.run(self.complete_message(message)).await?;
            let Some(loaded) = read(&message)? else {
                continue;
            };
            if loaded.bundle.matches(envelope) {
                if found.is_some() {
                    bail!("TELEGRAM_DUPLICATE_OPERATION");
                }
                found = Some((message.id(), loaded));
            }
        }
        Ok(found)
    }
    async fn edit_bundle(&self, id: i32, loaded: &Loaded) -> Result<()> {
        if loaded.bundle.legacy { bail!("BACKUP_LEGACY_READ_ONLY"); }
        self.client
            .invoke(&tl::functions::messages::EditMessage {
                no_webpage: true,
                invert_media: false,
                peer: self.peer.into(),
                id,
                message: Some(loaded.bundle.heading.clone()),
                media: None,
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                quick_reply_shortcut_id: None,
                rich_message: Some(rich_input(loaded)?),
            })
            .await
            .map_err(rpc_error)?;
        Ok(())
    }
    pub(super) async fn send_bundle(
        &self,
        uploaded: PreparedUpload,
        envelope: Envelope,
        random_id: i64,
        video: Option<VideoAttributes>,
        io: &Io,
    ) -> Result<Option<i32>> {
        let existing = if envelope.kind == "event"
            && self.new_delete_operation.as_deref() == Some(&envelope.operation_id)
        {
            None
        } else {
            self.operation_bundle(&envelope, io).await?
        };
        let existing = match (existing, self.preferred_message) {
            (Some(existing), _) => Some(existing),
            (None, Some(id)) => {
                let message = io.run(self.message(id)).await?;
                if let Some(loaded) = read(&message)? {
                    let first = &loaded.bundle.attachments[0].envelope;
                    if first.library_id != envelope.library_id || first.asset_id != envelope.asset_id { bail!("BACKUP_SCOPE_MISMATCH"); }
                    Some((id, loaded))
                } else {
                    let first: Envelope = serde_json::from_str(message.text()).context("TELEGRAM_INDEX_INVALID")?;
                    if first.library_id != envelope.library_id || first.asset_id != envelope.asset_id || first.kind != "video" { bail!("BACKUP_SCOPE_MISMATCH"); }
                    None
                }
            }
            (None, None) => None,
        };
        let existing = existing.filter(|(_, loaded)| !loaded.bundle.legacy);
        let (id, mut loaded) = match existing {
            Some((id, loaded)) => (Some(id), loaded),
            None => (
                None,
                Loaded {
                    bundle: Bundle {
                        legacy: false,
                        yingqu_bundle: 1,
                        heading: self.bundle_heading.clone().unwrap_or_else(|| "映取备份".into()),
                        attachments: vec![],
                    },
                    documents: vec![],
                },
            ),
        };
        if let Some(heading) = &self.bundle_heading {
            loaded.bundle.heading = heading.clone();
        }
        if let Some(old) = loaded.bundle.attachments.iter().find(|a| {
            a.envelope.operation_id == envelope.operation_id && a.envelope.kind == envelope.kind
        }) {
            if old.envelope != envelope {
                bail!("TELEGRAM_OPERATION_MISMATCH");
            }
            return Ok(id);
        }
        if loaded
            .bundle
            .attachments
            .iter()
            .any(|a| a.envelope.operation_id == envelope.operation_id && a.envelope.kind == "event")
        {
            bail!("TELEGRAM_OPERATION_MISMATCH");
        }
        let media = self
            .client
            .invoke(&tl::functions::messages::UploadMedia {
                business_connection_id: None,
                peer: self.peer.into(),
                media: uploaded_media(uploaded.file, video.as_ref(), uploaded.thumbnail)?,
            })
            .await
            .map_err(rpc_error)?;
        let tl::enums::MessageMedia::Document(media) = media else {
            bail!("TELEGRAM_MEDIA_CHANGED");
        };
        let Some(tl::enums::Document::Document(document)) = media.document else {
            bail!("TELEGRAM_MEDIA_MISSING");
        };
        if u64::try_from(document.size).ok() != Some(envelope.bytes) {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        let event_sequence = if envelope.kind == "event" {
            Some(
                loaded
                    .bundle
                    .attachments
                    .iter()
                    .filter_map(|a| a.event_sequence)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .context("BACKUP_METADATA_TOO_LARGE")?,
            )
        } else {
            None
        };
        loaded.bundle.attachments.push(Attachment {
            document_id: document.id,
            envelope,
            event_sequence,
        });
        if !loaded.documents.iter().any(|d| d.id == document.id) {
            loaded.documents.push(document);
        }
        if let Some(id) = id {
            self.edit_bundle(id, &loaded).await?;
            Ok(Some(id))
        } else {
            let updates = self
                .client
                .invoke(&tl::functions::messages::SendMessage {
                    no_webpage: true,
                    silent: true,
                    background: false,
                    clear_draft: false,
                    noforwards: false,
                    update_stickersets_order: false,
                    invert_media: false,
                    allow_paid_floodskip: false,
                    peer: self.peer.into(),
                    reply_to: None,
                    message: loaded.bundle.heading.clone(),
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
                    rich_message: Some(rich_input(&loaded)?),
                })
                .await
                .map_err(rpc_error)?;
            Ok(sent_id(updates, random_id))
        }
    }
    pub(super) async fn remove_bundle_attachment(&self, expected: &RemoteDocument, message: &grammers_client::message::Message) -> Result<bool> {
        let Some(mut loaded) = read(message)? else {
            return Ok(false);
        };
        if loaded.bundle.legacy { bail!("BACKUP_LEGACY_READ_ONLY"); }
        let Some(a) = loaded.bundle.attachments.iter().find(|a| {
            a.document_id == expected.document_id
                && a.envelope.caption().ok().as_deref() == Some(expected.caption.as_str())
        }) else {
            return Ok(true);
        };
        let current = RemoteDocument {
            message_id: expected.message_id,
            document_id: a.document_id,
            bytes: a.envelope.bytes,
            caption: a.envelope.caption()?,
        };
        if &current != expected
            || loaded
                .bundle
                .attachments
                .iter()
                .any(|a| a.document_id == expected.document_id && a.envelope.kind == "event")
        {
            bail!("TELEGRAM_MEDIA_CHANGED");
        }
        loaded
            .bundle
            .attachments
            .retain(|a| a.document_id != expected.document_id);
        loaded.documents.retain(|d| d.id != expected.document_id);
        if loaded.documents.is_empty() {
            self.client
                .delete_messages(self.peer, &[expected.message_id])
                .await
                .map_err(rpc_error)?;
        } else {
            self.edit_bundle(expected.message_id, &loaded).await?;
        }
        match self.message(expected.message_id).await {
            Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => {}
            Ok(message) => {
                if documents(&message)?
                    .context("TELEGRAM_MEDIA_CHANGED")?
                    .iter()
                    .any(|(d, _)| d.document_id == expected.document_id)
                {
                    bail!("BACKUP_CLEANUP_PENDING");
                }
            }
            Err(e) => return Err(e),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_heading_prefers_title_and_falls_back_to_filename() {
        assert_eq!(
            display_heading(Some("  抖音原视频标题  "), Some("download.mp4")).as_deref(),
            Some("抖音原视频标题")
        );
        for title in [None, Some(""), Some(" \n\t")] {
            assert_eq!(
                display_heading(title, Some(" 原始文件.mov ")).as_deref(),
                Some("原始文件.mov")
            );
        }
        assert_eq!(display_heading(None, Some(" \t")), None);
        let title = "中文🎬".repeat(150);
        let heading = display_heading(Some(&title), Some("file.mp4")).unwrap();
        assert_eq!(heading, title.chars().take(300).collect::<String>());
    }

    #[test]
    fn video_title_is_plain_text_and_does_not_change_restore_evidence() {
        for heading in ["抖音标题 🎬 <b>原文</b> & #标签", "原视频.mov", "映取备份"]
        {
            let mut f = fixture();
            let envelopes = f
                .bundle
                .attachments
                .iter()
                .map(|a| a.envelope.caption().unwrap())
                .collect::<Vec<_>>();
            f.bundle.heading = display_heading(Some(heading), None).unwrap();
            let rich = received(&f);
            assert!(rich.blocks.iter().any(|block| matches!(block,
            tl::enums::PageBlock::Paragraph(p) if matches!(&p.text, tl::enums::RichText::TextPlain(t) if t.text == heading)
        )));
            let loaded = load(manifest(&rich).unwrap().unwrap(), &rich, false).unwrap();
            assert_eq!(loaded.bundle.heading, heading);
            assert_eq!(
                loaded
                    .bundle
                    .attachments
                    .iter()
                    .map(|a| a.envelope.caption().unwrap())
                    .collect::<Vec<_>>(),
                envelopes
            );
            assert_eq!(loaded.documents[0].id, f.documents[0].id);
        }
    }
    fn fixture() -> Loaded {
        let attachments = ["video", "cover", "transcript", "event"]
            .into_iter()
            .enumerate()
            .map(|(i, kind)| Attachment {
                document_id: 100 + i as i64,
                event_sequence: (kind == "event").then_some(1),
                envelope: Envelope {
                    generation: None,
                    yingqu_media: 2,
                    library_id: uuid::Uuid::nil().to_string(),
                    asset_id: uuid::Uuid::nil().to_string(),
                    operation_id: uuid::Uuid::nil().to_string(),
                    device_id: uuid::Uuid::nil().to_string(),
                    kind: kind.into(),
                    bytes: 17 + i as u64,
                    sha256: "ab".repeat(32),
                    container: if i < 2 { "mp4" } else { "json" }.into(),
                },
            })
            .collect::<Vec<_>>();
        let documents = attachments
            .iter()
            .map(|a| tl::types::Document {
                id: a.document_id,
                access_hash: 0,
                file_reference: vec![],
                date: 0,
                mime_type: "application/octet-stream".into(),
                size: a.envelope.bytes as i64,
                thumbs: None,
                video_thumbs: None,
                dc_id: 1,
                attributes: if a.envelope.kind == "video" {
                    document_attributes(
                        "original.mp4".into(),
                        Some(&VideoAttributes {
                            duration_ms: 2000,
                            width: 320,
                            height: 240,
                            supports_streaming: true,
                        }),
                    )
                    .1
                } else {
                    vec![]
                },
            })
            .collect();
        Loaded {
            bundle: Bundle {
                legacy: false,
                yingqu_bundle: 1,
                heading: "原始文件".into(),
                attachments,
            },
            documents,
        }
    }
    fn received(f: &Loaded) -> tl::types::RichMessage {
        let tl::enums::InputRichMessage::Message(input) = rich_input(f).unwrap() else {
            panic!("rich input required");
        };
        tl::types::RichMessage {
            rtl: false,
            part: false,
            blocks: input.blocks,
            photos: vec![],
            documents: f.documents.iter().cloned().map(Into::into).collect(),
        }
    }
    fn append_transcript_update(f: &mut Loaded, sequence: u32) {
        let operation = uuid::Uuid::new_v4().to_string();
        for (offset, kind) in ["transcript", "event"].into_iter().enumerate() {
            let mut a = f
                .bundle
                .attachments
                .iter()
                .find(|a| a.envelope.kind == kind)
                .unwrap()
                .clone();
            let mut d = f
                .documents
                .iter()
                .find(|d| d.id == a.document_id)
                .unwrap()
                .clone();
            a.document_id = 1000 + i64::from(sequence) * 2 + offset as i64;
            a.envelope.operation_id = operation.clone();
            a.event_sequence = (kind == "event").then_some(sequence);
            d.id = a.document_id;
            f.bundle.attachments.push(a);
            f.documents.push(d);
        }
    }
    #[test]
    fn later_transcript_keeps_original_video_and_events_but_renders_only_latest_text() {
        let mut f = fixture();
        let old = f.bundle.attachments[3].envelope.caption().unwrap();
        append_transcript_update(&mut f, 2);
        let raw = received(&f);
        let loaded = load(manifest(&raw).unwrap().unwrap(), &raw, false).unwrap();
        assert_eq!(loaded.bundle.attachments[0].document_id, 100);
        assert_eq!(
            loaded.bundle.attachments[3].envelope.caption().unwrap(),
            old
        );
        assert_eq!(loaded.bundle.attachments[3].event_sequence, Some(1));
        assert_eq!(loaded.bundle.attachments[5].event_sequence, Some(2));
        let tl::enums::InputRichMessage::Message(input) = rich_input(&loaded).unwrap() else {
            panic!("rich input");
        };
        assert_eq!(
            input
                .blocks
                .iter()
                .filter(|b| matches!(b, tl::enums::PageBlock::Document(_)))
                .count(),
            3
        );
        let details = input
            .blocks
            .iter()
            .find_map(|b| {
                if let tl::enums::PageBlock::Details(d) = b {
                    Some(d)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(!details.open);
        assert_eq!(details.blocks.len(), 2);
        assert!(details
            .blocks
            .iter()
            .any(|b| matches!(b,tl::enums::PageBlock::Document(d) if d.document_id==102)));
    }
    #[test]
    fn legacy_single_and_chunked_manifests_keep_unsequenced_event_identity() {
        for chunked in [false, true] {
            let mut f = fixture();
            f.bundle.attachments.iter_mut().find(|a| a.envelope.kind == "event").unwrap().event_sequence = None;
            let mut raw = received(&fixture());
            raw.blocks.retain(|b| !matches!(b, tl::enums::PageBlock::Anchor(_)));
            let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&f.bundle).unwrap());
            let anchors = if chunked { encoded.as_bytes().chunks(700).map(|s|std::str::from_utf8(s).unwrap().to_owned()).collect::<Vec<_>>() } else { vec![encoded] };
            for (i,part) in anchors.iter().enumerate() {
                raw.blocks.push(tl::types::PageBlockAnchor { name:if chunked {format!("yingqu_bundle_v1_part_{i}_{}_{}",anchors.len(),part)} else {format!("yingqu_bundle_v1_{part}")} }.into());
            }
            let bundle = manifest(&raw).unwrap().unwrap();
            assert!(bundle.legacy);
            assert!(bundle.attachments.iter().find(|a|a.envelope.kind=="event").unwrap().event_sequence.is_none());
            let loaded=load(bundle, &raw, false).unwrap();
            assert_eq!(loaded.documents.len(), 4);
            let expected=loaded.bundle.attachments.iter().map(|a|RemoteDocument{message_id:1,document_id:a.document_id,bytes:a.envelope.bytes,caption:a.envelope.caption().unwrap()}).collect::<Vec<_>>();
            assert!(!owned_message_matches(&loaded,&expected));
        }
    }
    #[test]
    fn repeated_transcripts_chunk_manifest_and_preserve_all_versions() {
        let mut f = fixture();
        for n in 2..=9 {
            append_transcript_update(&mut f, n);
        }
        let mut raw = received(&f);
        assert!(
            raw.blocks
                .iter()
                .filter(|b| matches!(b, tl::enums::PageBlock::Anchor(_)))
                .count()
                > 1
        );
        let loaded = load(manifest(&raw).unwrap().unwrap(), &raw, false).unwrap();
        assert_eq!(loaded.bundle.attachments.len(), 20);
        raw.blocks.remove(0);
        assert!(manifest(&raw).is_err());
    }
    #[test]
    fn event_revisions_advance_inside_original_video_message() {
        assert!(revision(42, 1) > i64::from(i32::MAX));
        assert!(revision(42, 2) > revision(42, 1));
        assert!(revision(43, 1) > revision(42, u32::MAX));
        assert_eq!(revision(i32::MAX, u32::MAX), i64::MAX);
    }
    #[test]
    fn rich_manifest_survives_server_rebuilt_fallback_and_rejects_duplicate_anchors() {
        let f = fixture();
        let mut raw = received(&f);
        let decoded = manifest(&raw).unwrap().unwrap();
        assert_eq!(decoded.caption().unwrap(), f.bundle.caption().unwrap());
        // The fallback message text is never consulted; the anchor carries typed bindings.
        raw.blocks.push(raw.blocks[0].clone());
        assert!(manifest(&raw).is_err());
    }
    #[test]
    fn bundle_has_one_streaming_video_three_original_documents_and_stable_envelopes() {
        let f = fixture();
        let tl::enums::InputRichMessage::Message(input) = rich_input(&f).unwrap() else {
            panic!("expected rich input");
        };
        assert_eq!(input.documents.unwrap().len(), 4);
        assert!(input.photos.is_none());
        assert_eq!(
            input
                .blocks
                .iter()
                .filter(|b| matches!(b, tl::enums::PageBlock::Video(_)))
                .count(),
            1
        );
        assert_eq!(
            input
                .blocks
                .iter()
                .filter(|b| matches!(b, tl::enums::PageBlock::Document(_)))
                .count(),
            3
        );
        let loaded = load(
            serde_json::from_str(&f.bundle.caption().unwrap()).unwrap(),
            &received(&f),
            false,
        )
        .unwrap();
        for (a, d) in loaded.bundle.attachments.iter().zip(&loaded.documents) {
            assert_eq!(a.document_id, d.id);
            assert_eq!(a.envelope.bytes, d.size as u64);
        }
    }
    #[test]
    fn bundle_rejects_partial_duplicate_retargeted_or_changed_documents() {
        let f = fixture();
        let mut raw = received(&f);
        raw.part = true;
        assert!(load(f.bundle.clone(), &raw, false).is_err());
        assert!(load(f.bundle.clone(), &received(&f), true).is_err());
        let mut b = f.bundle.clone();
        b.attachments[1].document_id = b.attachments[0].document_id;
        assert!(b.validate().is_err());
        let mut b = f.bundle.clone();
        b.attachments[1].envelope.kind = "video".into();
        assert!(b.validate().is_err());
        let mut b = f.bundle.clone();
        b.attachments[1].envelope.asset_id = uuid::Uuid::new_v4().to_string();
        assert!(b.validate().is_err());
        let mut raw = received(&f);
        raw.documents.pop();
        assert!(load(f.bundle.clone(), &raw, false).is_err());
        let mut raw = received(&f);
        if let tl::enums::Document::Document(d) = &mut raw.documents[1] {
            d.size += 1;
        }
        assert!(load(f.bundle, &raw, false).is_err());
    }
    #[test]
    fn removing_media_preserves_event_document_and_exact_index_envelope() {
        let mut f = fixture();
        let original = f.bundle.attachments[3].envelope.caption().unwrap();
        f.bundle.attachments.retain(|a| a.envelope.kind == "event");
        f.documents.retain(|d| d.id == 103);
        let loaded = load(f.bundle.clone(), &received(&f), false).unwrap();
        assert_eq!(loaded.bundle.attachments.len(), 1);
        assert_eq!(
            loaded.bundle.attachments[0].envelope.caption().unwrap(),
            original
        );
        assert_eq!(loaded.documents[0].id, 103);
    }

    #[test]
    fn whole_message_removal_requires_all_exact_archived_and_media_documents() {
        let loaded=fixture();
        let expected:Vec<_>=loaded.bundle.attachments.iter().map(|a|RemoteDocument {message_id:10,document_id:a.document_id,bytes:a.envelope.bytes,caption:a.envelope.caption().unwrap()}).collect();
        assert!(owned_message_matches(&loaded,&expected));
        assert!(!owned_message_matches(&loaded,&expected[1..]),"must not delete unlisted attachments");
        let mut changed=expected.clone();changed[0].document_id+=100;
        assert!(!owned_message_matches(&loaded,&changed));
        let mut changed=expected.clone();changed[0].caption.push(' ');
        assert!(!owned_message_matches(&loaded,&changed));
        assert!(!owned_message_matches(&loaded,&[]));
    }
}
