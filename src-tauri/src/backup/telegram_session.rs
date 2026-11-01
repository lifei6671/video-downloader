//! Independent Telegram storage authorization and read-only private-channel discovery.
use super::telegram_vault::{SystemSessionVault, VaultSession};
use anyhow::{bail, Context, Result};
use grammers_client::{
    peer::{Peer, User},
    sender::{ConnectionParams, NoRetries, SenderPoolHandle},
    tl, Client, InvocationError, SenderPool, SignInError,
};
use serde::Serialize;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
include!(concat!(env!("OUT_DIR"), "/telegram_app.rs"));

struct AppIdentity {
    id: i32,
    hash: Zeroizing<String>,
}
impl AppIdentity {
    fn load() -> Result<Self> {
        Self::parse(TELEGRAM_API_ID, TELEGRAM_API_HASH)
    }
    fn parse(id: Option<&str>, hash: Option<&str>) -> Result<Self> {
        let id = id.and_then(|s| s.parse::<i32>().ok()).filter(|id| *id > 0);
        let hash = hash.filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()));
        match (id, hash) {
            (Some(id), Some(hash)) => Ok(Self {
                id,
                hash: Zeroizing::new(hash.into()),
            }),
            _ => bail!("TELEGRAM_APP_NOT_CONFIGURED"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    SignedOut,
    Connecting,
    Qr,
    Password,
    Ready,
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub configured: bool,
    pub operation_id: Option<String>,
    pub phase: Phase,
    pub account: Option<Account>,
    pub last_error: Option<String>,
    pub qr: Option<Vec<Vec<bool>>>,
    pub qr_expires_at: Option<i64>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            configured: AppIdentity::load().is_ok(),
            operation_id: None,
            phase: Phase::SignedOut,
            account: None,
            last_error: None,
            qr: None,
            qr_expires_at: None,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Channel {
    pub id: String,
    pub title: String,
    pub can_post: bool,
    pub can_delete: bool,
}

struct Control {
    view: View,
    cancel: watch::Sender<bool>,
}
struct Connection {
    client: Client,
    session: Arc<VaultSession>,
    handle: SenderPoolHandle,
    runner: Mutex<Option<JoinHandle<()>>>,
    revoked: AtomicBool,
    stopped: AtomicBool,
    updates: tokio::sync::Mutex<
        tokio::sync::mpsc::Receiver<grammers_client::session::updates::UpdatesLike>,
    >,
}
impl Connection {
    async fn open(namespace: String, api_id: i32) -> Result<Self> {
        let mut session =
            VaultSession::load(namespace.clone(), Arc::new(SystemSessionVault)).await?;
        if session.revoked() {
            session.erase().await?;
            session = VaultSession::load(namespace, Arc::new(SystemSessionVault)).await?;
        }
        Ok(Self::start(session, api_id))
    }
    fn start(session: Arc<VaultSession>, api_id: i32) -> Self {
        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::with_configuration(
            session.clone(),
            api_id,
            ConnectionParams {
                device_model: "Yingqu Telegram Storage".into(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                retry_policy: Box::new(NoRetries),
                ..Default::default()
            },
        );
        let thin = handle.thin.clone();
        Self {
            client: Client::new(handle),
            session,
            handle: thin,
            runner: Mutex::new(Some(tokio::spawn(runner.run()))),
            revoked: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            updates: tokio::sync::Mutex::new(updates),
        }
    }
    async fn close(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.handle.quit();
        let runner = self
            .runner
            .lock()
            .expect("telegram runner lock poisoned")
            .take();
        if let Some(mut runner) = runner {
            // A connecting sender may not service quit yet. Abort its owned pool if it cannot drain.
            if tokio::time::timeout(Duration::from_secs(2), &mut runner)
                .await
                .is_err()
            {
                runner.abort();
                let _ = runner.await;
            }
        }
    }
    async fn reconnect_if_stopped(&mut self, api_id: i32) {
        if self.stopped.load(Ordering::SeqCst) && !self.revoked.load(Ordering::SeqCst) {
            self.close().await;
            *self = Self::start(self.session.clone(), api_id);
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.handle.quit();
        if let Some(runner) = self
            .runner
            .get_mut()
            .expect("telegram runner lock poisoned")
            .take()
        {
            runner.abort();
        }
    }
}
struct QrConnection<'a> {
    connection: &'a Connection,
    identity: &'a AppIdentity,
}
impl super::telegram_qr::Transport for QrConnection<'_> {
    async fn export(&self) -> Result<super::telegram_qr::Response> {
        super::telegram_qr::response(
            self.connection
                .client
                .invoke(&tl::functions::auth::ExportLoginToken {
                    api_id: self.identity.id,
                    api_hash: self.identity.hash.to_string(),
                    except_ids: vec![],
                })
                .await,
        )
    }
    async fn import(&self, dc: i32, token: &[u8]) -> Result<super::telegram_qr::Response> {
        use grammers_client::session::Session;
        // Persist the target before importing, so an uncertain response can be checked on restart.
        self.connection.session.set_home_dc_id(dc).await?;
        super::telegram_qr::response(
            self.connection
                .client
                .invoke_in_dc(
                    dc,
                    &tl::functions::auth::ImportLoginToken {
                        token: token.to_vec(),
                    },
                )
                .await,
        )
    }
    async fn account(&self) -> Result<Account> {
        let user = self.connection.client.get_me().await.map_err(rpc_error)?;
        save_account(self.connection, &user).await
    }
    async fn scan_update(&self) -> Result<()> {
        let mut updates = self.connection.updates.lock().await;
        while let Some(update) = updates.recv().await {
            if super::telegram_qr::is_login_update(update) {
                return Ok(());
            }
        }
        bail!("TELEGRAM_CONNECTION_ERROR")
    }
}
#[derive(Default)]
struct State {
    connection: Option<Connection>,
}

pub struct TelegramSession {
    control: Mutex<Control>,
    state: tokio::sync::Mutex<State>,
    closed: AtomicBool,
    transfer: Mutex<Option<watch::Sender<bool>>>,
}
impl Default for TelegramSession {
    fn default() -> Self {
        Self {
            control: Mutex::new(Control {
                view: View::default(),
                cancel: watch::channel(false).0,
            }),
            state: tokio::sync::Mutex::new(State::default()),
            closed: AtomicBool::new(false),
            transfer: Mutex::new(None),
        }
    }
}
impl TelegramSession {
    pub fn status(&self) -> View {
        self.control
            .lock()
            .expect("telegram control lock poisoned")
            .view
            .clone()
    }
    fn start_operation(&self) -> Result<watch::Receiver<bool>> {
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if self.closed.load(Ordering::SeqCst) {
            bail!("TELEGRAM_CLOSED");
        }
        control.cancel.send_replace(true);
        let (cancel, rx) = watch::channel(false);
        control.cancel = cancel;
        control.view = View {
            configured: true,
            operation_id: Some(uuid::Uuid::new_v4().to_string()),
            phase: Phase::Connecting,
            account: None,
            last_error: None,
            qr: None,
            qr_expires_at: None,
        };
        Ok(rx)
    }
    fn operation(&self, id: &str) -> Result<watch::Receiver<bool>> {
        let control = self.control.lock().expect("telegram control lock poisoned");
        if self.closed.load(Ordering::SeqCst)
            || *control.cancel.borrow()
            || control.view.operation_id.as_deref() != Some(id)
        {
            bail!("TELEGRAM_STALE_OPERATION");
        }
        Ok(control.cancel.subscribe())
    }
    fn set_view(
        &self,
        cancel: &watch::Receiver<bool>,
        phase: Phase,
        account: Option<Account>,
        error: Option<String>,
    ) {
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if !*cancel.borrow() && !self.closed.load(Ordering::SeqCst) {
            control.view.phase = phase;
            control.view.account = account;
            control.view.last_error = error;
            control.view.qr = None;
            control.view.qr_expires_at = None;
        }
    }
    async fn finish(
        &self,
        state: &mut State,
        cancel: &watch::Receiver<bool>,
        result: Result<()>,
    ) -> Result<View> {
        if let Err(error) = result {
            let message = error.to_string();
            if matches!(
                message.as_str(),
                "TELEGRAM_CANCELED" | "TELEGRAM_TIMEOUT" | "TELEGRAM_QR_TIMEOUT"
            ) {
                if let Some(connection) = state.connection.take() {
                    connection.close().await;
                }
                self.set_view(cancel, Phase::SignedOut, None, Some(message));
            } else {
                let mut control = self.control.lock().expect("telegram control lock poisoned");
                if !*cancel.borrow() {
                    control.view.last_error = Some(message);
                }
            }
            return Err(error);
        }
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if *cancel.borrow() {
            bail!("TELEGRAM_CANCELED");
        }
        control.view.last_error = None;
        Ok(control.view.clone())
    }
    /// Resume uses only the app's own vault namespace and never starts a new authorization.
    pub async fn resume(&self, device_id: String) -> Result<View> {
        self.connect(device_id, false).await
    }
    pub(super) async fn ready_for_backup(&self, device_id: String) -> Result<View> {
        let view = self.status();
        match view.phase {
            Phase::Ready => Ok(view),
            Phase::SignedOut | Phase::Connecting => self.resume(device_id).await,
            _ => bail!("TELEGRAM_BUSY"),
        }
    }
    pub async fn login_qr(&self, device_id: String) -> Result<View> {
        self.connect(device_id, true).await
    }
    async fn connect(&self, device_id: String, qr_login: bool) -> Result<View> {
        if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
        let view = self.status();
        if matches!(view.phase, Phase::Ready) { return Ok(view); }
        let mut state = if qr_login {
            self.state.try_lock().map_err(|_| anyhow::anyhow!("TELEGRAM_BUSY"))?
        } else {
            // Another backup worker may already be restoring this same session.
            tokio::time::timeout(REQUEST_TIMEOUT, self.state.lock()).await
                .map_err(|_| anyhow::anyhow!("TELEGRAM_TIMEOUT"))?
        };
        if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
        let view = self.status();
        if matches!(view.phase, Phase::Ready) {
            return Ok(view);
        }
        if !qr_login && !matches!(self.status().phase, Phase::SignedOut) {
            bail!("TELEGRAM_BUSY");
        }
        let identity = AppIdentity::load()?;
        let mut cancel = self.start_operation()?;
        if let Some(connection) = state.connection.take() {
            connection.close().await;
        }
        let result = async {
            state.connection =
                Some(Connection::open(format!("{device_id}/session-v1"), identity.id).await?);
            let connection = state
                .connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?;
            if bounded(&mut cancel, async {
                connection.client.is_authorized().await.map_err(rpc_error)
            })
            .await?
            {
                let account = checked_account(connection, &mut cancel).await?;
                self.set_view(&cancel, Phase::Ready, Some(account), None);
            } else if qr_login {
                if connection.session.account_id().is_some() {
                    bail!("TELEGRAM_AUTH_REQUIRED");
                }
                let transport = QrConnection {
                    connection,
                    identity: &identity,
                };
                let display_cancel = cancel.clone();
                let outcome = tokio::time::timeout(
                    super::telegram_qr::LOGIN_TIMEOUT,
                    super::telegram_qr::run(&transport, &mut cancel, |qr| {
                        let mut control =
                            self.control.lock().expect("telegram control lock poisoned");
                        if !*display_cancel.borrow() && !self.closed.load(Ordering::SeqCst) {
                            control.view.phase = if qr.is_some() {
                                Phase::Qr
                            } else {
                                Phase::Connecting
                            };
                            control.view.qr_expires_at = qr.as_ref().map(|qr| qr.expires);
                            control.view.qr = qr.map(|qr| qr.matrix);
                        }
                    }),
                )
                .await
                .map_err(|_| anyhow::anyhow!("TELEGRAM_QR_TIMEOUT"))??;
                match outcome {
                    super::telegram_qr::Outcome::Ready(account) => {
                        self.set_view(&cancel, Phase::Ready, Some(account), None)
                    }
                    super::telegram_qr::Outcome::Password => {
                        self.set_view(&cancel, Phase::Password, None, None)
                    }
                }
            } else {
                self.set_view(&cancel, Phase::SignedOut, None, None);
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            self.set_view(&cancel, Phase::SignedOut, None, None);
        }
        self.finish(&mut state, &cancel, result).await
    }
    pub async fn submit_password(&self, id: &str, password: String) -> Result<View> {
        let password = Zeroizing::new(password);
        if password.is_empty() || password.len() > 1024 {
            bail!("TELEGRAM_INVALID_PASSWORD");
        }
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| anyhow::anyhow!("TELEGRAM_BUSY"))?;
        let mut cancel = self.operation(id)?;
        if !matches!(self.status().phase, Phase::Password) {
            bail!("TELEGRAM_INVALID_PHASE");
        }
        let result = async {
            let connection = state
                .connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?;
            let token = bounded(&mut cancel, async {
                connection
                    .client
                    .invoke(&tl::functions::account::GetPassword {})
                    .await
                    .map_err(rpc_error)
            })
            .await?;
            let tl::enums::account::Password::Password(token) = token;
            let user = bounded(&mut cancel, async {
                connection
                    .client
                    .check_password(
                        grammers_client::client::PasswordToken::new(token),
                        password.as_str(),
                    )
                    .await
                    .map_err(sign_in_error)
            })
            .await?;
            let account = save_account(connection, &user).await?;
            self.set_view(&cancel, Phase::Ready, Some(account), None);
            Ok(())
        }
        .await;
        self.finish(&mut state, &cancel, result).await
    }
    pub async fn channels(&self, id: &str) -> Result<Vec<Channel>> {
        let mut cancel = self.operation(id)?;
        // Channel discovery shares the connection with backup requests; contention is not a failure.
        let mut state = bounded(&mut cancel, async { Ok(self.state.lock().await) }).await?;
        self.operation(id)?;
        if !matches!(self.status().phase, Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        let result = async {
            let connection = state
                .connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?;
            checked_account(connection, &mut cancel).await?;
            bounded(&mut cancel, async {
                let mut channels = Vec::new();
                let mut dialogs = connection.client.iter_dialogs();
                while let Some(dialog) = dialogs.next().await.map_err(rpc_error)? {
                    if let Peer::Channel(channel) = dialog.peer() {
                        if channel.raw.left
                            || channel.username().is_some()
                            || !channel.usernames().is_empty()
                        {
                            continue;
                        }
                        let rights = channel.raw.admin_rights.as_ref().map(|r| {
                            let tl::enums::ChatAdminRights::Rights(rights) = r;
                            rights
                        });
                        let can_post =
                            channel.raw.creator || rights.is_some_and(|r| r.post_messages);
                        let can_delete =
                            channel.raw.creator || rights.is_some_and(|r| r.delete_messages);
                        channels.push(Channel {
                            id: channel.id().bare_id_unchecked().to_string(),
                            title: channel.title().into(),
                            can_post,
                            can_delete,
                        });
                    }
                }
                Ok(channels)
            })
            .await
        }
        .await;
        match result {
            Ok(channels) => {
                self.finish(&mut state, &cancel, Ok(())).await?;
                Ok(channels)
            }
            Err(error) => {
                self.finish(&mut state, &cancel, Err(error)).await?;
                unreachable!("error finish always fails")
            }
        }
    }
    /// T2 transport entry point; library publication remains gated until T3.
    pub async fn upload_media(
        &self,
        core: &crate::desktop_core::Core,
        request: super::telegram_transfer::UploadRequest,
        cancel: watch::Receiver<bool>,
    ) -> Result<super::provider::UploadReceipt> {
        let output = self
            .transfer_media(
                core,
                &request.login_id,
                request.target,
                super::telegram_transfer::Job::Upload(request.source),
                cancel,
            )
            .await?;
        let super::telegram_transfer::Output::Uploaded(receipt) = output else {
            unreachable!("upload job always returns an upload receipt");
        };
        Ok(receipt)
    }
    pub async fn restore_media(
        &self,
        core: &crate::desktop_core::Core,
        request: super::telegram_transfer::RestoreRequest,
        cancel: watch::Receiver<bool>,
    ) -> Result<std::path::PathBuf> {
        let output = self
            .transfer_media(
                core,
                &request.login_id,
                request.target,
                super::telegram_transfer::Job::Restore {
                    reference: request.reference,
                    destination: request.destination,
                },
                cancel,
            )
            .await?;
        let super::telegram_transfer::Output::Restored(path) = output else {
            unreachable!("restore job always returns a committed path");
        };
        Ok(path)
    }
    async fn transfer_media(
        &self,
        core: &crate::desktop_core::Core,
        id: &str,
        target: super::telegram_transfer::Target,
        job: super::telegram_transfer::Job,
        cancel: watch::Receiver<bool>,
    ) -> Result<super::telegram_transfer::Output> {
        use super::{
            telegram_transfer::{self as transfer, Job, Output},
            telegram_transfer_journal::CoreJournal,
        };
        let control = match &job {
            Job::Upload(source) => source.control.clone(),
            _ => None,
        };
        let bound = target.clone();
        self.with_transport(
            core,
            id,
            target,
            cancel,
            control,
            move |core, transport, io| {
                Box::pin(async move {
                    let journal = CoreJournal(core);
                    match job {
                        Job::Upload(source) => Ok(Output::Uploaded(
                            transfer::upload(transport, &journal, &bound, source, io).await?,
                        )),
                        Job::Restore {
                            reference,
                            destination,
                        } => Ok(Output::Restored(
                            transfer::restore(
                                transport,
                                &journal,
                                &bound,
                                &reference,
                                &destination,
                                io,
                            )
                            .await?,
                        )),
                    }
                })
            },
        )
        .await
    }
    /// Serialize channel requests on the existing owned connection and cancellation lifecycle.
    pub(super) async fn with_transport<T>(
        &self,
        core: &crate::desktop_core::Core,
        id: &str,
        target: super::telegram_transfer::Target,
        mut cancel: watch::Receiver<bool>,
        control: Option<Arc<super::provider::TransferControl>>,
        work: impl for<'a> FnOnce(
            &'a crate::desktop_core::Core,
            &'a super::telegram_transport::GrammersTransport,
            &'a super::telegram_transfer::Io,
        ) -> super::provider::ProviderFuture<'a, T>,
    ) -> Result<T> {
        use super::{
            telegram_transfer as transfer,
            telegram_transfer_journal::{CoreJournal, Journal},
            telegram_transport::GrammersTransport,
        };
        if *cancel.borrow() {
            bail!("TELEGRAM_TRANSFER_CANCELED");
        }
        let state = tokio::select! { biased;
            _=cancel.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
            state=self.state.lock()=>state,
        };
        let stop = self.operation(id)?;
        if !matches!(self.status().phase, Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        target.validate()?;
        let journal = CoreJournal(core);
        let wait = journal
            .cooldown(&target.account_id)?
            .saturating_sub(transfer::now());
        if wait > 0 {
            bail!("TELEGRAM_FLOOD_WAIT:{wait}");
        }
        let io = transfer::Io::new(cancel, stop.clone(), control);
        let connection = state
            .connection
            .as_ref()
            .context("TELEGRAM_NOT_CONNECTED")?;
        *self
            .transfer
            .lock()
            .expect("telegram transfer lock poisoned") = Some(io.abort_handle());
        let mut registration = TransferRegistration {
            session: self,
            connection,
            stop: stop.clone(),
            completed: false,
        };
        let result = async {
            let user = io
                .run(async { connection.client.get_me().await.map_err(rpc_error) })
                .await?;
            let account = save_account(connection, &user).await?;
            if account.id != target.account_id {
                bail!("TELEGRAM_ACCOUNT_MISMATCH");
            }
            let transport = GrammersTransport::connect(
                connection.client.clone(),
                &target,
                matches!(&user.raw,tl::enums::User::User(user) if user.premium),
                &io,
            )
            .await?;
            super::telegram_spaces::remember_channel_title(core, &target, transport.channel_title())?;
            work(core, &transport, &io).await
        }
        .await;
        if let Err(error) = &result {
            journal.failure(&target.account_id, &error.to_string())?;
            connection.close().await;
            self.set_view(&stop, Phase::SignedOut, None, Some(error.to_string()));
        }
        registration.completed = true;
        result
    }
    /// Signal first so an in-flight network request cannot hold cancellation behind the state lock.
    pub async fn cancel(&self, id: &str) -> Result<View> {
        {
            let control = self.control.lock().expect("telegram control lock poisoned");
            if control.view.operation_id.as_deref() != Some(id) {
                bail!("TELEGRAM_STALE_OPERATION");
            }
            control.cancel.send_replace(true);
        }
        let mut state = self.state.lock().await;
        if self.status().operation_id.as_deref() != Some(id) {
            bail!("TELEGRAM_STALE_OPERATION");
        }
        if let Some(connection) = state.connection.take() {
            connection.close().await;
        }
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        control.view = View::default();
        Ok(control.view.clone())
    }
    pub async fn logout(&self, id: &str) -> Result<View> {
        self.operation(id)?;
        let transfer = self
            .transfer
            .lock()
            .expect("telegram transfer lock poisoned")
            .clone();
        let mut state = if let Some(cancel) = transfer {
            cancel.send_replace(true);
            self.state.lock().await
        } else {
            self.state
                .try_lock()
                .map_err(|_| anyhow::anyhow!("TELEGRAM_BUSY"))?
        };
        let mut cancel = self.operation(id)?;
        if let Some(connection) = state.connection.as_mut() {
            connection
                .reconnect_if_stopped(AppIdentity::load()?.id)
                .await;
        }
        let result = async {
            let connection = state
                .connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?;
            // Revoke first. On failure keep the keys so retry can resolve an uncertain outcome.
            if !connection.revoked.load(Ordering::SeqCst) {
                bounded(&mut cancel, async {
                    match connection.client.sign_out().await {
                        Ok(_) => Ok(()),
                        Err(InvocationError::Rpc(error)) if error.code == 401 => Ok(()),
                        Err(error) => Err(rpc_error(error)),
                    }
                })
                .await?;
                connection.revoked.store(true, Ordering::SeqCst);
            }
            connection.close().await;
            connection.session.erase().await?;
            Ok(())
        }
        .await;
        self.finish(&mut state, &cancel, result).await?;
        state.connection = None;
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        control.view = View::default();
        Ok(control.view.clone())
    }
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.control
            .lock()
            .expect("telegram control lock poisoned")
            .cancel
            .send_replace(true);
        let mut state = self.state.lock().await;
        if let Some(connection) = state.connection.take() {
            connection.close().await;
        }
    }
}
struct TransferRegistration<'a> {
    session: &'a TelegramSession,
    connection: &'a Connection,
    stop: watch::Receiver<bool>,
    completed: bool,
}
impl Drop for TransferRegistration<'_> {
    fn drop(&mut self) {
        self.session
            .transfer
            .lock()
            .expect("telegram transfer lock poisoned")
            .take();
        if !self.completed {
            // A caller can abandon a future. Stop its actual sender and retain the join handle for shutdown.
            self.connection.handle.quit();
            self.connection.stopped.store(true, Ordering::SeqCst);
            if let Some(runner) = self
                .connection
                .runner
                .lock()
                .expect("telegram runner lock poisoned")
                .as_ref()
            {
                runner.abort();
            }
            self.session.set_view(
                &self.stop,
                Phase::SignedOut,
                None,
                Some("TELEGRAM_TRANSFER_CANCELED".into()),
            );
        }
    }
}

