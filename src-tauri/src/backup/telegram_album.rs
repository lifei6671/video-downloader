//! Independent original documents, using the existing send journal for each member.
use super::{
    album,
    provider::*,
    telegram_transfer::{self, Io, Target, Transport},
    telegram_transfer_journal::Journal,
};
use crate::files::AlbumFile;
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub(super) fn member_operation(operation: &str, name: &str) -> String {
    let digest = Sha256::digest(format!("yingqu-album:{operation}:{name}"));
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub(super) async fn upload(
    core: &crate::desktop_core::Core,
    transport: &impl Transport,
    journal: &impl Journal,
    target: &Target,
    source: MediaSource,
    files: Vec<AlbumFile>,
    io: &Io,
) -> Result<UploadReceipt> {
    album::validate_manifest(&files)?;
    io.check()?;
    if let Some(control) = &source.control {
        (control.checkpoint)(ResumeState {
            scope: target.scope.clone(),
            value:
                serde_json::json!({"operation_id":source.operation_id,"asset_id":source.asset_id,
                "device_id":source.device_id,"kind":"album","files":files})
                .to_string(),
        })?;
    }
    let mut members = Vec::new();
    let mut completed = 0;
    let mut message = None;
    for (name, bytes, hash) in album::entries(&files) {
        io.check()?;
        let progress = Arc::new(AtomicU64::new(0));
        let mut child = source.clone();
        child.path = source.path.join(name);
        child.kind = "album_file".into();
        child.video = None;
        child.bytes = bytes;
        child.sha256 = hash.into();
        child.container = name
            .rsplit_once('.')
            .context("BACKUP_INVALID_ALBUM")?
            .1
            .into();
        child.operation_id = member_operation(&source.operation_id, name);
        child.control = source.control.as_ref().map(|parent| {
            let checker = parent.clone();
            let checkpoint = parent.clone();
            Arc::new(TransferControl {
                check: Box::new(move || (checker.check)()),
                // The per-file nonce and send state are already durable in the journal.
                checkpoint: Box::new(move |_| (checkpoint.check)()),
                progress: progress.clone(),
            })
        });
        let upload = telegram_transfer::upload_album_member(
            transport, journal, target, child, io, message, core,
        );
        tokio::pin!(upload);
        let mut timer = tokio::time::interval(std::time::Duration::from_millis(100));
        let receipt = loop {
            tokio::select! {
                result = &mut upload => break result?,
                _ = timer.tick() => {
                    io.check()?;
                    if let Some(parent) = &source.control {
                        parent.progress.store(completed + progress.load(Ordering::Relaxed).min(bytes), Ordering::Relaxed);
                    }
                }
            }
        };
        ensure!(
            receipt.reference.bytes == bytes && receipt.reference.sha256 == hash,
            "BACKUP_SOURCE_CHANGED"
        );
        let sent_message = target.message_id(&receipt.reference)?;
        ensure!(
            message.is_none_or(|id| id == sent_message),
            "TELEGRAM_MEDIA_CHANGED"
        );
        message = Some(sent_message);
        members.push(AlbumMemberRef {
            filename: name.into(),
            reference: receipt.reference,
        });
        completed += bytes;
    }
    let reference = MediaRef {
        scope: target.scope.clone(),
        location: MediaLocation::TelegramAlbum { files: members },
        bytes: source.bytes,
        sha256: source.sha256,
        container: "album".into(),
    };
    album::validate_reference(&reference, &files)?;
    if let Some(parent) = &source.control {
        parent.progress.store(source.bytes, Ordering::Relaxed);
    }
    Ok(UploadReceipt {
        reference,
        operation_id: source.operation_id,
        device_id: source.device_id,
    })
}
