//! Telegram target identity is separate from S3 configuration and credentials.
use super::{
    model::Space,
    provider::*,
    repository,
    spaces::{self, Config, TelegramConfig, Update, View},
    telegram_provider::TelegramProvider,
    telegram_transfer::Target,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

pub(super) fn remember_channel_title(core: &Core, target: &Target, title: &str) -> Result<()> {
    core.backup_db(|db| {
        db.execute("INSERT INTO telegram_space_labels(space_id,channel_title) SELECT t.space_id,?5 FROM telegram_spaces t JOIN backup_spaces s ON s.space_id=t.space_id WHERE t.space_id=?1 AND t.account_id=?2 AND t.channel_id=?3 AND s.library_id=?4 ON CONFLICT(space_id) DO UPDATE SET channel_title=excluded.channel_title WHERE channel_title<>excluded.channel_title", params![target.scope.space_id,target.account_id,target.channel_id,target.scope.library_id,title])?;
        Ok(())
    })
}

pub(super) fn binding(core: &Core, id: &str) -> Result<Option<(TelegramConfig, bool)>> {
    core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT account_id,channel_id,initializing FROM telegram_spaces WHERE space_id=?",
                [id],
                |r| {
                    Ok((
                        TelegramConfig {
                            account_id: r.get(0)?,
                            channel_id: r.get(1)?,
                        },
                        r.get(2)?,
                    ))
                },
            )
            .optional()?)
    })
}
fn target(space: &Space, binding: TelegramConfig) -> Target {
    Target {
        scope: Scope {
            space_id: space.space_id.clone(),
            library_id: space.library_id.clone(),
            provider_type: ProviderType::Telegram,
            protocol: 2,
        },
        account_id: binding.account_id,
        channel_id: binding.channel_id,
    }
}
pub(super) fn provider(
    core: &Arc<Core>,
    space: &Space,
    cfg: Config,
) -> Result<Arc<dyn BackupProvider>> {
    let binding = binding(core, &space.space_id)?
        .context("BACKUP_INVALID_SPACE")?
        .0;
    let target = target(space, binding);
    target.validate()?;
    Ok(Arc::new(TelegramProvider::new(
        core,
        target,
        cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?,
    )))
}
fn pending_caps() -> ProviderCapabilities {
    ProviderCapabilities {
        change_coordination: ChangeCoordination::EventReplay,
        incremental_discovery: false,
        range_read: false,
        resumable_upload: false,
        max_single_media_bytes: 0,
        can_delete_owned_media: false,
    }
}
pub(super) async fn connect(core: &Arc<Core>, mut update: Update, save: bool) -> Result<View> {
    let _guard = core.backup_config_lock.lock().await;
    if update.profile != "telegram"
        || update.credentials.is_some()
        || update.cleanup_deleted_records
        || !update.endpoint.is_empty()
        || !update.bucket.is_empty()
        || !update.prefix.is_empty()
    {
        bail!("BACKUP_INVALID_SPACE");
    }
    let binding = update.telegram.take().context("BACKUP_INVALID_SPACE")?;
    let name = update
        .display_name
        .as_deref()
        .map(spaces::normalized_name)
        .transpose()?;
    let old_id: Option<String> = core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT space_id FROM telegram_spaces WHERE account_id=?1 AND channel_id=?2",
                params![binding.account_id, binding.channel_id],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    let old = core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|s| Some(&s.space_id) == old_id.as_ref());
    let initializing = old_id
        .as_deref()
        .map(|id| self::binding(core, id))
        .transpose()?
        .flatten()
        .is_some_and(|(_, initializing)| initializing);
    let create = old.is_none() && update.create_library || initializing;
    let mut space = old.clone().unwrap_or_else(|| Space {
        space_id: uuid::Uuid::new_v4().to_string(),
        library_id: uuid::Uuid::new_v4().to_string(),
        endpoint: String::new(),
        bucket: String::new(),
        prefix: String::new(),

        credential_ref: None,
        enabled: false,
    });
    let candidate = TelegramProvider::new(core, target(&space, binding.clone()), pending_caps());
    candidate.scope().provider_type.require_implemented()?;
    let probe_target = target(&space, binding.clone());
    probe_target.validate()?;
    let (found, caps, channel_title) = candidate
        .request("probe_space", None, move |_, transport, io| {
            Box::pin(async move {
                Ok((
                    TelegramProvider::discover(&probe_target, transport, io).await?,
                    TelegramProvider::caps(transport),
                    transport.channel_title().to_owned(),
                ))
            })
        })
        .await?;
    if let Some(id) = found {
        if old.as_ref().is_some_and(|s| s.library_id != id) {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        space.library_id = id;
    } else if !save || !create {
        bail!("BACKUP_LIBRARY_NOT_FOUND");
    }
    if !save {
        return Ok(View {
            telegram: Some(binding),
            telegram_channel_title: Some(channel_title),
            can_delete_owned_media: caps.can_delete_owned_media,
            display_name: name.unwrap_or_default(),
            space_id: space.space_id,
            library_id: space.library_id,
            provider_type: ProviderType::Telegram,
            profile: "telegram".into(),
            endpoint: String::new(),
            bucket: String::new(),
            prefix: String::new(),
            region: "auto".into(),
            path_style: true,
            enabled: false,
            credentials_configured: false,
            protocol: 2,
            last_error: None,
            connection_pending: false,
            cleanup_deleted_records: false,
            deleted_record_retention_days: 30,
        });
    }
    // Reserve stable library/operation identity before the first remotely visible descriptor.
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        if old.is_none() {repository::save_space(&tx,&space)?;}
        tx.execute("INSERT INTO telegram_spaces VALUES(?1,?2,?3,?4) ON CONFLICT(space_id) DO UPDATE SET initializing=excluded.initializing",params![space.space_id,binding.account_id,binding.channel_id,create])?;
        tx.execute("INSERT INTO telegram_space_labels(space_id,channel_title) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET channel_title=excluded.channel_title",params![space.space_id,channel_title])?;
        tx.execute("UPDATE backup_provider_config SET provider_type='telegram',profile='telegram',protocol=2 WHERE space_id=?",[&space.space_id])?;
        if let Some(name)=&name {spaces::write_name(&tx,&space.space_id,name)?;}
        tx.commit()?;Ok(())
    })?;
    let provider = TelegramProvider::new(core, target(&space, binding), caps.clone());
    if let Err(error) = provider.open_library(create).await {
        core.backup_db(|db|{db.execute("INSERT INTO backup_sync_checkpoints(space_id,last_error) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET last_error=excluded.last_error",params![space.space_id,error.to_string()])?;Ok(())})?;
        return Err(error);
    }
    let enabled = spaces::enabled(core)?;
    core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let current:Option<String>=tx.query_row("SELECT space_id FROM backup_current_space WHERE id=1",[],|r|r.get(0)).optional()?;
        let select=update.activate || current.as_deref()==Some(&space.space_id);
        space.enabled=select && if update.activate {update.enabled} else {enabled};
        repository::save_space(&tx,&space)?;
        tx.execute("UPDATE backup_provider_config SET provider_type='telegram',profile='telegram',protocol=2,capabilities_json=?2 WHERE space_id=?1",params![space.space_id,serde_json::to_string(&caps)?])?;
        tx.execute("UPDATE telegram_spaces SET initializing=0 WHERE space_id=?",[&space.space_id])?;
        tx.execute("DELETE FROM backup_removed_spaces WHERE space_id=?",[&space.space_id])?;
        tx.execute("UPDATE backup_sync_checkpoints SET last_error=NULL WHERE space_id=?",[&space.space_id])?;
        if update.activate {tx.execute("INSERT INTO backup_sync_preference VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",[update.enabled])?;}
        if select {spaces::select_in_transaction(&tx,&space.space_id,space.enabled)?;}
        else if spaces::select_initial_default(&tx)?.as_deref()==Some(&space.space_id) {
            spaces::select_in_transaction(&tx,&space.space_id,enabled)?;
        }
        tx.commit()?;Ok(())
    })?;
    core.wake_backup();
    spaces::list(core)?
        .into_iter()
        .find(|s| s.space_id == space.space_id)
        .context("BACKUP_SPACE_NOT_FOUND")
}
pub(super) async fn prepare(core: &Arc<Core>, id: &str) -> Result<()> {
    let (binding, initializing) = binding(core, id)?.context("BACKUP_INVALID_SPACE")?;
    connect(
        core,
        Update {
            telegram: Some(binding),
            create_library: initializing,
            display_name: None,
            provider_type: ProviderType::Telegram,
            profile: "telegram".into(),
            endpoint: String::new(),
            bucket: String::new(),
            prefix: String::new(),
            region: "auto".into(),
            path_style: true,
            enabled: true,
            activate: false,
            credentials: None,
            cleanup_deleted_records: false,
            deleted_record_retention_days: 30,
        },
        true,
    )
    .await?;
    Ok(())
}
