//! Exact document ownership checks; full readable hashes precede local release.
use super::{
    provider::*,
    telegram_replay::Projection,
    telegram_transfer::{Envelope, Io, RemoteDocument, Target, Transport},
    telegram_transfer_journal::{Entry, SendState},
};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

pub(super) fn in_use(target: &Target, projected: &Projection, message: i32) -> bool {
    projected
        .assets
        .iter()
        .filter(|(_, s)| !s.deleted)
        .any(|(asset, _)| {
            projected
                .references
                .get(asset)
                .into_iter()
                .flatten()
                .any(|r| target.message_id(r).ok() == Some(message))
        })
}
pub(super) trait OwnedMessageRemoval: Transport {
    fn read_owned_ids(&self, groups: &[(i32, Vec<RemoteDocument>)], io: &Io)
        -> impl std::future::Future<Output = Result<Vec<i32>>> + Send;
    fn delete_verified_ids(&self, ids: &[i32]) -> impl std::future::Future<Output = Result<()>> + Send;
    fn confirm_ids_gone(&self, ids: &[i32]) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Validate every group before sending the first deletion, even when the asset spans multiple batches.
pub(super) async fn reconcile_snapshot(
    transport: &impl OwnedMessageRemoval,
    target: &Target,
    asset: &str,
    io: &Io,
    projected: &Projection,
    media: Vec<RemoteDocument>,
    archived: Vec<(i32, Vec<RemoteDocument>)>,
) -> Result<usize> {
    if !transport.can_delete() {
        bail!("BACKUP_DELETE_UNSUPPORTED");
    }
    let deletion = projected
        .assets
        .get(asset)
        .filter(|s| s.deleted)
        .context("BACKUP_TOMBSTONE_REQUIRED")?
        .revision;
    let mut groups: std::collections::BTreeMap<i32, Vec<RemoteDocument>> =
        archived.into_iter().collect();
    for document in media {
        io.check()?;
        let known = projected
            .references
            .get(asset)
            .into_iter()
            .flatten()
            .find(|r| {
                target.message_id(r).ok() == Some(document.message_id)
                    && r.document_id() == Some(document.document_id)
                    && r.bytes == document.bytes
                    && serde_json::from_str::<Envelope>(&document.caption)
                        .is_ok_and(|e| e.sha256 == r.sha256 && e.container == r.container)
            });
        let Some(reference) = candidate(target, asset, deletion, &document, known)? else {
            continue;
        };
        if in_use(target, projected, document.message_id) {
            bail!("BACKUP_MEDIA_IN_USE");
        }
        if known.is_none() {
            verify(transport, target, &reference, io).await?;
        }
        groups
            .entry(document.message_id)
            .or_default()
            .push(document);
    }
    if groups.keys().any(|&id| in_use(target, projected, id)) {
        bail!("BACKUP_MEDIA_IN_USE");
    }
    let groups: Vec<_> = groups.into_iter().collect();
    let mut verified = Vec::new();
    const BATCH_SIZE: usize = 100;
    for batch in groups.chunks(BATCH_SIZE) {
        let _read = super::delete_timing::setup_span("read_owned_message");
        verified.extend(io.run(transport.read_owned_ids(batch, io)).await?);
    }
    for batch in verified.chunks(BATCH_SIZE) {
        {
            let _delete = super::delete_timing::setup_span("delete_message");
            io.run(transport.delete_verified_ids(batch)).await?;
        }
        let _confirm = super::delete_timing::setup_span("confirm_message_gone");
        io.run(transport.confirm_ids_gone(batch)).await?;
    }
    Ok(groups.len())
}

pub(super) async fn cancel(
    transport: &impl Transport,
    target: &Target,
    entry: &Entry,
    projected: &Projection,
    io: &Io,
) -> Result<()> {
    if projected
        .operations
        .contains_key(&entry.envelope.operation_id)
    {
        return Ok(());
    }
    let (document, identity) = if let Some(original) = &entry.document {
        let current = match io.run(transport.inspect_document(original.message_id,original.document_id)).await {
            Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
            other => other?,
        };
        (current, Some(original.document_id))
    } else if entry.state == SendState::Sending {
        (
            io.run(transport.find(&entry.envelope, io))
                .await?
                .context("TELEGRAM_SEND_UNCONFIRMED")?,
            None,
        )
    } else {
        return Ok(());
    };
    if projected
        .references
        .values()
        .flatten()
        .any(|r| target.message_id(r).ok() == Some(document.message_id)
            && r.document_id()==Some(document.document_id)
            && r.bytes==entry.envelope.bytes && r.sha256==entry.envelope.sha256 && r.container==entry.envelope.container)
    {
        return Ok(());
    }
    document.check(&entry.envelope)?;
    let reference = MediaRef {
        scope: target.scope.clone(),
        location: MediaLocation::TelegramMessage {
            account_id: target.account_id.clone(),
            channel_id: target.channel_id.clone(),
            message_id: document.message_id as i64,
            document_id: document.document_id,
        },
        bytes: entry.envelope.bytes,
        sha256: entry.envelope.sha256.clone(),
        container: entry.envelope.container.clone(),
    };
    if identity.is_none() {verify(transport,target,&reference,io).await?;}
    remove(transport, target, &reference, io).await
}

pub(super) fn metadata(
    target: &Target,
    reference: &MediaRef,
    document: &RemoteDocument,
) -> Result<Envelope> {
    if target.message_id(reference)? != document.message_id {
        bail!("BACKUP_SCOPE_MISMATCH");
    }
    let envelope: Envelope =
        serde_json::from_str(&document.caption).context("TELEGRAM_MEDIA_CHANGED")?;
    document.check(&envelope)?;
    if envelope.yingqu_media != 2
        || envelope.library_id != target.scope.library_id
        || envelope.bytes != reference.bytes
        || envelope.sha256 != reference.sha256
        || envelope.container != reference.container
        || !matches!(envelope.kind.as_str(), "video" | "cover" | "transcript")
        || matches!(&reference.location,MediaLocation::TelegramMessage{document_id:id,..} if *id!=document.document_id)
    {
        bail!("TELEGRAM_MEDIA_CHANGED");
    }
    Ok(envelope)
}
pub(super) async fn verify(
    transport: &impl Transport,
    target: &Target,
    reference: &MediaRef,
    io: &Io,
) -> Result<()> {
    for attempt in 0..2 {
        let result = async {
            let document = io
                .run(transport.inspect_reference(target,reference))
                .await?;
            metadata(target, reference, &document)?;
            let mut reader = io.run(transport.download(document)).await?;
            let mut hash = Sha256::new();
            let mut bytes = 0_u64;
            let mut buffer = vec![0; 65536];
            loop {
                let count = io
                    .run(async { reader.read(&mut buffer).await.map_err(anyhow::Error::from) })
                    .await?;
                if count == 0 {
                    break;
                }
                bytes += count as u64;
                if bytes > reference.bytes {
                    bail!("BACKUP_CHECKSUM_MISMATCH");
                }
                hash.update(&buffer[..count]);
                io.touch();
            }
            if bytes != reference.bytes || format!("{:x}", hash.finalize()) != reference.sha256 {
                bail!("BACKUP_CHECKSUM_MISMATCH");
            }
            Ok(())
        }
        .await;
        match result {
            Err(e) if attempt == 0 && e.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" => {
                continue
            }
            other => return other,
        }
    }
    unreachable!("bounded full verification always returns")
}
pub(super) fn candidate(
    target: &Target,
    asset: &str,
    deletion: u64,
    document: &RemoteDocument,
    known: Option<&MediaRef>,
) -> Result<Option<MediaRef>> {
    let envelope: Envelope =
        serde_json::from_str(&document.caption).context("TELEGRAM_MEDIA_CHANGED")?;
    if envelope.library_id != target.scope.library_id
        || envelope.asset_id != asset
        || !matches!(envelope.kind.as_str(), "video" | "cover" | "transcript")
    {
        return Ok(None);
    }
    if envelope
        .generation
        .is_some_and(|generation| generation >= deletion)
    {
        return Ok(None);
    }
    let reference = known.cloned().unwrap_or_else(|| MediaRef {
        scope: target.scope.clone(),
        location: MediaLocation::TelegramMessage {
            account_id: target.account_id.clone(),
            channel_id: target.channel_id.clone(),
            message_id: document.message_id as i64,
            document_id: document.document_id,
        },
        bytes: envelope.bytes,
        sha256: envelope.sha256.clone(),
        container: envelope.container.clone(),
    });
    metadata(target, &reference, document)?;
    Ok(Some(reference))
}
pub(super) async fn remove(
    transport: &impl Transport,
    target: &Target,
    reference: &MediaRef,
    io: &Io,
) -> Result<()> {
    if !transport.can_delete() {
        bail!("BACKUP_DELETE_UNSUPPORTED");
    }
    let document = match io
        .run(transport.inspect_reference(target,reference))
        .await
    {
        Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => return Ok(()),
        other => other?,
    };
    metadata(target, reference, &document)?;
    io.check()?;
    io.run(transport.remove(document)).await
}