async fn checked_account(
    connection: &Connection,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Account> {
    let user: User = bounded(cancel, async {
        connection.client.get_me().await.map_err(rpc_error)
    })
    .await?;
    save_account(connection, &user).await
}
async fn save_account(connection: &Connection, user: &User) -> Result<Account> {
    use grammers_client::session::{types::PeerInfo, Session};
    let account = account(connection, user)?;
    let peer = PeerInfo::from(&user.raw);
    if !matches!(
        &peer,
        PeerInfo::User {
            is_self: Some(true),
            ..
        }
    ) {
        bail!("TELEGRAM_ACCOUNT_MISMATCH");
    }
    // Raw QR RPCs bypass Client::complete_login; get_me itself does not cache the self peer.
    // Persist the stable identity before publishing Ready, including during session recovery.
    connection.session.cache_peer(peer).await?;
    Ok(account)
}
fn account(connection: &Connection, user: &User) -> Result<Account> {
    let id = user.id().bare_id_unchecked();
    if user.is_bot() || connection.session.account_id().is_some_and(|old| old != id) {
        bail!("TELEGRAM_ACCOUNT_MISMATCH");
    }
    Ok(Account {
        id: id.to_string(),
        name: user.full_name(),
    })
}
pub(super) async fn bounded<T>(
    cancel: &mut watch::Receiver<bool>,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    if *cancel.borrow() {
        bail!("TELEGRAM_CANCELED");
    }
    tokio::select! {
        biased;
        _ = cancel.wait_for(|value| *value) => bail!("TELEGRAM_CANCELED"),
        result = tokio::time::timeout(REQUEST_TIMEOUT, work) => result.map_err(|_| anyhow::anyhow!("TELEGRAM_TIMEOUT"))?,
    }
}
pub(super) fn rpc_error(error: InvocationError) -> anyhow::Error {
    // Never forward Display/Debug: RPC errors may carry request or connection context.
    let code = match error {
        InvocationError::Rpc(error) => {
            if error.name.starts_with("FILE_REFERENCE_") {
                return anyhow::anyhow!("TELEGRAM_FILE_REFERENCE_EXPIRED");
            }
            if error.code == 420 {
                return anyhow::anyhow!("TELEGRAM_FLOOD_WAIT:{}", error.value.unwrap_or(60));
            }
            match error.name.as_str() {
                "PHONE_NUMBER_INVALID" => "TELEGRAM_INVALID_PHONE",
                "PHONE_CODE_INVALID" | "PHONE_CODE_EMPTY" => "TELEGRAM_INVALID_CODE",
                "PHONE_CODE_EXPIRED" => "TELEGRAM_CODE_EXPIRED",
                "PASSWORD_HASH_INVALID" => "TELEGRAM_INVALID_PASSWORD",
                "API_ID_INVALID" => "TELEGRAM_APP_INVALID",
                "AUTH_KEY_UNREGISTERED"
                | "SESSION_REVOKED"
                | "SESSION_EXPIRED"
                | "USER_DEACTIVATED" => "TELEGRAM_AUTH_REQUIRED",
                "CHAT_ADMIN_REQUIRED" | "CHANNEL_PRIVATE" | "CHAT_WRITE_FORBIDDEN" | "CHAT_DELETE_FORBIDDEN" => {
                    "TELEGRAM_CHANNEL_FORBIDDEN"
                }
                "FILE_PARTS_INVALID" | "FILE_PART_TOO_BIG" => "TELEGRAM_FILE_TOO_LARGE",
                _ => "TELEGRAM_RPC_ERROR",
            }
        }
        InvocationError::Session(_) => "TELEGRAM_SESSION_STORAGE",
        _ => "TELEGRAM_CONNECTION_ERROR",
    };
    anyhow::anyhow!(code)
}
fn sign_in_error(error: SignInError) -> anyhow::Error {
    match error {
        SignInError::InvalidCode => anyhow::anyhow!("TELEGRAM_INVALID_CODE"),
        SignInError::InvalidPassword(_) => anyhow::anyhow!("TELEGRAM_INVALID_PASSWORD"),
        SignInError::SignUpRequired => anyhow::anyhow!("TELEGRAM_SIGN_UP_REQUIRED"),
        SignInError::Other(error) => rpc_error(error),
        SignInError::PasswordRequired(_) => anyhow::anyhow!("TELEGRAM_INVALID_PHASE"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn telegram_busy_background_resumes_wait_and_reuse_the_same_ready_operation() {
        let session = TelegramSession::default();
        let token = session.start_operation().unwrap();
        let id = session.status().operation_id;
        let guard = session.state.lock().await;
        let first = session.ready_for_backup("unused".into());
        let second = session.ready_for_backup("unused".into());
        tokio::pin!(first, second);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut first).await.is_err());
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut second).await.is_err());
        session.set_view(&token, Phase::Ready, Some(Account { id: "123".into(), name: "fixture".into() }), None);
        drop(guard);
        let (first, second) = tokio::join!(first, second);
        for view in [first.unwrap(), second.unwrap()] {
            assert!(matches!(view.phase, Phase::Ready));
            assert_eq!(view.operation_id, id);
        }
        assert!(!*token.borrow());
        assert!(session.state.try_lock().is_ok());
        session.shutdown().await;
    }

    #[tokio::test]
    async fn telegram_busy_ready_resume_does_not_interrupt_an_existing_transfer() {
        let session = TelegramSession::default();
        let token = session.start_operation().unwrap();
        session.set_view(&token, Phase::Ready, Some(Account { id: "123".into(), name: "fixture".into() }), None);
        let id = session.status().operation_id;
        let guard = session.state.lock().await;
        let view = tokio::time::timeout(Duration::from_millis(100), session.resume("unused".into())).await.unwrap().unwrap();
        assert_eq!(view.operation_id, id);
        assert!(!*token.borrow());
        drop(guard);
        session.shutdown().await;
    }

    #[tokio::test]
    async fn telegram_busy_channel_wait_can_be_canceled_without_acquiring_the_connection() {
        let session = TelegramSession::default();
        let token = session.start_operation().unwrap();
        session.set_view(&token, Phase::Ready, None, None);
        let id = session.status().operation_id.unwrap();
        let guard = session.state.lock().await;
        let channels = session.channels(&id);
        tokio::pin!(channels);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut channels).await.is_err());
        session.control.lock().unwrap().cancel.send_replace(true);
        let error = tokio::time::timeout(Duration::from_millis(100), channels).await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), "TELEGRAM_CANCELED");
        assert!(session.state.try_lock().is_err());
        drop(guard);
        session.shutdown().await;
    }

    #[tokio::test]
    async fn telegram_busy_channel_wait_still_reports_the_actual_connection_error() {
        let session = TelegramSession::default();
        let token = session.start_operation().unwrap();
        session.set_view(&token, Phase::Ready, None, None);
        let id = session.status().operation_id.unwrap();
        let guard = session.state.lock().await;
        let channels = session.channels(&id);
        tokio::pin!(channels);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut channels).await.is_err());
        drop(guard);
        assert_eq!(channels.await.unwrap_err().to_string(), "TELEGRAM_NOT_CONNECTED");
        assert_eq!(session.status().last_error.as_deref(), Some("TELEGRAM_NOT_CONNECTED"));
        session.shutdown().await;
    }

    #[tokio::test]
    async fn telegram_busy_waiting_resume_cannot_reopen_a_closed_session() {
        let session = TelegramSession::default();
        let _token = session.start_operation().unwrap();
        let guard = session.state.lock().await;
        let resume = session.ready_for_backup("unused".into());
        tokio::pin!(resume);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut resume).await.is_err());
        session.closed.store(true, Ordering::SeqCst);
        drop(guard);
        assert_eq!(resume.await.unwrap_err().to_string(), "TELEGRAM_CLOSED");
        assert!(session.state.lock().await.connection.is_none());
        session.shutdown().await;
    }
    #[tokio::test]
    async fn telegram_t3_background_work_preserves_interactive_qr_and_password_login() {
        let session = TelegramSession::default();
        let token = session.start_operation().unwrap();
        let id = session.status().operation_id;
        for phase in [Phase::Connecting, Phase::Qr, Phase::Password] {
            session.set_view(&token, phase, None, None);
            assert_eq!(
                session
                    .ready_for_backup("unused".into())
                    .await
                    .unwrap_err()
                    .to_string(),
                "TELEGRAM_BUSY"
            );
            assert_eq!(session.status().operation_id, id);
            assert!(!*token.borrow());
        }
        session.shutdown().await;
    }
    #[tokio::test]
    async fn telegram_abandoned_transfer_joins_pool_before_logout_reconnect() {
        struct EmptyVault;
        impl super::super::telegram_vault::SessionVault for EmptyVault {
            fn read(&self, _: &str) -> Result<Option<zeroize::Zeroizing<String>>> {
                Ok(None)
            }
            fn write(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        let keys = VaultSession::load("offline-lifecycle".into(), Arc::new(EmptyVault))
            .await
            .unwrap();
        let mut connection = Connection::start(keys.clone(), 12);
        let session = TelegramSession::default();
        let stop = session.start_operation().unwrap();
        let old_runner = connection
            .runner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .abort_handle();
        drop(TransferRegistration {
            session: &session,
            connection: &connection,
            stop,
            completed: false,
        });
        assert!(connection.stopped.load(Ordering::SeqCst));
        // This is the exact reconnect path used by logout, without issuing a real sign-out RPC.
        connection.reconnect_if_stopped(12).await;
        assert!(old_runner.is_finished());
        assert!(!connection.stopped.load(Ordering::SeqCst));
        assert!(Arc::ptr_eq(&connection.session, &keys));
        assert!(!connection
            .runner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_finished());
        connection.close().await;
        assert!(connection.runner.lock().unwrap().is_none());
    }
    #[test]
    fn telegram_app_identity_requires_complete_valid_configuration() {
        for (id, hash) in [
            (None, None),
            (Some("0"), Some("0123456789abcdef0123456789abcdef")),
            (Some("12"), None),
            (Some("12"), Some("secret")),
        ] {
            assert!(AppIdentity::parse(id, hash).is_err());
        }
        assert!(AppIdentity::parse(Some("12"), Some("0123456789abcdef0123456789abcdef")).is_ok());
    }
    #[tokio::test]
    async fn telegram_cancel_drops_the_actual_request_and_rejects_stale_actions() {
        let session = TelegramSession::default();
        let mut cancel = session.start_operation().unwrap();
        let id = session.status().operation_id.unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        struct Request(Arc<AtomicBool>);
        impl Drop for Request {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let request = Request(dropped.clone());
        let work = async move {
            let _request = request;
            std::future::pending::<Result<()>>().await
        };
        {
            let future = bounded(&mut cancel, work);
            tokio::pin!(future);
            tokio::select! { _ = &mut future => panic!("request should remain in flight"), _ = tokio::task::yield_now() => {} }
            session.control.lock().unwrap().cancel.send_replace(true);
            assert_eq!(future.await.unwrap_err().to_string(), "TELEGRAM_CANCELED");
            assert!(dropped.load(Ordering::SeqCst));
        }
        session.cancel(&id).await.unwrap();
        assert!(session.operation(&id).is_err());
        session.set_view(
            &cancel,
            Phase::Ready,
            Some(Account {
                id: "late".into(),
                name: "late".into(),
            }),
            None,
        );
        assert!(session.status().account.is_none());
        session.shutdown().await;
        assert!(session.start_operation().is_err());
    }
}
