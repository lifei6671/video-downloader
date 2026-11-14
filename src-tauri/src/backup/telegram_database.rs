//! Database snapshots use standalone encrypted Telegram documents.
use super::{
    database::{password_identity, Snapshot},
    model::Space,
    provider::{MediaLocation, MediaRef, MediaSource, TransferControl},
    spaces::TelegramConfig,
    telegram_provider::TelegramProvider,
    telegram_spaces,
    telegram_transfer::{self, Envelope, Target, Transport},
    telegram_transfer_journal::CoreJournal,
};
use crate::desktop_core::Core;
use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) fn matches(target: &Target, recipient: &str, envelope: &Envelope) -> bool {
    envelope.kind == "database"
        && envelope.has_supported_version()
        && envelope.library_id == target.scope.library_id
        && envelope.asset_id == recipient
        && envelope.container == "age"
}

pub(super) struct SnapshotFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

pub(super) async fn publish(
    provider: &TelegramProvider,
    recipient: String,
    device: String,
    file: SnapshotFile,
    control: Arc<TransferControl>,
    cancel: watch::Receiver<bool>,
) -> Result<()> {
    let target = provider.target().clone();
    let operation_id = Uuid::new_v4().to_string();
    let source = MediaSource {
        asset_id: recipient.clone(),
        kind: "database".into(),
        path: file.path,
        bytes: file.bytes,
        sha256: file.sha256,
        container: "age".into(),
        operation_id,
        device_id: device.clone(),
        video: None,
        base_revision: None,
        recreate_revision: None,
        control: Some(control.clone()),
    };
    let result = provider.request_with_cancel("upload_media", Some(control), cancel, move |core, transport, io| Box::pin(async move {
        let receipt = telegram_transfer::upload(transport, &CoreJournal(core), &target, source, io).await?;
        let recent_id = target.message_id(&receipt.reference)?;
        super::database::complete(core, &target.scope.space_id, Some(recent_id), file.bytes)?;
        if transport.can_delete() {
            let prune = async {
                let mut documents = transport.database_documents(&target, &recipient, io).await?;
                documents.sort_unstable_by_key(|(doc, _, _)| std::cmp::Reverse(doc.message_id));
                for (doc, _, _) in documents.into_iter().filter(|(doc, envelope, _)| {
                    doc.message_id != recent_id && envelope.device_id == device
                }).skip(2) {
                    io.run(transport.remove(doc)).await?;
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            if let Err(error) = prune {
                eprintln!("Database snapshot retention: {error}");
            }
        }
        if let Err(error) = core.backup_db(|db| {
            db.execute("DELETE FROM telegram_extra_sends WHERE kind='database' AND json_extract(entry_json,'$.target.scope.space_id')=? AND json_extract(entry_json,'$.state')='confirmed'", [&target.scope.space_id])?;
            Ok(())
        }) {
            eprintln!("Database snapshot send journal: {error}");
        }
        Ok(())
    })).await;
    result.map_err(|error| {
        if let Some(rpc) = error.downcast_ref::<super::telegram_session::RejectedRpc>() {
            anyhow::anyhow!(
                "DATABASE_BACKUP_TELEGRAM_REJECTED:{}:{}",
                rpc.code,
                rpc.name
            )
        } else {
            error
        }
    })
}

pub(super) struct Selected {
    pub space: Space,
    pub config: super::spaces::Config,
    pub binding: TelegramConfig,
    pub snapshot: Snapshot,
    pub message_id: i32,
    pub key: age::x25519::Identity,
}

fn message_id(channel_id: &str, link: &str) -> Result<i32> {
    let link = crate::telegram_download::message_link(link)
        .context("DATABASE_BACKUP_INVALID_MESSAGE_LINK")?;
    let channel = link
        .channel
        .context("DATABASE_BACKUP_INVALID_MESSAGE_LINK")?;
    ensure!(
        channel.to_string() == channel_id,
        "DATABASE_BACKUP_WRONG_CHANNEL"
    );
    Ok(i32::try_from(link.message)?)
}

pub(super) async fn selected(
    core: &Arc<Core>,
    binding: TelegramConfig,
    password: Zeroizing<String>,
    link: &str,
    output: Option<&Path>,
    max_bytes: u64,
) -> Result<Selected> {
    let message_id = message_id(&binding.channel_id, link)?;
    // Old channels can have a random library ID. Read it from the chosen document.
    let mut space = Space {
        space_id: Uuid::new_v4().to_string(),
        library_id: super::library::target_id(&[
            "telegram",
            &binding.account_id,
            &binding.channel_id,
        ]),
        endpoint: String::new(),
        bucket: String::new(),
        prefix: String::new(),
        credential_ref: None,
        enabled: false,
    };
    let mut target = telegram_spaces::target(&space, binding.clone());
    let provider = TelegramProvider::new(core, target.clone(), telegram_spaces::pending_caps());
    let output = output.map(Path::to_owned);
    let (library_id, snapshot, capabilities, key) = provider
        .request("database_message", None, move |core, transport, io| {
            Box::pin(async move {
                let (doc, created_at) = transport.database_message(message_id, io).await?;
                let envelope = serde_json::from_str::<Envelope>(&doc.caption)
                    .context("DATABASE_BACKUP_INVALID_SNAPSHOT")?;
                Uuid::parse_str(&envelope.library_id)
                    .context("DATABASE_BACKUP_INVALID_SNAPSHOT")?;
                target.scope.library_id.clone_from(&envelope.library_id);
                ensure!(
                    envelope.kind == "database",
                    "DATABASE_BACKUP_INVALID_SNAPSHOT"
                );
                let key = password_identity(password, envelope.library_id.clone()).await?;
                let recipient = key.to_public().to_string();
                ensure!(
                    envelope.asset_id == recipient,
                    "DATABASE_BACKUP_DECRYPT_FAILED"
                );
                ensure!(
                    matches(&target, &recipient, &envelope),
                    "DATABASE_BACKUP_INVALID_SNAPSHOT"
                );
                doc.check(&envelope)?;
                ensure!(doc.bytes <= max_bytes, "DATABASE_BACKUP_TOO_LARGE");
                if let Some(output) = output {
                    let reference = MediaRef {
                        scope: target.scope.clone(),
                        location: MediaLocation::TelegramMessage {
                            account_id: target.account_id.clone(),
                            channel_id: target.channel_id.clone(),
                            message_id: i64::from(message_id),
                            document_id: doc.document_id,
                        },
                        bytes: doc.bytes,
                        sha256: envelope.sha256,
                        container: envelope.container,
                    };
                    telegram_transfer::restore(
                        transport,
                        &CoreJournal(core),
                        &target,
                        &reference,
                        &output,
                        io,
                    )
                    .await?;
                }
                let snapshot = Snapshot {
                    id: message_id.to_string(),
                    created_at,
                    bytes: doc.bytes,
                };
                Ok((
                    target.scope.library_id,
                    snapshot,
                    TelegramProvider::caps(transport),
                    key,
                ))
            })
        })
        .await?;
    space.library_id = library_id;
    Ok(Selected {
        space,
        binding,
        snapshot,
        message_id,
        key,
        config: super::spaces::Config {
            provider_type: super::provider::ProviderType::Telegram,
            profile: "telegram".into(),
            region: "auto".into(),
            path_style: true,
            protocol: 2,
            capabilities: Some(capabilities),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::message_id;

    #[test]
    fn telegram_database_snapshot_link_must_name_the_selected_private_channel() {
        assert_eq!(message_id("100", "https://t.me/c/100/12").unwrap(), 12);
        assert_eq!(
            message_id("101", "https://t.me/c/100/12")
                .unwrap_err()
                .to_string(),
            "DATABASE_BACKUP_WRONG_CHANNEL"
        );
        for link in [
            "https://t.me/public/12",
            "https://t.me/c/100/12/13",
            "https://example.com/c/100/12",
            "https://t.me/c/100/2147483648",
        ] {
            assert_eq!(
                message_id("100", link).unwrap_err().to_string(),
                "DATABASE_BACKUP_INVALID_MESSAGE_LINK"
            );
        }
    }
}
