//! Local account choices for Telegram downloads and backups. Default choices share one session.
use crate::{
    backup::telegram_session::{Phase, ProxySettings, TelegramSession, View},
    desktop_core::Core,
};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Download,
    #[default]
    Backup,
}
impl Purpose {
    fn index(self) -> usize {
        match self {
            Self::Download => 0,
            Self::Backup => 1,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Backup => "backup",
        }
    }
    fn other(self) -> Self {
        match self {
            Self::Download => Self::Backup,
            Self::Backup => Self::Download,
        }
    }
}

#[derive(Clone, Copy)]
pub enum LoginMode {
    Reuse,
    OtherAccount,
}

#[derive(Clone)]
pub struct Binding {
    pub namespace: String,
    pub session: Arc<TelegramSession>,
}
impl Binding {
    fn new(namespace: String, proxy: ProxySettings) -> Self {
        Self {
            namespace,
            session: Arc::new(TelegramSession::with_proxy(proxy)),
        }
    }
}
struct Choices {
    active: [Binding; 2],
    pending: [Option<Binding>; 2],
    closed: bool,
    network_changing: bool,
}
pub(crate) struct NetworkSessions<'a> {
    accounts: &'a TelegramAccounts,
    pub sessions: Vec<Arc<TelegramSession>>,
}
impl Drop for NetworkSessions<'_> {
    fn drop(&mut self) { self.accounts.choices.lock().unwrap().network_changing = false; }
}
pub struct TelegramAccounts {
    proxy: ProxySettings,
    choices: Mutex<Choices>,
}
impl TelegramAccounts {
    pub fn load(db: &Connection) -> Result<Self> {
        let default = crate::backup::repository::device_id(db)?;
        let load = |purpose: Purpose| -> Result<String> {
            let value: Option<String> = db
                .query_row(
                    "SELECT session_ref FROM telegram_account_bindings WHERE purpose=?",
                    [purpose.key()],
                    |row| row.get(0),
                )
                .optional()?;
            let value = value.unwrap_or_else(|| default.clone());
            uuid::Uuid::parse_str(&value)?;
            Ok(value)
        };
        let proxy = Arc::new(Mutex::new(None));
        let download = Binding::new(load(Purpose::Download)?, proxy.clone());
        let namespace = load(Purpose::Backup)?;
        let backup = if namespace == download.namespace {
            download.clone()
        } else {
            Binding::new(namespace, proxy.clone())
        };
        Ok(Self {
            proxy,
            choices: Mutex::new(Choices {
                active: [download, backup],
                pending: [None, None],
                closed: false,
                network_changing: false,
            }),
        })
    }
    pub fn configure_proxy(&self, profile: Option<crate::settings::Profile>) {
        *self.proxy.lock().unwrap() = profile;
    }
    pub(crate) fn network_sessions(&self) -> Result<NetworkSessions<'_>> {
        let mut choices = self.choices.lock().unwrap();
        if choices.closed { bail!("TELEGRAM_CLOSED"); }
        if choices.network_changing { bail!("TELEGRAM_BUSY"); }
        choices.network_changing = true;
        let mut sessions = vec![];
        for binding in choices.active.iter().chain(choices.pending.iter().flatten()) {
            if !sessions.iter().any(|session| Arc::ptr_eq(session, &binding.session)) {
                sessions.push(binding.session.clone());
            }
        }
        Ok(NetworkSessions { accounts: self, sessions })
    }
    pub fn binding(&self, purpose: Purpose) -> Binding {
        self.choices.lock().unwrap().active[purpose.index()].clone()
    }
    fn displayed(&self, purpose: Purpose) -> Binding {
        let choices = self.choices.lock().unwrap();
        choices.pending[purpose.index()]
            .as_ref()
            .unwrap_or(&choices.active[purpose.index()])
            .clone()
    }
    pub fn status(&self, purpose: Purpose) -> View {
        let choices = self.choices.lock().unwrap();
        if let Some(pending) = &choices.pending[purpose.index()] {
            let mut view = pending.session.status();
            // A newly authorized account is not selected until its local binding commits.
            if matches!(view.phase, Phase::Ready) {
                view.phase = Phase::Connecting;
                view.account = None;
            }
            view
        } else {
            choices.active[purpose.index()].session.status()
        }
    }
    fn save(core: &Arc<Core>, purpose: Purpose, binding: &Binding) -> Result<()> {
        core.backup_db(|db| { db.execute("INSERT INTO telegram_account_bindings VALUES(?1,?2) ON CONFLICT(purpose) DO UPDATE SET session_ref=excluded.session_ref", params![purpose.key(), binding.namespace])?; Ok(()) })
    }
    // Commit a replacement only after authorization succeeds; old choices survive QR cancel/failure.
    async fn finish(
        &self,
        core: &Arc<Core>,
        purpose: Purpose,
        binding: &Binding,
        result: Result<View>,
    ) -> Result<View> {
        let view = match result {
            Ok(view) => view,
            Err(error) => {
                if matches!(binding.session.status().phase, Phase::SignedOut | Phase::Disconnected) {
                    let pending = {
                        let mut choices = self.choices.lock().unwrap();
                        if choices.pending[purpose.index()]
                            .as_ref()
                            .is_some_and(|pending| Arc::ptr_eq(&pending.session, &binding.session))
                        {
                            choices.pending[purpose.index()].take()
                        } else {
                            None
                        }
                    };
                    if let Some(pending) = pending {
                        pending.session.shutdown().await;
                    }
                }
                return Err(error);
            }
        };
        if matches!(view.phase, Phase::Ready) {
            let is_pending = self.choices.lock().unwrap().pending[purpose.index()]
                .as_ref()
                .is_some_and(|pending| Arc::ptr_eq(&pending.session, &binding.session));
            if is_pending {
                let current = binding.session.status();
                if !matches!(current.phase, Phase::Ready)
                    || current.operation_id != view.operation_id
                {
                    let stale = self.choices.lock().unwrap().pending[purpose.index()].take();
                    if let Some(stale) = stale {
                        stale.session.shutdown().await;
                    }
                    bail!("TELEGRAM_CANCELED");
                }
                Self::stop_current(core, purpose, &self.binding(purpose)).await?;
                let (retired, failed) = {
                    let mut choices = self.choices.lock().unwrap();
                    if choices.closed {
                        bail!("TELEGRAM_CLOSED");
                    }
                    let current = binding.session.status();
                    if !matches!(current.phase, Phase::Ready)
                        || current.operation_id != view.operation_id
                        || !choices.pending[purpose.index()]
                            .as_ref()
                            .is_some_and(|pending| Arc::ptr_eq(&pending.session, &binding.session))
                    {
                        bail!("TELEGRAM_CANCELED");
                    }
                    match Self::save(core, purpose, binding) {
                        Ok(()) => {
                            // A media read may have entered after stop_current drained the old binding.
                            if matches!(purpose, Purpose::Backup) {
                                choices.active[purpose.index()].session.invalidate_backup_reads();
                            }
                            let old = std::mem::replace(
                                &mut choices.active[purpose.index()],
                                binding.clone(),
                            );
                            choices.pending[purpose.index()] = None;
                            (
                                (!Arc::ptr_eq(
                                    &old.session,
                                    &choices.active[purpose.other().index()].session,
                                ))
                                .then_some(old),
                                None,
                            )
                        }
                        Err(error) => (choices.pending[purpose.index()].take(), Some(error)),
                    }
                };
                if let Some(old) = retired {
                    old.session.shutdown().await;
                }
                if let Some(error) = failed {
                    return Err(error);
                }
            }
        }
        Ok(self.status(purpose))
    }
    async fn stop_current(core: &Arc<Core>, purpose: Purpose, binding: &Binding) -> Result<()> {
        match purpose {
            Purpose::Download => {
                // Close historical recovery admission before taking the cancellation snapshot.
                core.stop_telegram_recovery();
                let mut tasks: Vec<_> = core
                    .list()
                    .into_iter()
                    .filter(|task| {
                        (task.telegram_link.is_some()
                            || task
                                .video
                                .as_ref()
                                .is_some_and(|v| v["platform"] == "telegram"))
                            && matches!(
                                task.status,
                                crate::desktop_core::Status::Parsing
                                    | crate::desktop_core::Status::Queued
                                    | crate::desktop_core::Status::Downloading
                            )
                    })
                    .collect();
                // Remove queued work before cancelling an active task frees a download slot.
                tasks.sort_by_key(|task| {
                    !matches!(task.status, crate::desktop_core::Status::Queued)
                });
                for task in tasks {
                    core.cancel(&task.id).await?;
                }
            }
            Purpose::Backup => binding.session.cancel_backup_transfer().await,
        }
        Ok(())
    }
    pub async fn resume(&self, core: &Arc<Core>, purpose: Purpose) -> Result<View> {
        let binding = self.displayed(purpose);
        let view = binding.session.resume(binding.namespace.clone()).await?;
        if matches!(view.phase, Phase::SignedOut) {
            let other = self.binding(purpose.other());
            if !Arc::ptr_eq(&binding.session, &other.session) {
                let other_view = other.session.resume(other.namespace.clone()).await?;
                if matches!(other_view.phase, Phase::Ready) {
                    let adopted = self.adopt(core, purpose, &binding, &other)?;
                    if adopted {
                        binding.session.shutdown().await;
                        return Ok(other_view);
                    }
                    return Ok(self.status(purpose));
                }
            }
        }
        self.finish(core, purpose, &binding, Ok(view)).await
    }
    /// Called by the desktop's background startup task, never from Core::open.
    /// Shared bindings restore once; separate account choices keep their own identity.
    pub async fn restore_saved(&self) {
        let bindings = {
            let choices = self.choices.lock().unwrap();
            if choices.closed { return; }
            let mut bindings = vec![choices.active[0].clone()];
            if !Arc::ptr_eq(&choices.active[0].session, &choices.active[1].session) {
                bindings.push(choices.active[1].clone());
            }
            bindings
        };
        for binding in bindings {
            let view = binding.session.status();
            if view.configured && matches!(view.phase, Phase::Disconnected) {
                // connect/finish retain visible errors; network failure never opens a QR flow.
                let _ = binding.session.resume(binding.namespace).await;
            }
        }
    }
    fn adopt(
        &self,
        core: &Arc<Core>,
        purpose: Purpose,
        original: &Binding,
        other: &Binding,
    ) -> Result<bool> {
        let mut choices = self.choices.lock().unwrap();
        if choices.closed {
            bail!("TELEGRAM_CLOSED");
        }
        if choices.pending[purpose.index()].is_some()
            || !Arc::ptr_eq(&choices.active[purpose.index()].session, &original.session)
            || !Arc::ptr_eq(
                &choices.active[purpose.other().index()].session,
                &other.session,
            )
        {
            return Ok(false);
        }
        Self::save(core, purpose, other)?;
        choices.active[purpose.index()] = other.clone();
        Ok(true)
    }
    fn replacement(&self, purpose: Purpose, original: Option<&Binding>) -> Result<Binding> {
        let mut choices = self.choices.lock().unwrap();
        if choices.closed {
            bail!("TELEGRAM_CLOSED");
        }
        if choices.network_changing { bail!("TELEGRAM_BUSY"); }
        if choices.pending[purpose.index()].is_some() {
            bail!("TELEGRAM_BUSY");
        }
        if original
            .is_some_and(|old| !Arc::ptr_eq(&old.session, &choices.active[purpose.index()].session))
        {
            bail!("TELEGRAM_STALE_OPERATION");
        }
        let binding = Binding::new(uuid::Uuid::new_v4().to_string(), self.proxy.clone());
        choices.pending[purpose.index()] = Some(binding.clone());
        Ok(binding)
    }
    pub async fn login(&self, core: &Arc<Core>, purpose: Purpose, mode: LoginMode) -> Result<View> {
        let binding = if matches!(mode, LoginMode::OtherAccount) {
            self.replacement(purpose, None)?
        } else {
            if matches!(self.resume(core, purpose).await?.phase, Phase::Ready) {
                return Ok(self.status(purpose));
            }
            self.displayed(purpose)
        };
        let result = binding.session.login_qr(binding.namespace.clone()).await;
        if let Some(fresh) =
            self.replacement_after_expiry(purpose, mode, &binding, result.as_ref().err())?
        {
            let result = fresh.session.login_qr(fresh.namespace.clone()).await;
            return self.finish(core, purpose, &fresh, result).await;
        }
        self.finish(core, purpose, &binding, result).await
    }
    fn replacement_after_expiry(
        &self,
        purpose: Purpose,
        mode: LoginMode,
        binding: &Binding,
        error: Option<&anyhow::Error>,
    ) -> Result<Option<Binding>> {
        if !matches!(mode, LoginMode::Reuse)
            || !error.is_some_and(|e| e.to_string() == "TELEGRAM_RELOGIN_REQUIRED")
        {
            return Ok(None);
        }
        let id = binding
            .session
            .status()
            .operation_id
            .ok_or_else(|| anyhow::anyhow!("TELEGRAM_CANCELED"))?;
        if !binding.session.has_operation(&id) {
            bail!("TELEGRAM_CANCELED");
        }
        Ok(Some(self.replacement(purpose, Some(binding))?))
    }
    pub async fn password(
        &self,
        core: &Arc<Core>,
        purpose: Purpose,
        id: &str,
        password: String,
    ) -> Result<View> {
        let binding = self.displayed(purpose);
        let result = binding.session.submit_password(id, password).await;
        self.finish(core, purpose, &binding, result).await
    }
    pub async fn cancel(&self, purpose: Purpose, id: &str) -> Result<View> {
        let (binding, pending) = {
            let mut choices = self.choices.lock().unwrap();
            if choices.closed {
                bail!("TELEGRAM_CLOSED");
            }
            let binding = choices.pending[purpose.index()]
                .as_ref()
                .unwrap_or(&choices.active[purpose.index()])
                .clone();
            let view = binding.session.status();
            if view.operation_id.as_deref() != Some(id) {
                bail!("TELEGRAM_STALE_OPERATION");
            }
            // Closing a QR dialog can arrive after its authorization has already committed.
            if choices.pending[purpose.index()].is_none() && matches!(view.phase, Phase::Ready) {
                return Ok(view);
            }
            let pending = choices.pending[purpose.index()].take();
            (binding, pending)
        };
        let result = binding.session.cancel(id).await;
        if let Some(pending) = pending {
            pending.session.shutdown().await;
        }
        result?;
        Ok(self.status(purpose))
    }
    pub async fn logout(&self, core: &Arc<Core>, purpose: Purpose, id: &str) -> Result<View> {
        let binding = self.binding(purpose);
        if binding.session.status().operation_id.as_deref() != Some(id) {
            bail!("TELEGRAM_STALE_OPERATION");
        }
        let detached = {
            let mut choices = self.choices.lock().unwrap();
            if choices.closed {
                bail!("TELEGRAM_CLOSED");
            }
            if choices.network_changing { bail!("TELEGRAM_BUSY"); }
            if choices.pending[purpose.index()].is_some() {
                bail!("TELEGRAM_BUSY");
            }
            if Arc::ptr_eq(
                &binding.session,
                &choices.active[purpose.other().index()].session,
            ) {
                let signed_out = Binding::new(uuid::Uuid::new_v4().to_string(), self.proxy.clone());
                signed_out.session.detached_signed_out();
                Self::save(core, purpose, &signed_out)?;
                choices.active[purpose.index()] = signed_out;
                true
            } else {
                false
            }
        };
        Self::stop_current(core, purpose, &binding).await?;
        if detached {
            return Ok(self.status(purpose));
        }
        binding.session.logout(id).await
    }
    pub async fn shutdown(&self) {
        let bindings = {
            let mut choices = self.choices.lock().unwrap();
            choices.closed = true;
            choices
                .active
                .iter()
                .chain(choices.pending.iter().flatten())
                .cloned()
                .collect::<Vec<_>>()
        };
        for binding in bindings {
            binding.session.shutdown().await;
        }
    }
}

/// Keep the original binding and stop waiting when its purpose is switched.
pub(crate) async fn recover_network(
    core: &Core, purpose: Purpose, cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let binding = core.telegram_accounts.binding(purpose);
    let work = binding.session.recover_network(&binding.namespace, cancel);
    tokio::pin!(work);
    loop {
        tokio::select! { biased;
            result = &mut work => {
                let current = core.telegram_accounts.binding(purpose);
                if !Arc::ptr_eq(&binding.session, &current.session) { bail!("TELEGRAM_ACCOUNT_MISMATCH"); }
                return result;
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                let current = core.telegram_accounts.binding(purpose);
                if !Arc::ptr_eq(&binding.session, &current.session) { bail!("TELEGRAM_ACCOUNT_MISMATCH"); }
                if matches!(binding.session.status().phase, Phase::SignedOut) { bail!("TELEGRAM_AUTH_REQUIRED"); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn proxy_transition_blocks_new_accounts_until_its_snapshot_is_released() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(&root.path().join("qa.sqlite"), root.path().join("downloads"),
            "unused-engine".into(), vec![], |_| {}).unwrap();
        let accounts = &core.telegram_accounts;
        let binding = accounts.binding(Purpose::Download);
        binding.session.set_test_view(Phase::Ready, "123");
        let login = binding.session.status().operation_id.unwrap();
        let network = accounts.network_sessions().unwrap();
        assert_eq!(network.sessions.len(), 1, "shared account is paused once");
        assert_eq!(accounts.login(&core, Purpose::Download, LoginMode::OtherAccount).await.err().unwrap().to_string(), "TELEGRAM_BUSY");
        assert_eq!(accounts.logout(&core, Purpose::Download, &login).await.err().unwrap().to_string(), "TELEGRAM_BUSY");
        let update = |proxy| crate::settings::ProfileUpdate {
            platform: "telegram".into(), user_agent: "test".into(), cookie: None,
            proxy, http_chunk_size: None,
        };
        assert_eq!(core.save_profile_with_network(update(Some(String::new()))).await.err().unwrap().to_string(), "TELEGRAM_BUSY",
                   "even an unchanged proxy must serialize comparison with persistence");
        core.save_profile_with_network(update(None)).await.unwrap();
        assert!(accounts.choices.lock().unwrap().pending[0].is_none());
        drop(network);
        accounts.replacement(Purpose::Download, None).unwrap();
        core.shutdown().await;
    }
    fn core(path: &std::path::Path) -> Arc<Core> {
        Core::open(
            &path.join("qa.sqlite"),
            path.join("downloads"),
            "unused-engine".into(),
            vec![],
            |_| {},
        )
        .unwrap()
    }
    fn pending(accounts: &TelegramAccounts, purpose: Purpose, phase: Phase) -> Binding {
        let next = Binding::new(uuid::Uuid::new_v4().to_string(), accounts.proxy.clone());
        next.session.set_test_view(phase, "account-b");
        accounts.choices.lock().unwrap().pending[purpose.index()] = Some(next.clone());
        next
    }
    #[tokio::test]
    async fn late_dialog_cancel_keeps_committed_and_shared_accounts_ready() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let shared = app.telegram_accounts.binding(Purpose::Backup);
        shared.session.set_test_view(Phase::Ready, "account-a");
        let view = shared.session.status();
        for purpose in [Purpose::Download, Purpose::Backup] {
            let result = app
                .telegram_accounts
                .cancel(purpose, view.operation_id.as_deref().unwrap())
                .await
                .unwrap();
            assert!(matches!(result.phase, Phase::Ready));
            assert!(shared
                .session
                .has_operation(view.operation_id.as_deref().unwrap()));
        }
        let next = pending(&app.telegram_accounts, Purpose::Download, Phase::Ready);
        app.telegram_accounts
            .finish(&app, Purpose::Download, &next, Ok(next.session.status()))
            .await
            .unwrap();
        let result = app
            .telegram_accounts
            .cancel(
                Purpose::Download,
                next.session.status().operation_id.as_deref().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(result.account.unwrap().id, "account-b");
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Backup)
                .account
                .unwrap()
                .id,
            "account-a"
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn stale_resume_cannot_replace_a_newer_account_choice() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let original = app.telegram_accounts.binding(Purpose::Download);
        let other = app.telegram_accounts.binding(Purpose::Backup);
        let selected = pending(&app.telegram_accounts, Purpose::Download, Phase::Ready);
        app.telegram_accounts
            .finish(
                &app,
                Purpose::Download,
                &selected,
                Ok(selected.session.status()),
            )
            .await
            .unwrap();
        assert!(!app
            .telegram_accounts
            .adopt(&app, Purpose::Download, &original, &other)
            .unwrap());
        assert_eq!(
            app.telegram_accounts.binding(Purpose::Download).namespace,
            selected.namespace
        );
        let stored: String = app
            .backup_db(|db| {
                Ok(db.query_row(
                    "SELECT session_ref FROM telegram_account_bindings WHERE purpose='download'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(stored, selected.namespace);
        app.shutdown().await;
    }
    #[tokio::test]
    async fn expired_login_uses_a_fresh_pending_session_without_overwriting_the_shared_one() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let old = app.telegram_accounts.binding(Purpose::Download);
        old.session.set_test_view(Phase::SignedOut, "old-account");
        let error = anyhow::anyhow!("TELEGRAM_RELOGIN_REQUIRED");
        assert!(app
            .telegram_accounts
            .replacement_after_expiry(
                Purpose::Download,
                LoginMode::Reuse,
                &old,
                Some(&anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR"))
            )
            .unwrap()
            .is_none());
        let fresh = app
            .telegram_accounts
            .replacement_after_expiry(Purpose::Download, LoginMode::Reuse, &old, Some(&error))
            .unwrap()
            .unwrap();
        assert_ne!(fresh.namespace, old.namespace);
        assert_eq!(
            app.telegram_accounts.binding(Purpose::Backup).namespace,
            old.namespace
        );
        assert_eq!(
            app.telegram_accounts.binding(Purpose::Download).namespace,
            old.namespace
        );
        fresh.session.set_test_view(Phase::Ready, "account-a");
        app.telegram_accounts
            .finish(&app, Purpose::Download, &fresh, Ok(fresh.session.status()))
            .await
            .unwrap();
        assert_eq!(
            app.telegram_accounts.binding(Purpose::Download).namespace,
            fresh.namespace
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn defaults_share_existing_namespace_and_login_reuses_ready_account() {
        let root = tempfile::tempdir().unwrap();
        let core = core(root.path());
        let download = core.telegram_accounts.binding(Purpose::Download);
        assert_eq!(
            download.namespace,
            core.telegram_storage_device_id().unwrap()
        );
        assert!(Arc::ptr_eq(
            &download.session,
            &core.telegram_accounts.binding(Purpose::Backup).session
        ));
        download.session.set_test_view(Phase::Ready, "account-a");
        let ready = core
            .telegram_accounts
            .login(&core, Purpose::Backup, LoginMode::Reuse)
            .await
            .unwrap();
        assert_eq!(ready.account.unwrap().id, "account-a");
        assert_eq!(ready.operation_id, download.session.status().operation_id);
        core.shutdown().await;
    }
    #[tokio::test]
    async fn switching_each_purpose_keeps_other_account_and_persists_selection() {
        for purpose in [Purpose::Download, Purpose::Backup] {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let old = app.telegram_accounts.binding(purpose);
            old.session.set_test_view(Phase::Ready, "account-a");
            let next = pending(&app.telegram_accounts, purpose, Phase::Ready);
            app.telegram_accounts
                .finish(&app, purpose, &next, Ok(next.session.status()))
                .await
                .unwrap();
            assert_eq!(
                app.telegram_accounts.status(purpose).account.unwrap().id,
                "account-b"
            );
            assert_eq!(
                app.telegram_accounts
                    .status(purpose.other())
                    .account
                    .unwrap()
                    .id,
                "account-a"
            );
            assert!(Arc::ptr_eq(
                &old.session,
                &app.telegram_accounts.binding(purpose.other()).session
            ));
            app.shutdown().await;
            drop(app);
            let restored = core(root.path());
            assert_eq!(
                restored.telegram_accounts.binding(purpose).namespace,
                next.namespace
            );
            assert_eq!(
                restored
                    .telegram_accounts
                    .binding(purpose.other())
                    .namespace,
                old.namespace
            );
            assert!(!Arc::ptr_eq(
                &restored.telegram_accounts.binding(purpose).session,
                &restored.telegram_accounts.binding(purpose.other()).session
            ));
            restored.shutdown().await;
        }
    }
    #[tokio::test]
    async fn canceled_or_failed_replacement_keeps_original_account() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.telegram_accounts
            .binding(Purpose::Download)
            .session
            .set_test_view(Phase::Ready, "account-a");
        let qr = pending(&app.telegram_accounts, Purpose::Download, Phase::Qr);
        let view = app
            .telegram_accounts
            .cancel(
                Purpose::Download,
                qr.session.status().operation_id.as_deref().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(view.account.unwrap().id, "account-a");
        let failed = pending(&app.telegram_accounts, Purpose::Backup, Phase::SignedOut);
        assert!(app
            .telegram_accounts
            .finish(
                &app,
                Purpose::Backup,
                &failed,
                Err(anyhow::anyhow!("TELEGRAM_QR_TIMEOUT"))
            )
            .await
            .is_err());
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Backup)
                .account
                .unwrap()
                .id,
            "account-a"
        );
        assert!(Arc::ptr_eq(
            &app.telegram_accounts.binding(Purpose::Backup).session,
            &app.telegram_accounts.binding(Purpose::Download).session
        ));
        app.shutdown().await;
    }
    #[tokio::test]
    async fn connection_failure_during_account_replacement_keeps_the_selected_login() {
        let root=tempfile::tempdir().unwrap();let app=core(root.path());
        let original=app.telegram_accounts.binding(Purpose::Backup);
        original.session.set_test_view(Phase::Ready,"account-a");
        let failed=pending(&app.telegram_accounts,Purpose::Backup,Phase::Disconnected);
        assert!(app.telegram_accounts.finish(&app,Purpose::Backup,&failed,Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR"))).await.is_err());
        assert!(Arc::ptr_eq(&app.telegram_accounts.binding(Purpose::Backup).session,&original.session));
        assert!(matches!(app.telegram_accounts.status(Purpose::Backup).phase,Phase::Ready));
        assert!(app.telegram_accounts.choices.lock().unwrap().pending[Purpose::Backup.index()].is_none());
        app.shutdown().await;
    }
    #[tokio::test]
    async fn logout_shared_account_detaches_only_current_purpose() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let shared = app.telegram_accounts.binding(Purpose::Backup);
        shared.session.set_test_view(Phase::Ready, "account-a");
        let original_id = shared.session.status().operation_id.unwrap();
        let out = app
            .telegram_accounts
            .logout(&app, Purpose::Download, &original_id)
            .await
            .unwrap();
        assert!(matches!(out.phase, Phase::SignedOut));
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Backup)
                .operation_id
                .as_deref(),
            Some(original_id.as_str())
        );
        assert_ne!(
            app.telegram_accounts.binding(Purpose::Download).namespace,
            shared.namespace
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn failed_binding_write_does_not_replace_active_account() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.telegram_accounts
            .binding(Purpose::Backup)
            .session
            .set_test_view(Phase::Ready, "account-a");
        let next = pending(&app.telegram_accounts, Purpose::Backup, Phase::Ready);
        app.backup_db(|db| { db.execute_batch("CREATE TRIGGER reject_account_binding BEFORE INSERT ON telegram_account_bindings BEGIN SELECT RAISE(FAIL,'QA write failure'); END")?; Ok(()) }).unwrap();
        assert!(app
            .telegram_accounts
            .finish(&app, Purpose::Backup, &next, Ok(next.session.status()))
            .await
            .is_err());
        assert_eq!(
            app.telegram_accounts
                .binding(Purpose::Backup)
                .session
                .status()
                .account
                .unwrap()
                .id,
            "account-a"
        );
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Backup)
                .account
                .unwrap()
                .id,
            "account-a"
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn canceled_authorization_cannot_commit_a_late_ready_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.telegram_accounts
            .binding(Purpose::Download)
            .session
            .set_test_view(Phase::Ready, "account-a");
        let next = pending(&app.telegram_accounts, Purpose::Download, Phase::Ready);
        let ready = next.session.status();
        app.telegram_accounts
            .cancel(Purpose::Download, ready.operation_id.as_deref().unwrap())
            .await
            .unwrap();
        let _ = app
            .telegram_accounts
            .finish(&app, Purpose::Download, &next, Ok(ready))
            .await;
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Download)
                .account
                .unwrap()
                .id,
            "account-a"
        );
        assert_eq!(
            app.telegram_accounts
                .binding(Purpose::Download)
                .session
                .status()
                .account
                .unwrap()
                .id,
            "account-a"
        );
        let direct = pending(&app.telegram_accounts, Purpose::Download, Phase::Ready);
        let old_ready = direct.session.status();
        direct
            .session
            .cancel(old_ready.operation_id.as_deref().unwrap())
            .await
            .unwrap();
        assert!(app
            .telegram_accounts
            .finish(&app, Purpose::Download, &direct, Ok(old_ready))
            .await
            .is_err());
        assert_eq!(
            app.telegram_accounts
                .status(Purpose::Download)
                .account
                .unwrap()
                .id,
            "account-a"
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn removing_telegram_backup_configuration_keeps_both_account_bindings_and_proxy() {
        let root = tempfile::tempdir().unwrap(); let app = core(root.path());
        let download = app.telegram_accounts.binding(Purpose::Download);
        let backup = app.telegram_accounts.binding(Purpose::Backup);
        backup.session.set_test_view(Phase::Ready,"account-a");
        app.save_profile(crate::settings::ProfileUpdate { platform:"telegram".into(),user_agent:"test".into(),cookie:None,proxy:Some("socks5://127.0.0.1:1080".into()),http_chunk_size:None }).unwrap();
        app.backup_db(|db| {
            db.execute_batch("INSERT INTO backup_spaces VALUES('telegram-qa','library-qa','','','',NULL,0);
                INSERT INTO backup_provider_config(space_id,provider_type,profile) VALUES('telegram-qa','telegram','telegram');
                INSERT INTO telegram_spaces(space_id,account_id,channel_id) VALUES('telegram-qa','account-a','100');")?;
            Ok(())
        }).unwrap();
        crate::backup::spaces::remove(&app,"telegram-qa",false).await.unwrap();
        assert!(crate::backup::spaces::list(&app).unwrap().is_empty());
        for (purpose, binding) in [(Purpose::Download,&download),(Purpose::Backup,&backup)] {
            assert!(Arc::ptr_eq(&app.telegram_accounts.binding(purpose).session,&binding.session));
            assert!(matches!(app.telegram_accounts.status(purpose).phase,Phase::Ready));
        }
        let id = download.session.status().operation_id.unwrap();
        app.telegram_accounts.logout(&app,Purpose::Download,&id).await.unwrap();
        assert!(matches!(backup.session.status().phase,Phase::Ready));
        assert_eq!(app.proxy("telegram").unwrap().as_deref(),Some("socks5://127.0.0.1:1080/"));
        assert!(app.telegram_accounts.proxy.lock().unwrap().as_ref().unwrap().proxy.is_some());
        app.shutdown().await;
    }

}
