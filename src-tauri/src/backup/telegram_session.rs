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
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
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
    Disconnected,
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
            phase: Phase::Disconnected,
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
pub(crate) type ProxySettings = Arc<Mutex<Option<crate::settings::Profile>>>;

struct DeleteConfig {
    login: String,
    target: super::telegram_transfer::Target,
    generation: u64,
    premium: bool,
    bytes: u64,
}

tokio::task_local! {
    static DELETE_CONFIG: std::cell::RefCell<Option<DeleteConfig>>;
}

pub(super) async fn with_delete_config<F: Future>(work: F) -> F::Output {
    DELETE_CONFIG.scope(Default::default(), work).await
}

impl DeleteConfig {
    fn take_for(kind: &str) -> Option<Self> {
        let previous = DELETE_CONFIG.try_with(|slot| slot.take()).ok().flatten();
        previous.filter(|_| kind == "remote_cleanup")
    }
    fn matches(&self, login: &str, target: &super::telegram_transfer::Target, generation: u64) -> bool {
        self.login == login && self.target == *target && self.generation == generation
    }
    fn save(login: &str, target: &super::telegram_transfer::Target, generation: u64, premium: bool, bytes: u64) {
        let _ = DELETE_CONFIG.try_with(|slot| slot.replace(Some(Self {
            login: login.to_owned(), target: target.clone(), generation, premium, bytes,
        })));
    }
}

struct Connection {
    api_id: i32,
    generation: u64,
    proxy_url: Option<String>,
    client: Client,
    session: Arc<VaultSession>,
    handle: SenderPoolHandle,
    media_budget: Mutex<Option<Arc<crate::telegram_transfer_engine::Budget>>>,
    runner: Mutex<Option<JoinHandle<()>>>,
    revoked: AtomicBool,
    stopped: AtomicBool,
    reader_stop: watch::Sender<bool>,
    updates: tokio::sync::Mutex<
        tokio::sync::mpsc::Receiver<grammers_client::session::updates::UpdatesLike>,
    >,
}
impl Connection {
    async fn open(namespace: String, api_id: i32, proxy_url: Option<String>) -> Result<Arc<Self>> {
        let mut session =
            VaultSession::load(namespace.clone(), Arc::new(SystemSessionVault)).await?;
        if session.revoked() {
            session.erase().await?;
            session = VaultSession::load(namespace, Arc::new(SystemSessionVault)).await?;
        }
        Ok(Self::start(session, api_id, proxy_url))
    }
    fn start(session: Arc<VaultSession>, api_id: i32, proxy_url: Option<String>) -> Arc<Self> {
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
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
                proxy_url: proxy_url.clone(),
                ..Default::default()
            },
        );
        let thin = handle.thin.clone();
        Arc::new(Self {
            api_id,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            proxy_url,
            client: Client::new(handle),
            session,
            handle: thin,
            media_budget: Mutex::new(None),
            runner: Mutex::new(Some(tokio::spawn(runner.run()))),
            revoked: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            reader_stop: watch::channel(false).0,
            updates: tokio::sync::Mutex::new(updates),
        })
    }
    fn stop(&self) {
        self.reader_stop.send_replace(true);
        self.stopped.store(true, Ordering::SeqCst);
        self.handle.quit();
    }
    fn enable_media_pool(&self) {
        if self.stopped.load(Ordering::SeqCst) || self.revoked.load(Ordering::SeqCst) { return; }
        let Some(account) = self.session.account_id() else { return; };
        let mut saved = self.media_budget.lock().expect("media connection budget");
        if saved.is_some() { return; }
        let budget = crate::telegram_transfer_engine::Budget::account(&account.to_string());
        if self.handle.enable_media_pool(budget.media_connections()) { *saved = Some(budget); }
    }
    async fn close(&self) {
        self.stop();
        let runner = self
            .runner
            .lock()
            .expect("telegram runner lock poisoned")
            .take();
        if let Some(runner) = runner {
            let mut joining = PoolJoin { connection: self, runner: Some(runner) };
            // A connecting sender may not service quit yet. Abort its owned pool if it cannot drain.
            if tokio::time::timeout(Duration::from_secs(2), joining.runner.as_mut().expect("owned pool runner"))
                .await
                .is_err()
            {
                let runner = joining.runner.as_mut().expect("owned pool runner");
                runner.abort();
                let _ = runner.await;
            }
            joining.runner.take();
        }
    }
    async fn reconnect_if_stopped(self: &mut Arc<Self>, api_id: i32, proxy_url: Option<String>) {
        if (self.stopped.load(Ordering::SeqCst) || self.proxy_url != proxy_url || self.runner.lock().expect("telegram runner").as_ref().is_none_or(|runner| runner.is_finished())) && !self.revoked.load(Ordering::SeqCst) {
            self.close().await;
            *self = Self::start(self.session.clone(), api_id, proxy_url);
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
// Restore can be dropped while loading the vault, before a sender pool even exists.
struct AuthorizationAttempt<'a> {
    session: &'a TelegramSession,
    cancel: watch::Receiver<bool>,
    completed: bool,
}
impl Drop for AuthorizationAttempt<'_> {
    fn drop(&mut self) {
        if self.completed { return; }
        let mut control = self.session.control.lock().expect("telegram control lock poisoned");
        if *self.cancel.borrow() || self.session.closed.load(Ordering::SeqCst) { return; }
        control.cancel.send_replace(true);
        control.view.phase = Phase::Disconnected;
        control.view.account = None;
        control.view.qr = None;
        control.view.qr_expires_at = None;
        control.view.last_error = Some("TELEGRAM_CANCELED".into());
    }
}
// Keep the join handle owned if a caller abandons close while its pool is draining.
struct PoolJoin<'a> { connection: &'a Connection, runner: Option<JoinHandle<()>> }
impl Drop for PoolJoin<'_> {
    fn drop(&mut self) {
        if let Some(runner) = self.runner.take() {
            runner.abort();
            *self.connection.runner.lock().expect("telegram runner lock poisoned") = Some(runner);
        }
    }
}
// A task may cancel while restoring authorization. Retain the owned runner for the next join.
struct ConnectingPool<'a> { connection: &'a Connection, completed: bool }
impl Drop for ConnectingPool<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.connection.reader_stop.send_replace(true);
            self.connection.stopped.store(true, Ordering::SeqCst);
            self.connection.handle.quit();
            if let Some(runner) = self.connection.runner.lock().expect("telegram runner lock poisoned").as_ref() { runner.abort(); }
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
        save_account(&self.connection.session, &user).await
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
    connection: Option<Arc<Connection>>,
}

#[path = "telegram_lock_usage.rs"]
mod lock_usage;
#[cfg(test)]
type ReconnectProbe = fn() -> Result<()>;

pub struct TelegramSession {
    lock_usage: lock_usage::LockUsage,
    proxy: ProxySettings,
    control: Mutex<Control>,
    state: tokio::sync::Mutex<State>,
    closed: AtomicBool,
    network_changing: AtomicBool,
    transfer: Mutex<Option<watch::Sender<bool>>>,
    downloads: Mutex<Option<DownloadAuthorization>>,
    backup_read: tokio::sync::Mutex<()>,
    backup_transaction: tokio::sync::RwLock<()>,
    backup_read_stop: watch::Sender<()>,
    network_stop: watch::Sender<()>,
    pub(crate) reconnect: crate::telegram_reconnect::Reconnect,
    #[cfg(test)]
    pub(crate) reconnect_probe: Mutex<Option<ReconnectProbe>>,
}
impl Default for TelegramSession {
    fn default() -> Self {
        Self {
            lock_usage: Default::default(),
            proxy: Arc::new(Mutex::new(None)),
            control: Mutex::new(Control {
                view: View::default(),
                cancel: watch::channel(false).0,
            }),
            state: tokio::sync::Mutex::new(State::default()),
            closed: AtomicBool::new(false),
            network_changing: AtomicBool::new(false),
            transfer: Mutex::new(None),
            downloads: Mutex::new(None),
            backup_read: Default::default(),
            backup_transaction: Default::default(),
            backup_read_stop: watch::channel(()).0,
            network_stop: watch::channel(()).0,
            reconnect: Default::default(),
            #[cfg(test)]
            reconnect_probe: Mutex::new(None),
        }
    }
}
impl TelegramSession {
    pub(crate) fn with_proxy(proxy: ProxySettings) -> Self {
        Self { proxy, ..Self::default() }
    }
    fn proxy_url(&self) -> Result<Option<String>> {
        self.proxy.lock().unwrap().as_ref().map(crate::settings::profile_proxy).transpose().map(Option::flatten)
    }
    #[cfg(test)]
    pub(crate) fn set_test_view(&self, phase: Phase, account_id: &str) {
        let mut control = self.control.lock().unwrap();
        control.view.operation_id = Some(uuid::Uuid::new_v4().to_string());
        control.view.phase = phase;
        control.view.account = Some(Account { id: account_id.into(), name: account_id.into() });
    }
    pub fn status(&self) -> View {
        let mut view = self.control
            .lock()
            .expect("telegram control lock poisoned")
            .view
            .clone();
        if self.reconnect.active() && view.last_error.as_deref().is_none_or(crate::telegram_reconnect::network_error) {
            view.last_error = Some("TELEGRAM_RECONNECTING".into());
        }
        view
    }
    fn clear_network_error(&self, id: &str) {
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if control.view.operation_id.as_deref() == Some(id)
            && control.view.last_error.as_deref().is_some_and(crate::telegram_reconnect::network_error) {
            control.view.last_error = None;
        }
    }
    pub(crate) fn detached_signed_out(&self) {
        // Only used for a newly created, unshared binding after explicit logout.
        self.control.lock().expect("telegram control lock poisoned").view.phase = Phase::SignedOut;
    }
    fn start_operation(&self) -> Result<watch::Receiver<bool>> {
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if self.network_changing.load(Ordering::SeqCst) { bail!("TELEGRAM_BUSY"); }
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
        if self.network_changing.load(Ordering::SeqCst) { bail!("TELEGRAM_NETWORK_CHANGED"); }
        let control = self.control.lock().expect("telegram control lock poisoned");
        if self.closed.load(Ordering::SeqCst)
            || *control.cancel.borrow()
            || control.view.operation_id.as_deref() != Some(id)
        {
            bail!("TELEGRAM_STALE_OPERATION");
        }
        Ok(control.cancel.subscribe())
    }
    pub(crate) fn has_operation(&self, id: &str) -> bool { self.operation(id).is_ok() }
    fn record_error(&self, cancel: &watch::Receiver<bool>, message: String) {
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        let auth_lost = matches!(control.view.phase, Phase::SignedOut)
            && matches!(control.view.last_error.as_deref(), Some("TELEGRAM_AUTH_REQUIRED" | "TELEGRAM_RELOGIN_REQUIRED"));
        if !*cancel.borrow() && !auth_lost { control.view.last_error = Some(message); }
    }
    fn record_transfer_error(&self, cancel: &watch::Receiver<bool>, message: String) {
        // Request errors are returned to the owning task. Only a connection
        // failure is shared with account settings; auth loss has its own path.
        if crate::telegram_reconnect::network_error(&message) {
            self.record_error(cancel, message);
        }
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
            if matches!(message.as_str(), "TELEGRAM_AUTH_REQUIRED" | "TELEGRAM_RELOGIN_REQUIRED")
                || state.connection.as_ref().is_some_and(|c| c.revoked.load(Ordering::SeqCst) || c.session.revoked()) {
                self.close_downloads().await;
                if let Some(connection) = &state.connection { connection.close().await; }
                // Retain connection/operation so a failed local credential erase remains retryable.
                self.set_view(cancel, Phase::SignedOut, None, Some(message));
            } else if matches!(
                message.as_str(),
                "TELEGRAM_CANCELED" | "TELEGRAM_TIMEOUT" | "TELEGRAM_QR_TIMEOUT"
            ) {
                if let Some(connection) = state.connection.take() {
                    connection.close().await;
                }
                self.set_view(cancel, Phase::Disconnected, None, Some(message));
            } else {
                self.record_error(cancel, message);
            }
            self.authorize_downloads(state, &self.status());
            return Err(error);
        }
        let mut control = self.control.lock().expect("telegram control lock poisoned");
        if *cancel.borrow() {
            bail!("TELEGRAM_CANCELED");
        }
        control.view.last_error = None;
        self.authorize_downloads(state, &control.view);
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
            Phase::Disconnected | Phase::SignedOut | Phase::Connecting => self.resume(device_id).await,
            _ => bail!("TELEGRAM_BUSY"),
        }
    }
    pub async fn login_qr(&self, device_id: String) -> Result<View> {
        self.connect(device_id, true).await
    }
    async fn connect(&self, device_id: String, qr_login: bool) -> Result<View> {
        if self.network_changing.load(Ordering::SeqCst) { bail!("TELEGRAM_BUSY"); }
        if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
        let view = self.status();
        if matches!(view.phase, Phase::Ready) {
            if self.register_connection().is_ok() { return Ok(view); }
            // Proxy changes or a dead runner replace the one main pool, never
            // create an additional main session for a file task.
            let mut state = tokio::time::timeout(REQUEST_TIMEOUT, self.state.lock()).await.map_err(|_| anyhow::anyhow!("TELEGRAM_TIMEOUT"))?;
            if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
            if self.network_changing.load(Ordering::SeqCst) { bail!("TELEGRAM_NETWORK_CHANGED"); }
            if !matches!(self.status().phase, Phase::Ready) { return Ok(self.status()); }
            if let Some(connection) = state.connection.as_mut() {
                connection.reconnect_if_stopped(connection.api_id, self.proxy_url()?).await;
            }
            let view = self.status();
            self.authorize_downloads(&state, &view);
            return Ok(view);
        }
        let mut state = if qr_login {
            self.state.try_lock().map_err(|_| anyhow::anyhow!("TELEGRAM_BUSY"))?
        } else {
            // Another backup worker may already be restoring this same session.
            let wait = super::delete_timing::setup_span("session_lock");
            let _owners = wait.as_ref().map(|_| self.lock_usage.observe_wait());
            tokio::time::timeout(REQUEST_TIMEOUT, self.state.lock()).await
                .map_err(|_| anyhow::anyhow!("TELEGRAM_TIMEOUT"))?
        };
        let _holder = self.lock_usage.hold(if qr_login { "login_qr" } else { "resume" });
        if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
        let view = self.status();
        if matches!(view.phase, Phase::Ready) {
            return Ok(view);
        }
        if !qr_login && !matches!(self.status().phase, Phase::Disconnected | Phase::SignedOut) {
            bail!("TELEGRAM_BUSY");
        }
        // A local erase may fail after Telegram has already revoked the authorization.
        // Finish that cleanup before replacing the connection and losing its retry evidence.
        if let Some(connection) = state.connection.as_ref().filter(|c| c.revoked.load(Ordering::SeqCst)) {
            connection.session.erase().await?;
        }
        let identity = AppIdentity::load()?;
        let mut cancel = self.start_operation()?;
        let mut attempt = AuthorizationAttempt { session:self, cancel:cancel.clone(), completed:false };
        if let Some(connection) = state.connection.take() {
            connection.close().await;
        }
        let result = async {
            state.connection =
                Some(Connection::open(format!("{device_id}/session-v1"), identity.id, self.proxy_url()?).await?);
            let connection = state
                .connection
                .as_ref()
                .context("TELEGRAM_NOT_CONNECTED")?;
            let mut guard = ConnectingPool { connection, completed: false };
            if bounded(&mut cancel, async {
                connection.client.is_authorized().await.map_err(rpc_error)
            })
            .await?
            {
                let account = checked_account(connection, &mut cancel).await?;
                self.set_view(&cancel, Phase::Ready, Some(account), None);
            } else if qr_login {
                if connection.session.account_id().is_some() {
                    bail!("TELEGRAM_RELOGIN_REQUIRED");
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
            guard.completed = true;
            Ok(())
        }
        .await;
        if result.is_err() {
            self.set_view(&cancel, Phase::Disconnected, None, None);
        }
        let result = self.finish(&mut state, &cancel, result).await;
        attempt.completed = true;
        result
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
        let _holder = self.lock_usage.hold("submit_password");
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
            let account = save_account(&connection.session, &user).await?;
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
        let _holder = self.lock_usage.hold("list_channels");
        self.operation(id)?;
        if !matches!(self.status().phase, Phase::Ready) {
            bail!("TELEGRAM_AUTH_REQUIRED");
        }
        let result = async {
            if let Some(connection) = state.connection.as_mut().filter(|c| !c.revoked.load(Ordering::SeqCst)) {
                connection.reconnect_if_stopped(AppIdentity::load()?.id, self.proxy_url()?).await;
            }
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
        let kind = match &job { Job::Upload(_) => "upload_media", Job::Restore { .. } => "restore_media" };
        let bound = target.clone();
        self.with_transport(
            core,
            id,
            target,
            cancel,
            (kind, control),
            move |core, transport, io| {
                Box::pin(async move {
                    let journal = CoreJournal(core);
                    match job {
                        Job::Upload(mut source) => {
                            let transport = super::telegram_provider::for_upload(core, transport, &bound, &source)?;
                            transfer::prepare_video(core, &mut source, io).await?;
                            Ok(Output::Uploaded(transfer::upload(&transport, &journal, &bound, source, io).await?))
                        },
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
        (kind, control): (&'static str, Option<Arc<super::provider::TransferControl>>),
        work: impl for<'a> FnOnce(
            &'a crate::desktop_core::Core,
            &'a super::telegram_transport::GrammersTransport,
            &'a super::telegram_transfer::Io,
        ) -> super::provider::ProviderFuture<'a, T>,
    ) -> Result<T> {
        use super::{
            telegram_transfer as transfer,
            telegram_transfer_journal::{CoreJournal, Journal},
        };
        let mut binding_stop = self.backup_read_stop.subscribe();
        let mut network_stop = self.network_stop.subscribe();
        if matches!(kind, "download_media" | "restore_media" | "verify_media") {
            return self.with_media_reader(core, id, &target, cancel, (kind, control, binding_stop), work).await;
        }
        if kind == "upload_media" {
            let mut stop = self.operation(id)?;
            // File transfers may overlap. Publication, deletion and lifecycle
            // changes still take the exclusive side and drain every uploader.
            let _transaction = tokio::select! { biased;
                _ = binding_stop.changed() => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = stop.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = cancel.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = network_stop.changed() => bail!("TELEGRAM_NETWORK_CHANGED"),
                guard = self.backup_transaction.read() => guard,
            };
            return self.with_media_reader(core, id, &target, cancel, (kind, control, binding_stop), work).await;
        }
        let previous = DeleteConfig::take_for(kind);
        if *cancel.borrow() {
            bail!("TELEGRAM_TRANSFER_CANCELED");
        }
        let mut stop = self.operation(id)?;
        // Preserve the original backup snapshot / generation / publication ordering.
        // The connection state lock is independent of this business transaction.
        let _transaction = tokio::select! { biased;
            _ = binding_stop.changed() => bail!("TELEGRAM_TRANSFER_CANCELED"),
            _ = stop.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
            _ = cancel.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
            _ = network_stop.changed() => bail!("TELEGRAM_NETWORK_CHANGED"),
            guard = self.backup_transaction.write() => guard,
        };
        let mut state = {
            let wait = super::delete_timing::setup_span("session_lock");
            let _owners = wait.as_ref().map(|_| self.lock_usage.observe_wait());
            tokio::select! { biased;
                _=binding_stop.changed()=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=stop.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=cancel.wait_for(|v|*v)=>bail!("TELEGRAM_TRANSFER_CANCELED"),
                _=network_stop.changed()=>bail!("TELEGRAM_NETWORK_CHANGED"),
                state=self.state.lock()=>state,
            }
        };
        let _holder = self.lock_usage.hold(kind);
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
        let io = transfer::Io::new(cancel, stop.clone(), control)
            .with_session_stops(stop.clone(), binding_stop)
            .with_network_stop(network_stop);
        if let Some(connection) = state.connection.as_mut() {
            let _reconnect = super::delete_timing::setup_span("session_reconnect");
            connection.reconnect_if_stopped(AppIdentity::load()?.id, self.proxy_url()?).await;
        }
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
            preserve_sender: preserves_shared_sender(kind)
                || DELETE_CONFIG.try_with(|_| ()).is_ok(),
        };
        let result: Result<T> = async {
            let cached = previous.filter(|v| v.matches(id, &target, connection.generation)
                && !connection.revoked.load(Ordering::SeqCst)
                && connection.session.account_id().is_some_and(|account| target.account_id == account.to_string()));
            let (transport, premium) = prepare_transport(&connection.client, &connection.session, &target, kind, cached, &io).await?;
            super::telegram_spaces::remember_channel_title(core, &target, transport.channel_title())?;
            let result = work(core, &transport, &io).await?;
            if kind == "publish_delete" && io.check().is_ok() {
                use transfer::Transport;
                DeleteConfig::save(id, &target, connection.generation, premium, transport.max_bytes());
            }
            Ok(result)
        }
        .await;
        if let Err(error) = &result {
            if error.to_string() == "TELEGRAM_AUTH_REQUIRED" { self.authorization_failed(id).await; }
            journal.failure(&target.account_id, &error.to_string())?;
            // A stopped publication/deletion must not disconnect unrelated file leases.
            // Its journal still owns any uncertain server result.
            if !registration.preserve_sender || reset_after_transport_error(&error.to_string()) {
                connection.close().await;
            }
            self.record_transfer_error(&stop, error.to_string());
        }
        if result.is_ok() { self.clear_network_error(id); }
        registration.completed = true;
        result
    }
    /// Lease the authenticated sender for media I/O; only leasing it takes the control lock.
    async fn with_media_reader<T>(
        &self,
        core: &crate::desktop_core::Core,
        id: &str,
        target: &super::telegram_transfer::Target,
        mut cancel: watch::Receiver<bool>,
        (kind, control, mut binding_stop): (
            &'static str,
            Option<Arc<super::provider::TransferControl>>,
            watch::Receiver<()>,
        ),
        work: impl for<'a> FnOnce(
            &'a crate::desktop_core::Core,
            &'a super::telegram_transport::GrammersTransport,
            &'a super::telegram_transfer::Io,
        ) -> super::provider::ProviderFuture<'a, T>,
    ) -> Result<T> {
        use super::telegram_transfer_journal::{CoreJournal, Journal};
        let mut network_stop = self.network_stop.subscribe();
        let mut login_stop = self.operation(id)?;
        let _slot = if kind == "upload_media" {
            None // The caller owns a shared transaction lease; readers keep their own slot.
        } else {
            Some(tokio::select! { biased;
                _ = binding_stop.changed() => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = login_stop.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = cancel.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = network_stop.changed() => bail!("TELEGRAM_NETWORK_CHANGED"),
                slot = self.backup_read.lock() => slot,
            })
        };
        let (client, session, generation_stop) = {
            let mut state = tokio::select! { biased;
                _ = binding_stop.changed() => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = login_stop.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = cancel.wait_for(|v| *v) => bail!("TELEGRAM_TRANSFER_CANCELED"),
                _ = network_stop.changed() => bail!("TELEGRAM_NETWORK_CHANGED"),
                state = self.state.lock() => state,
            };
            let _holder = self.lock_usage.hold("lease_media_reader");
            let active = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
            if !std::ptr::eq(self, active.session.as_ref()) {
                bail!("TELEGRAM_STALE_OPERATION");
            }
            let view = self.status();
            if view.operation_id.as_deref() != Some(id) || !matches!(view.phase, Phase::Ready) {
                bail!("TELEGRAM_AUTH_REQUIRED");
            }
            if view.account.as_ref().map(|account| account.id.as_str()) != Some(&target.account_id) {
                bail!("TELEGRAM_ACCOUNT_MISMATCH");
            }
            let connection = state.connection.as_mut().context("TELEGRAM_NOT_CONNECTED")?;
            connection.reconnect_if_stopped(connection.api_id, self.proxy_url()?).await;
            if connection.revoked.load(Ordering::SeqCst) || connection.session.revoked() {
                bail!("TELEGRAM_AUTH_REQUIRED");
            }
            { connection.enable_media_pool(); (connection.client.clone(), connection.session.clone(), connection.reader_stop.subscribe()) }
        };
        target.validate()?;
        let journal = CoreJournal(core);
        let wait = journal.cooldown(&target.account_id)?.saturating_sub(super::telegram_transfer::now());
        if wait > 0 {
            bail!("TELEGRAM_FLOOD_WAIT:{wait}");
        }

        let io = super::telegram_transfer::Io::new(cancel, generation_stop, control)
            .with_session_stops(login_stop.clone(), binding_stop.clone())
            .with_network_stop(network_stop);
        let read = async {
            let (transport, _) = prepare_transport(&client, &session, target, kind, None, &io).await?;
            super::telegram_spaces::remember_channel_title(core, target, transport.channel_title())?;
            work(core, &transport, &io).await
        };
        let result = if kind == "upload_media" {
            // Io propagates stops into disk workers and awaits them; do not abandon
            // the whole upload future from an outer cancellation select.
            read.await
        } else {
            tokio::select! { biased;
                _ = binding_stop.changed() => Err(anyhow::anyhow!("TELEGRAM_TRANSFER_CANCELED")),
                _ = login_stop.wait_for(|v| *v) => Err(anyhow::anyhow!("TELEGRAM_TRANSFER_CANCELED")),
                result = read => result,
            }
        };
        if let Err(error) = &result {
            let message = error.to_string();
            if message == "TELEGRAM_AUTH_REQUIRED" {
                self.authorization_failed(id).await;
            }
            journal.failure(&target.account_id, &message)?;
            self.record_transfer_error(&login_stop, message);
        }
        if result.is_ok() { self.clear_network_error(id); }
        result
    }
    pub(crate) fn invalidate_backup_reads(&self) {
        self.backup_read_stop.send_replace(());
    }
    pub(crate) async fn recover_network(&self, namespace: &str, cancel: watch::Receiver<bool>) -> Result<()> {
        let authorization = Mutex::new(self.status().operation_id);
        let result = self.reconnect.run(cancel, || Box::pin(async {
            if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
            #[cfg(test)]
            if let Some(probe) = *self.reconnect_probe.lock().unwrap() { return probe(); }
            let view = Box::pin(self.resume(namespace.to_owned())).await?;
            if !matches!(view.phase, Phase::Ready) { bail!("TELEGRAM_AUTH_REQUIRED"); }
            *authorization.lock().unwrap() = view.operation_id;
            let connection = self.metadata_connection()?;
            let client = connection.client();
            let probe = tokio::time::timeout(REQUEST_TIMEOUT, Box::pin(client.get_me())).await
                .map_err(|_| anyhow::anyhow!("TELEGRAM_TIMEOUT")).and_then(|r| r.map_err(rpc_error));
            if probe.as_ref().is_err_and(|e| matches!(e.to_string().as_str(), "TELEGRAM_TIMEOUT" | "TELEGRAM_CONNECTION_ERROR")) {
                // This lease owns a generation, not whichever pool is current now.
                // Keep account keys and send evidence; resume replaces only the pool.
                connection.connection.close().await;
            }
            let user = probe?;
            account(&connection.connection.session, &user)?;
            Ok(())
        })).await;
        let id = authorization.into_inner().unwrap();
        if let Some(id) = id {
            if result.is_ok() { self.clear_network_error(&id); }
            else if result.as_ref().is_err_and(|error| error.to_string() == "TELEGRAM_AUTH_REQUIRED") {
                self.authorization_failed(&id).await;
            }
        }
        result
    }
    pub(crate) fn network_cancellation(&self) -> watch::Receiver<()> {
        self.network_stop.subscribe()
    }
    pub(crate) fn network_change_pending(&self) -> bool {
        self.network_changing.load(Ordering::SeqCst)
    }

    /// Stop only backup I/O. Shared download authorization and its sender pools remain valid.
    pub(crate) async fn cancel_backup_transfer(&self) {
        self.invalidate_backup_reads();
        let transfer = self.transfer.lock().expect("telegram transfer lock poisoned").clone();
        if let Some(cancel) = transfer {
            cancel.send_replace(true);
            let _state = self.state.lock().await; // The transfer owns the lock until request/pool cleanup.
            let _holder = self.lock_usage.hold("cancel_transfer");
        }
        let _transaction = self.backup_transaction.write().await;
        // Backup reads have a separate slot. User download pools keep running.
        let _drained = self.backup_read.lock().await;
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
        self.close_downloads().await;
        let _transaction = self.backup_transaction.write().await;
        let mut state = self.state.lock().await;
        let _holder = self.lock_usage.hold("cancel_session");
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
        self.invalidate_backup_reads();
        let transfer = self
            .transfer
            .lock()
            .expect("telegram transfer lock poisoned")
            .clone();
        if let Some(cancel) = &transfer { cancel.send_replace(true); }
        let _transaction = self.backup_transaction.write().await;
        let mut state = if let Some(cancel) = transfer {
            cancel.send_replace(true);
            self.state.lock().await
        } else {
            self.state
                .try_lock()
                .map_err(|_| anyhow::anyhow!("TELEGRAM_BUSY"))?
        };
        let _holder = self.lock_usage.hold("logout");
        self.close_downloads().await;
        let mut cancel = self.operation(id)?;
        if let Some(connection) = state.connection.as_mut() {
            if connection.stopped.load(Ordering::SeqCst) && !connection.revoked.load(Ordering::SeqCst) {
                connection.reconnect_if_stopped(AppIdentity::load()?.id, self.proxy_url()?).await;
            }
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
        control.view.phase = Phase::SignedOut;
        Ok(control.view.clone())
    }
    fn authorize_downloads(&self, state: &State, view: &View) {
        if self.closed.load(Ordering::SeqCst) || !matches!(view.phase, Phase::Ready) { return; }
        if let Some(connection) = &state.connection {
            if connection.revoked.load(Ordering::SeqCst) || connection.session.revoked() { return; }
            connection.enable_media_pool();
            let mut downloads = self.downloads.lock().expect("telegram downloads lock poisoned");
            if let Some(authorization) = downloads.as_mut().filter(|a| Arc::ptr_eq(&a.session, &connection.session)) {
                // Authorization outlives a TCP/pool generation. Keep its watch
                // alive and false; the old pool's reader_stop ends old leases.
                authorization.connections = vec![Arc::downgrade(connection)];
            } else {
                if let Some(old) = downloads.take() { old.cancel.send_replace(true); }
                *downloads = Some(DownloadAuthorization { session: connection.session.clone(), cancel: watch::channel(false).0, connections: vec![Arc::downgrade(connection)] });
            }
        }
    }
    /// File and metadata tasks lease the authorized control pool. Only its
    /// file RPCs create extra media sessions; individual tasks never own quit.
    pub(crate) fn download_connection(&self) -> Result<DownloadConnection> {
        self.register_connection()
    }
    pub(crate) fn metadata_connection(&self) -> Result<DownloadConnection> {
        self.register_connection()
    }
    fn register_connection(&self) -> Result<DownloadConnection> {
        if self.network_changing.load(Ordering::SeqCst) { bail!("TELEGRAM_NETWORK_CHANGED"); }
        if self.closed.load(Ordering::SeqCst) { bail!("TELEGRAM_CLOSED"); }
        let downloads = self.downloads.lock().expect("telegram downloads lock poisoned");
        let authorization = downloads.as_ref().context("TELEGRAM_AUTH_REQUIRED")?;
        if authorization.session.revoked() { bail!("TELEGRAM_AUTH_REQUIRED"); }
        let connection = authorization.connections.first().and_then(Weak::upgrade).filter(|c|
            !c.stopped.load(Ordering::SeqCst) && !c.revoked.load(Ordering::SeqCst)
            && c.runner.lock().expect("telegram runner").as_ref().is_some_and(|runner| !runner.is_finished())
        ).context("TELEGRAM_NOT_CONNECTED")?;
        Ok(DownloadConnection {connection, cancel: authorization.cancel.subscribe()})
    }

    #[cfg(test)]
    fn register_download(&self, _: i32) -> Result<DownloadConnection> { self.register_connection() }

    /// Invalidate only the authorization that produced a definitive RPC authentication error.
    pub(crate) async fn authorization_failed(&self, id: &str) {
        {
            let mut control = self.control.lock().expect("telegram control lock poisoned");
            if self.closed.load(Ordering::SeqCst) || control.view.operation_id.as_deref() != Some(id) || !matches!(control.view.phase, Phase::Ready) { return; }
            control.view.phase = Phase::SignedOut; control.view.account = None;
            control.view.last_error = Some("TELEGRAM_AUTH_REQUIRED".into());
        }
        if let Some(cancel) = self.transfer.lock().expect("telegram transfer lock poisoned").clone() { cancel.send_replace(true); }
        self.close_downloads().await;
    }
    async fn close_downloads(&self) {
        self.invalidate_backup_reads();
        let downloads = self.downloads.lock().expect("telegram downloads lock poisoned").take();
        if let Some(downloads) = downloads {
            downloads.cancel.send_replace(true);
            for connection in downloads.connections.into_iter().filter_map(|c| c.upgrade()) { connection.close().await; }
        }
    }
    /// Hold admission and the state lock until the new proxy is saved. Authorization
    /// remains valid; stopping a connection is distinct from revoking the account.
    pub(crate) async fn pause_network(&self) -> Result<NetworkChange<'_>> {
        if self.network_changing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            bail!("TELEGRAM_BUSY");
        }
        let mut change = NetworkChange { session: self, _state: None };
        self.network_stop.send_replace(());
        {
            let mut control = self.control.lock().expect("telegram control lock poisoned");
            let pending = !matches!(control.view.phase, Phase::Ready | Phase::Disconnected | Phase::SignedOut);
            if pending {
                control.cancel.send_replace(true);
                control.view = View::default();
            }
        }
        let downloads: Vec<_> = {
            let mut authorization = self.downloads.lock().expect("telegram downloads lock poisoned");
            authorization.as_mut().map(|a| std::mem::take(&mut a.connections)
                .into_iter().filter_map(|c| c.upgrade()).collect()).unwrap_or_default()
        };
        for connection in &downloads { connection.stop(); }
        for connection in downloads { connection.close().await; }
        let _transaction = self.backup_transaction.write().await;
        let _readers = self.backup_read.lock().await;
        let state = self.state.lock().await;
        if let Some(connection) = &state.connection { connection.close().await; }
        change._state = Some(state);
        Ok(change)
    }
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.close_downloads().await;
        self.control
            .lock()
            .expect("telegram control lock poisoned")
            .cancel
            .send_replace(true);
        let _transaction = self.backup_transaction.write().await;
        let mut state = self.state.lock().await;
        let _holder = self.lock_usage.hold("shutdown");
        if let Some(connection) = state.connection.take() {
            connection.close().await;
        }
        drop(_holder);
        drop(state);
        let _drained = self.backup_read.lock().await;
    }
}
pub(crate) struct NetworkChange<'a> {
    session: &'a TelegramSession,
    _state: Option<tokio::sync::MutexGuard<'a, State>>,
}
impl Drop for NetworkChange<'_> {
    fn drop(&mut self) { self.session.network_changing.store(false, Ordering::SeqCst); }
}
struct DownloadAuthorization {
    session: Arc<VaultSession>,
    cancel: watch::Sender<bool>,
    connections: Vec<Weak<Connection>>,
}
pub(crate) struct DownloadConnection {
    connection: Arc<Connection>,
    cancel: watch::Receiver<bool>,
}
impl DownloadConnection {
    pub(crate) fn client(&self) -> Client { self.connection.enable_media_pool(); self.connection.client.clone() }
    pub(crate) fn file_authorization(&self) -> Arc<crate::telegram_transfer_engine::Authorization> { self.connection.session.file_authorization.clone() }
    pub(crate) fn cancellation(&self) -> watch::Receiver<bool> { self.cancel.clone() }
    pub(crate) fn stopped(&self) -> watch::Receiver<bool> { self.connection.reader_stop.subscribe() }
    /// Release only this lease. The session owner stops and joins the pool.
    pub(crate) async fn close(self) {}
}
fn reset_after_transport_error(code: &str) -> bool {
    matches!(code, "TELEGRAM_CONNECTION_ERROR" | "TELEGRAM_AUTH_REQUIRED")
}

fn preserves_shared_sender(kind: &str) -> bool {
    matches!(kind, "publish_change" | "publish_delete" | "remote_cleanup" | "cancel_upload")
}

struct TransferRegistration<'a> {
    session: &'a TelegramSession,
    connection: &'a Connection,
    stop: watch::Receiver<bool>,
    completed: bool,
    preserve_sender: bool,
}
impl Drop for TransferRegistration<'_> {
    fn drop(&mut self) {
        self.session
            .transfer
            .lock()
            .expect("telegram transfer lock poisoned")
            .take();
        if !self.completed && !self.preserve_sender {
            // A caller can abandon a future. Stop its actual sender and retain the join handle for shutdown.
            self.connection.handle.quit();
            self.connection.reader_stop.send_replace(true);
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
                Phase::Disconnected,
                None,
                Some("TELEGRAM_TRANSFER_CANCELED".into()),
            );
        }
    }
}

async fn prepare_transport(
    client: &Client,
    session: &VaultSession,
    target: &super::telegram_transfer::Target,
    kind: &str,
    cached: Option<DeleteConfig>,
    io: &super::telegram_transfer::Io,
) -> Result<(super::telegram_transport::GrammersTransport, bool)> {
    use super::telegram_transport::GrammersTransport;
    let premium = if let Some(config) = &cached {
        drop(super::delete_timing::setup_span("account_check_reuse"));
        config.premium
    } else {
        let _identity = super::delete_timing::setup_span("account_check");
        let user = io.run(retry_read(|| async { client.get_me().await.map_err(rpc_error) })).await?;
        let account = save_account(session, &user).await?;
        if account.id != target.account_id {
            bail!("TELEGRAM_ACCOUNT_MISMATCH");
        }
        matches!(&user.raw, tl::enums::User::User(user) if user.premium)
    };
    let upload_limit = async {
        if let Some(config) = &cached {
            drop(super::delete_timing::setup_span("app_config_reuse"));
            return Ok(config.bytes);
        }
        let _read = super::delete_timing::setup_span("app_config");
        super::telegram_transport::fetch_upload_limit(client, premium, io).await
    };
    let transport = GrammersTransport::connect(
        client.clone(),
        target,
        session,
        kind,
        upload_limit,
        io,
    )
    .await?;
    Ok((transport, premium))
}

async fn checked_account(
    connection: &Connection,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Account> {
    let _identity = super::delete_timing::setup_span("account_check");
    let user: User = bounded(cancel, async {
        connection.client.get_me().await.map_err(rpc_error)
    })
    .await?;
    save_account(&connection.session, &user).await
}
async fn save_account(session: &VaultSession, user: &User) -> Result<Account> {
    use grammers_client::session::{types::PeerInfo, Session};
    let account = account(session, user)?;
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
    session.cache_peer(peer).await?;
    Ok(account)
}
fn account(session: &VaultSession, user: &User) -> Result<Account> {
    let id = user.id().bare_id_unchecked();
    if user.is_bot() || session.account_id().is_some_and(|old| old != id) {
        bail!("TELEGRAM_ACCOUNT_MISMATCH");
    }
    Ok(Account {
        id: id.to_string(),
        name: user.full_name(),
    })
}
/// Retry only read-only requests. Publication/deletion keep their existing journals and retry rules.
pub(crate) fn retry_read<'a, T, F, Fut>(mut read: F) -> super::provider::ProviderFuture<'a, T>
where
    F: FnMut() -> Fut + Send + 'a,
    Fut: Future<Output = Result<T>> + Send + 'a,
    T: Send + 'a,
{
    // Box the retry future itself so its SDK state cannot inflate the backup worker's stack.
    Box::pin(async move {
        for attempt in 0..3 {
            match Box::pin(read()).await {
                Err(error)
                    if attempt < 2
                        && matches!(
                            error.to_string().as_str(),
                            "TELEGRAM_CONNECTION_ERROR" | "TELEGRAM_NOT_CONNECTED"
                        ) =>
                {
                    eprintln!("telegram_read event=retry attempt={}", attempt + 1);
                    tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
                }
                result => return result,
            }
        }
        unreachable!("bounded read retry")
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
pub(crate) fn rpc_error(error: InvocationError) -> anyhow::Error {
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
                | "AUTH_KEY_DUPLICATED"
                | "SESSION_REVOKED"
                | "SESSION_EXPIRED"
                | "USER_DEACTIVATED" => "TELEGRAM_AUTH_REQUIRED",
                "CHAT_ADMIN_REQUIRED" | "CHANNEL_PRIVATE" | "CHAT_WRITE_FORBIDDEN" | "CHAT_DELETE_FORBIDDEN" => {
                    "TELEGRAM_CHANNEL_FORBIDDEN"
                }
                "FILE_PART_MISSING" => "TELEGRAM_UPLOAD_PART_MISSING",
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
    async fn reconnect_feedback_clears_only_recovered_network_errors() {
        for code in ["TELEGRAM_TRANSFER_TIMEOUT", "TELEGRAM_CHANNEL_FORBIDDEN", "TELEGRAM_INDEX_CHANGED"] {
            let session = TelegramSession::default();
            session.set_test_view(Phase::Ready, "fixture");
            let id = session.status().operation_id.unwrap();
            session.control.lock().unwrap().view.last_error = Some(code.into());
            *session.reconnect_probe.lock().unwrap() = Some(|| Ok(()));
            let (stop, cancel) = watch::channel(false);
            let work = session.recover_network("unused", cancel);
            tokio::pin!(work);
            tokio::select! {
                result = &mut work => panic!("probe returned without backoff: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {},
            }
            let expected = if crate::telegram_reconnect::network_error(code) { "TELEGRAM_RECONNECTING" } else { code };
            assert_eq!(session.status().last_error.as_deref(), Some(expected));
            work.await.unwrap();
            assert_eq!(session.status().last_error.as_deref(), (!crate::telegram_reconnect::network_error(code)).then_some(code));
            session.control.lock().unwrap().view.last_error = Some("TELEGRAM_TRANSFER_TIMEOUT".into());
            session.clear_network_error("retired-operation");
            assert_eq!(session.status().last_error.as_deref(), Some("TELEGRAM_TRANSFER_TIMEOUT"));
            session.clear_network_error(&id);
            assert!(session.status().last_error.is_none());
            drop(stop);
        }
    }

    #[tokio::test]
    async fn reconnect_auth_failure_invalidates_shared_authorization() {
        let session = TelegramSession::default();
        session.set_test_view(Phase::Ready, "fixture");
        *session.reconnect_probe.lock().unwrap() = Some(|| Err(anyhow::anyhow!("TELEGRAM_AUTH_REQUIRED")));
        let (_stop, cancel) = watch::channel(false);
        assert_eq!(session.recover_network("unused", cancel).await.unwrap_err().to_string(), "TELEGRAM_AUTH_REQUIRED");
        assert!(matches!(session.status().phase, Phase::SignedOut));
        assert!(session.status().account.is_none());
    }

    #[tokio::test]
    async fn telegram_read_retries_recover_and_stop_at_three_attempts() {
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let value = retry_read(|| async {
            if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            Ok(42)
        })
        .await
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        attempts.store(0, Ordering::SeqCst);
        let error = retry_read(|| async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "TELEGRAM_CONNECTION_ERROR");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
    #[tokio::test]
    async fn telegram_read_retries_preserve_non_network_errors_and_cancellation() {
        for code in [
            "TELEGRAM_AUTH_REQUIRED",
            "TELEGRAM_CHANNEL_FORBIDDEN",
            "TELEGRAM_FLOOD_WAIT:3",
            "TELEGRAM_MEDIA_CHANGED",
            "TELEGRAM_CANCELED",
            "TELEGRAM_TIMEOUT",
        ] {
            let attempts = std::sync::atomic::AtomicUsize::new(0);
            let error = retry_read(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(anyhow::anyhow!(code.to_owned()))
            })
            .await
            .unwrap_err();
            assert_eq!(error.to_string(), code);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let canceled = tokio::time::timeout(
            Duration::from_millis(20),
            retry_read(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR"))
            }),
        )
        .await;
        assert!(canceled.is_err());
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn missing_upload_part_has_a_definitive_fixed_error_code() {
        let error=InvocationError::Rpc(tl::types::RpcError {error_code:400,error_message:"FILE_PART_3_MISSING".into()}.into());
        assert_eq!(rpc_error(error).to_string(),"TELEGRAM_UPLOAD_PART_MISSING");
    }
    #[tokio::test]
    async fn backup_business_errors_preserve_authorization_and_do_not_revive_signed_out_state() {
        let session = TelegramSession::default(); let cancel = session.start_operation().unwrap();
        session.set_view(&cancel, Phase::Ready, Some(Account{id:"123".into(),name:"test".into()}), None);
        for code in ["CANCELED", "TELEGRAM_TRANSFER_CANCELED", "TELEGRAM_NETWORK_CHANGED", "TELEGRAM_MEDIA_MISSING", "TELEGRAM_CHANNEL_FORBIDDEN", "TELEGRAM_FILE_TOO_LARGE", "TELEGRAM_INDEX_INVALID"] {
            session.record_transfer_error(&cancel, code.into());
            assert!(matches!(session.status().phase, Phase::Ready));
            assert_eq!(session.status().account.unwrap().id, "123");
            assert!(session.status().last_error.is_none());
        }
        session.record_transfer_error(&cancel, "TELEGRAM_CONNECTION_ERROR".into());
        session.record_transfer_error(&cancel, "CANCELED".into());
        assert_eq!(session.status().last_error.as_deref(), Some("TELEGRAM_CONNECTION_ERROR"));
        session.set_view(&cancel, Phase::SignedOut, None, Some("TELEGRAM_AUTH_REQUIRED".into()));
        session.record_transfer_error(&cancel, "TELEGRAM_CONNECTION_ERROR".into());
        assert_eq!(session.status().last_error.as_deref(), Some("TELEGRAM_AUTH_REQUIRED"));
        assert!(matches!(session.status().phase, Phase::SignedOut));
        assert!(session.status().account.is_none());
        session.shutdown().await;
    }
    #[tokio::test]
    async fn definitive_auth_failure_clears_account_but_network_and_password_errors_keep_their_phase() {
        for (message, phase, ready) in [("TELEGRAM_AUTH_REQUIRED", Phase::Ready, false), ("TELEGRAM_RELOGIN_REQUIRED", Phase::Ready, false), ("TELEGRAM_CONNECTION_ERROR", Phase::Ready, true), ("TELEGRAM_INVALID_PASSWORD", Phase::Password, false)] {
            let session = TelegramSession::default();
            let cancel = session.start_operation().unwrap();
            session.set_view(&cancel, phase.clone(), Some(Account{id:"123".into(),name:"test".into()}), None);
            let mut state = session.state.lock().await;
            assert!(session.finish(&mut state, &cancel, Err(anyhow::anyhow!(message))).await.is_err());
            let view = session.status();
            assert_eq!(matches!(view.phase, Phase::Ready), ready);
            if message == "TELEGRAM_AUTH_REQUIRED" || message == "TELEGRAM_RELOGIN_REQUIRED" { assert!(view.account.is_none()); assert!(matches!(view.phase, Phase::SignedOut)); }
            if message == "TELEGRAM_INVALID_PASSWORD" { assert!(matches!(view.phase, Phase::Password)); }
            drop(state); session.shutdown().await;
        }
    }
    #[tokio::test]
    async fn unverified_restore_failures_never_request_a_new_login() {
        for message in ["TELEGRAM_CONNECTION_ERROR", "TELEGRAM_TIMEOUT", "TELEGRAM_SESSION_STORAGE"] {
            let session = TelegramSession::default();
            assert!(matches!(session.status().phase, Phase::Disconnected));
            let token = session.start_operation().unwrap();
            // A failed connection attempt has no evidence that the saved key is invalid.
            session.set_view(&token, Phase::Disconnected, None, None);
            let mut state = session.state.lock().await;
            assert!(session.finish(&mut state, &token, Err(anyhow::anyhow!(message))).await.is_err());
            let view = session.status();
            assert!(matches!(view.phase, Phase::Disconnected));
            assert_eq!(view.last_error.as_deref(), Some(message));
            assert!(view.qr.is_none());
            drop(state);
            session.shutdown().await;
        }
    }
    #[tokio::test]
    async fn successful_remote_logout_with_failed_local_erase_is_signed_out_and_cleanup_can_retry() {
        struct EraseVault(std::sync::atomic::AtomicBool);
        impl super::super::telegram_vault::SessionVault for EraseVault {
            fn read(&self, _: &str) -> Result<Option<zeroize::Zeroizing<String>>> { Ok(None) }
            fn write(&self, _: &str, _: &str) -> Result<()> { Ok(()) }
            fn delete(&self, _: &str) -> Result<()> { if self.0.load(Ordering::SeqCst) { bail!("QA_ERASE_FAILED"); } Ok(()) }
        }
        let vault = Arc::new(EraseVault(std::sync::atomic::AtomicBool::new(true)));
        let keys = VaultSession::load("offline-erase-retry".into(), vault.clone()).await.unwrap();
        let connection = Connection::start(keys, 12, None);
        connection.revoked.store(true, Ordering::SeqCst); // Remote sign-out already succeeded.
        let session = TelegramSession::default(); let cancel = session.start_operation().unwrap();
        session.set_view(&cancel, Phase::Ready, Some(Account{id:"123".into(),name:"test".into()}), None);
        session.state.lock().await.connection = Some(connection);
        let id = session.status().operation_id.unwrap();
        assert!(session.logout(&id).await.is_err());
        assert!(matches!(session.status().phase, Phase::SignedOut));
        assert!(session.status().account.is_none());
        assert!(session.state.lock().await.connection.is_some());
        assert!(session.has_operation(&id));
        vault.0.store(false, Ordering::SeqCst);
        session.logout(&id).await.unwrap();
        assert!(session.state.lock().await.connection.is_none());
        assert!(matches!(session.status().phase, Phase::SignedOut));
        session.shutdown().await;
    }
    #[tokio::test]
    async fn download_auth_failure_ignores_old_operations_and_clears_the_current_account() {
        let session = TelegramSession::default(); session.set_test_view(Phase::Ready, "account-a");
        let id = session.status().operation_id.unwrap();
        session.authorization_failed("old-operation").await;
        assert!(matches!(session.status().phase, Phase::Ready));
        session.authorization_failed(&id).await;
        assert!(matches!(session.status().phase, Phase::SignedOut));
        assert!(session.status().account.is_none());
        assert_eq!(session.status().last_error.as_deref(), Some("TELEGRAM_AUTH_REQUIRED"));
        assert!(session.has_operation(&id), "authorization loss must leave local cleanup retryable");
        let keys = VaultSession::load("offline-auth-cleanup".into(), Arc::new(DownloadVault)).await.unwrap();
        let connection = Connection::start(keys, 12, None);
        connection.revoked.store(true, Ordering::SeqCst);
        session.state.lock().await.connection = Some(connection);
        session.logout(&id).await.unwrap();
        assert!(session.state.lock().await.connection.is_none());
        session.shutdown().await;
    }
    #[tokio::test]
    async fn backup_cancellation_preserves_shared_authorization_and_waits_for_cleanup() {
        let session = Arc::new(TelegramSession::default());
        session.set_test_view(Phase::Ready, "account-a");
        let (abort, receive) = watch::channel(false);
        *session.transfer.lock().unwrap() = Some(abort);
        let locked = session.state.lock().await;
        let stopping = tokio::spawn({ let session = session.clone(); async move { session.cancel_backup_transfer().await; } });
        tokio::time::timeout(Duration::from_secs(1), async { while !*receive.borrow() { tokio::task::yield_now().await; } }).await.unwrap();
        assert!(!stopping.is_finished());
        assert_eq!(session.status().account.unwrap().id, "account-a");
        assert!(!*session.control.lock().unwrap().cancel.borrow());
        drop(locked);
        stopping.await.unwrap();
        assert!(matches!(session.status().phase, Phase::Ready));
        session.shutdown().await;
    }
    struct DownloadVault;
    impl super::super::telegram_vault::SessionVault for DownloadVault {
        fn read(&self, _: &str) -> Result<Option<zeroize::Zeroizing<String>>> { Ok(None) }
        fn write(&self, _: &str, _: &str) -> Result<()> { Ok(()) }
        fn delete(&self, _: &str) -> Result<()> { Ok(()) }
    }
    async fn fixture_control(session: &TelegramSession, keys: Arc<VaultSession>) -> Arc<Connection> {
        let connection = Connection::start(keys, 12, session.proxy_url().unwrap());
        let mut state = session.state.lock().await;
        state.connection = Some(connection.clone());
        session.set_test_view(Phase::Ready, "123");
        session.authorize_downloads(&state, &session.status());
        connection
    }
    #[tokio::test]
    async fn dropped_restore_resets_connecting_before_and_after_pool_creation() {
        for with_pool in [false,true] {
            let session = TelegramSession::default();
            let keys = VaultSession::load("offline-restore-state".into(),Arc::new(DownloadVault)).await.unwrap();
            let mut restore = Box::pin(async {
                let mut state = session.state.lock().await;
                let cancel = session.start_operation().unwrap();
                let _attempt = AuthorizationAttempt { session:&session, cancel, completed:false };
                if with_pool { state.connection = Some(Connection::start(keys, 12, None)); }
                let _pool = state.connection.as_ref().map(|connection| ConnectingPool { connection, completed:false });
                std::future::pending::<()>().await;
            });
            assert!(tokio::time::timeout(Duration::from_millis(10), &mut restore).await.is_err());
            assert!(matches!(session.status().phase,Phase::Connecting));
            drop(restore);
            assert!(matches!(session.status().phase,Phase::Disconnected));
            assert_eq!(session.status().last_error.as_deref(),Some("TELEGRAM_CANCELED"));
            let token = session.start_operation().unwrap();
            assert!(!*token.borrow());
            session.shutdown().await;
        }
    }
    #[tokio::test]
    async fn metadata_and_downloads_lease_one_control_pool_without_owning_shutdown() {
        let keys = VaultSession::load("offline-metadata-reuse".into(), Arc::new(DownloadVault)).await.unwrap();
        let session = TelegramSession::default();
        let control = fixture_control(&session, keys).await;
        let mut leases = Vec::new();
        for _ in 0..8 {
            let lease = session.register_connection().unwrap();
            assert!(Arc::ptr_eq(&lease.connection, &control));
            leases.push(lease);
        }
        assert_eq!(session.downloads.lock().unwrap().as_ref().unwrap().connections.len(), 1);
        leases.pop().unwrap().close().await;
        assert!(!control.stopped.load(Ordering::SeqCst));
        assert!(!control.runner.lock().unwrap().as_ref().unwrap().is_finished());
        let cancel = leases[0].cancellation();
        session.close_downloads().await;
        assert!(*cancel.borrow());
        assert!(control.runner.lock().unwrap().is_none());
        for lease in leases { assert!(*lease.stopped().borrow()); lease.close().await; }
        session.shutdown().await;
    }

    #[tokio::test]
    async fn dead_main_pool_is_replaced_once_and_old_generation_leases_stop() {
        let keys = VaultSession::load("offline-metadata-finished".into(), Arc::new(DownloadVault)).await.unwrap();
        let session = TelegramSession::default();
        let control = fixture_control(&session, keys).await;
        let lease = session.register_connection().unwrap();
        control.runner.lock().unwrap().as_ref().unwrap().abort();
        while !control.runner.lock().unwrap().as_ref().unwrap().is_finished() {tokio::task::yield_now().await;}
        assert_eq!(session.register_connection().err().unwrap().to_string(), "TELEGRAM_NOT_CONNECTED");
        session.resume("unused".into()).await.unwrap();
        let next = session.register_connection().unwrap();
        assert_ne!(next.connection.generation, control.generation);
        assert!(*lease.stopped().borrow());
        assert!(!*lease.cancellation().borrow(), "transport replacement is not an authorization failure");
        let mut auth = lease.cancellation();
        let mut stopped = lease.stopped();
        let reason = tokio::select! { biased;
            _ = auth.wait_for(|v| *v) => "TELEGRAM_AUTH_REQUIRED",
            _ = stopped.wait_for(|v| *v) => "TELEGRAM_CONNECTION_ERROR",
        };
        // Follow the consumer's auth-failure callback to catch revocation of a
        // newly rebuilt pool by an older generation's wakeup.
        if reason == "TELEGRAM_AUTH_REQUIRED" {
            session.authorization_failed(session.status().operation_id.as_deref().unwrap()).await;
        }
        assert_eq!(reason, "TELEGRAM_CONNECTION_ERROR");
        assert!(matches!(session.status().phase, Phase::Ready));
        assert!(Arc::ptr_eq(&next.connection, session.state.lock().await.connection.as_ref().unwrap()));
        lease.close().await;next.close().await;session.shutdown().await;
    }

    #[tokio::test]
    async fn stopped_main_pool_never_creates_a_per_metadata_fallback() {
        let keys = VaultSession::load("offline-metadata-fallback".into(), Arc::new(DownloadVault)).await.unwrap();
        let session = TelegramSession::default();
        let control = fixture_control(&session, keys).await;
        control.close().await;
        assert_eq!(session.register_connection().err().unwrap().to_string(), "TELEGRAM_NOT_CONNECTED");
        session.resume("unused".into()).await.unwrap();
        let next = session.register_connection().unwrap();
        assert_ne!(next.connection.generation, control.generation);
        next.close().await;
        assert!(!session.state.lock().await.connection.as_ref().unwrap().stopped.load(Ordering::SeqCst));
        session.shutdown().await;
    }

    #[tokio::test]
    async fn dropped_publication_preserves_the_main_pool_and_unrelated_file_leases() {
        let keys = VaultSession::load("offline-publish-cancel".into(), Arc::new(DownloadVault)).await.unwrap();
        let session = TelegramSession::default();
        let control = fixture_control(&session, keys).await;
        let lease = session.register_connection().unwrap();
        let stop = session.operation(session.status().operation_id.as_deref().unwrap()).unwrap();
        drop(TransferRegistration {session: &session, connection: &control, stop, completed: false, preserve_sender: preserves_shared_sender("publish_change")});
        assert!(!control.stopped.load(Ordering::SeqCst));
        assert!(!*lease.stopped().borrow());
        assert!(matches!(session.status().phase, Phase::Ready));
        lease.close().await;session.shutdown().await;
    }
    #[tokio::test]
    async fn releasing_one_file_lease_keeps_other_transfers_and_the_main_pool_alive() {
        let keys = VaultSession::load("offline-downloads".into(), Arc::new(DownloadVault)).await.unwrap();
        let session = TelegramSession::default();
        let control = fixture_control(&session, keys).await;
        let a = session.register_download(12).unwrap();
        let b = session.register_download(12).unwrap();
        assert!(Arc::ptr_eq(&a.connection, &b.connection));
        a.close().await;
        assert!(!*b.stopped().borrow());assert!(!control.stopped.load(Ordering::SeqCst));
        let cancel = b.cancellation();
        session.close_downloads().await;
        assert!(*cancel.borrow());assert!(*b.stopped().borrow());
        assert!(control.runner.lock().unwrap().is_none());
        assert!(session.register_download(12).is_err());
        b.close().await;session.shutdown().await;
    }

    #[tokio::test]
    async fn backup_reads_multiplex_the_owned_sender_and_switch_drains_only_readers() {
        media_lease_releases_control_and_binding_switch_drains("download_media", false).await;
    }
    #[tokio::test]
    async fn parallel_backup_uploads_cancel_independently_and_drain_before_binding_switch() {
        media_lease_releases_control_and_binding_switch_drains("upload_media", false).await;
    }
    #[tokio::test]
    async fn proxy_save_closes_upload_and_download_pools_without_revoking_authorization() {
        media_lease_releases_control_and_binding_switch_drains("upload_media", true).await;
    }
    #[tokio::test]
    async fn abandoned_proxy_pause_clears_canceled_pending_login_and_releases_admission() {
        let session = TelegramSession::default();
        session.set_test_view(Phase::Connecting, "123");
        let state = session.state.lock().await;
        let mut pause = Box::pin(session.pause_network());
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut pause).await.is_err());
        assert!(matches!(session.status().phase, Phase::Disconnected));
        assert!(session.network_changing.load(Ordering::SeqCst));
        drop(pause);
        assert!(!session.network_changing.load(Ordering::SeqCst));
        drop(state);
        session.start_operation().unwrap();
        session.shutdown().await;
    }
    async fn media_lease_releases_control_and_binding_switch_drains(kind: &'static str, proxy_change: bool) {
        use super::super::{provider::{ProviderType, Scope}, telegram_transfer::Target};
        use crate::{desktop_core::Core, telegram_accounts::Purpose};

        let root = tempfile::tempdir().unwrap();
        let core = Core::open(&root.path().join("qa.sqlite"), root.path().join("downloads"),
            "unused-engine".into(), vec![], |_| {}).unwrap();
        // Stall a real SDK request at a local SOCKS server. No Telegram network or login is used.
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        core.save_profile(crate::settings::ProfileUpdate {
            platform: "telegram".into(), user_agent: "test".into(), cookie: None,
            proxy: Some(format!("socks5://{}", proxy.local_addr().unwrap())), http_chunk_size: None,
        }).unwrap();
        let session = core.telegram_accounts.binding(Purpose::Backup).session;
        let keys = VaultSession::load("offline-media-read".into(), Arc::new(DownloadVault)).await.unwrap();
        let _control = fixture_control(&session, keys.clone()).await;
        session.set_test_view(Phase::Ready, "123");
        let login = session.status().operation_id.unwrap();
        let target = Target {
            scope: Scope { space_id:"space".into(), library_id:"library".into(), provider_type:ProviderType::Telegram, protocol:2 },
            account_id:"123".into(), channel_id:"456".into(),
        };
        let (cancel, rx) = watch::channel(false);
        let independent = session.register_download(12).unwrap();
        let run = |kind, rx| session.with_transport(&core, &login, target.clone(), rx, (kind, None),
            |_, _, _| Box::pin(async { Err::<(), _>(anyhow::anyhow!("unexpected transport")) }));
        let mut active = Box::pin(run(kind, rx));
        let socket = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::select! { result = &mut active => panic!("read ended before proxy handshake: {result:?}"), accepted = proxy.accept() => accepted.unwrap().0 }
        }).await.unwrap();
        let control = session.state.try_lock().expect("control must be free during media I/O");
        if kind == "upload_media" {
            assert!(session.backup_transaction.try_write().is_err(), "publication remains exclusive while uploads run");
            assert!(session.backup_read.try_lock().is_ok(), "upload cannot occupy the reader slot");
        }
        if proxy_change {
            let update = |proxy, user_agent: &str| crate::settings::ProfileUpdate {
                platform: "telegram".into(), user_agent: user_agent.into(), cookie: None,
                proxy: Some(proxy), http_chunk_size: None,
            };
            tokio::time::timeout(Duration::from_secs(1), core.save_profile_with_network(
                update(format!("socks5://{}", proxy.local_addr().unwrap()), "same proxy"),
            )).await.unwrap().unwrap();
            for invalid in [update("http://127.0.0.1:9".into(), "test"), update("socks5://127.0.0.1:9".into(), "")] {
                assert_eq!(core.save_profile_with_network(invalid).await.err().unwrap().to_string(), "CONFIG_INVALID");
            }
            assert!(!control.connection.as_ref().unwrap().stopped.load(Ordering::SeqCst),
                    "unchanged or invalid proxy cannot interrupt a transfer");
        }
        let (_waiting_cancel, waiting_rx) = watch::channel(false);
        let mut waiting = Box::pin(run(if kind == "upload_media" { "upload_media" } else { "restore_media" }, waiting_rx));
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut waiting).await.is_err());
        assert_eq!(session.downloads.lock().unwrap().as_ref().unwrap().connections.len(), 1,
                   "only the unrelated user download owns a separate pool");
        let first_finished = if kind == "upload_media" && !proxy_change {
            cancel.send_replace(true);
            let result = tokio::time::timeout(Duration::from_secs(1), &mut active).await.unwrap();
            assert_eq!(result.as_ref().unwrap_err().to_string(), "TELEGRAM_TRANSFER_CANCELED");
            // The second future has not been polled again. Its shared lease must
            // already be held, rather than waiting behind the first upload.
            assert!(session.backup_transaction.try_write().is_err(), "second uploader never acquired a concurrent lease");
            Some(result)
        } else { None };
        drop(control);
        let stopping = async {
            if proxy_change {
                core.save_profile_with_network(crate::settings::ProfileUpdate {
                    platform: "telegram".into(), user_agent: "test".into(), cookie: None,
                    proxy: Some("socks5://127.0.0.1:9".into()), http_chunk_size: None,
                }).await.unwrap();
            } else { session.cancel_backup_transfer().await; }
        };
        let first = async { match first_finished { Some(result) => result, None => active.await } };
        let ((), first, second) = tokio::join!(stopping, first, waiting);
        drop(socket);
        for result in [first, second] {
            assert_eq!(result.unwrap_err().to_string(), if proxy_change { "TELEGRAM_NETWORK_CHANGED" } else { "TELEGRAM_TRANSFER_CANCELED" });
        }
        assert!(session.backup_transaction.try_write().is_ok());
        assert!(!*independent.cancellation().borrow());
        let state = session.state.lock().await;
        assert_eq!(independent.connection.stopped.load(Ordering::SeqCst), proxy_change);
        assert_eq!(state.connection.as_ref().unwrap().stopped.load(Ordering::SeqCst), proxy_change);
        drop(state);
        if proxy_change {
            assert!(independent.connection.runner.lock().unwrap().is_none(), "old pool was joined before save returned");
            assert!(matches!(session.status().phase, Phase::Ready));
            let pause = session.pause_network().await.unwrap();
            assert_eq!(session.register_download(12).err().unwrap().to_string(), "TELEGRAM_NETWORK_CHANGED");
            drop(pause);
            session.resume("unused".into()).await.unwrap();
            let replacement = session.register_download(12).unwrap();
            assert_eq!(replacement.connection.proxy_url.as_deref(), Some("socks5://127.0.0.1:9/"));
            assert!(!*replacement.cancellation().borrow());
            replacement.close().await;
        }
        independent.close().await;
        core.shutdown().await;
    }

    #[tokio::test]
    async fn yielding_canceled_upload_keeps_an_active_reader_and_login_alive() {
        let keys = VaultSession::load("offline-cleanup-yield".into(), Arc::new(DownloadVault))
            .await
            .unwrap();
        let connection = Connection::start(keys, 12, None);
        let session = TelegramSession::default();
        let stop = session.start_operation().unwrap();
        session.set_view(
            &stop,
            Phase::Ready,
            Some(Account {
                id: "123".into(),
                name: "offline".into(),
            }),
            None,
        );
        let reader = connection.reader_stop.subscribe();
        let (_cancel, cancel) = watch::channel(false);
        let io = super::super::telegram_transfer::Io::new(cancel, reader.clone(), None);
        let reading = io.run(std::future::pending::<Result<()>>());
        tokio::pin!(reading);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut reading)
                .await
                .is_err()
        );
        assert!(preserves_shared_sender("cancel_upload"));
        assert!(!preserves_shared_sender("upload_media"));
        drop(TransferRegistration {
            session: &session,
            connection: &connection,
            stop,
            completed: false,
            preserve_sender: preserves_shared_sender("cancel_upload"),
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut reading)
                .await
                .is_err(),
            "yield canceled an independent reader"
        );
        assert!(!*reader.borrow() && !connection.stopped.load(Ordering::SeqCst));
        assert!(!connection
            .runner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_finished());
        assert!(matches!(session.status().phase, Phase::Ready));
        assert_eq!(session.status().account.unwrap().id, "123");
        connection.close().await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut reading)
                .await
                .unwrap()
                .unwrap_err()
                .to_string(),
            "TELEGRAM_TRANSFER_CANCELED"
        );
    }
    #[tokio::test]
    async fn delete_business_error_and_abandoned_delete_keep_the_sender_for_readers() {
        let keys = VaultSession::load("offline-delete-abandon".into(), Arc::new(DownloadVault)).await.unwrap();
        let connection = Connection::start(keys, 12, None);
        let session = TelegramSession::default();
        let stop = session.start_operation().unwrap();
        let reader = connection.reader_stop.subscribe();
        for error in ["TELEGRAM_MEDIA_CHANGED", "BACKUP_TOMBSTONE_REQUIRED", "TELEGRAM_TRANSFER_CANCELED", "TELEGRAM_TRANSFER_TIMEOUT"] {
            assert!(!reset_after_transport_error(error));
        }
        drop(TransferRegistration { session:&session, connection:&connection, stop, completed:false, preserve_sender:true });
        assert!(!*reader.borrow() && !connection.stopped.load(Ordering::SeqCst));
        assert!(reset_after_transport_error("TELEGRAM_CONNECTION_ERROR"));
        connection.close().await;
        assert!(*reader.borrow());
    }
    #[tokio::test]
    async fn abandoned_login_restore_stops_its_pool_before_reconnect() {
        let keys = VaultSession::load("offline-connect-cancel".into(), Arc::new(DownloadVault)).await.unwrap();
        let mut connection = Connection::start(keys, 12, None);
        let runner = connection.runner.lock().unwrap().as_ref().unwrap().abort_handle();
        drop(ConnectingPool { connection:&connection, completed:false });
        assert!(connection.stopped.load(Ordering::SeqCst));
        connection.reconnect_if_stopped(12, None).await;
        assert!(runner.is_finished());
        connection.close().await;
    }
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
        let keys = VaultSession::load("offline-ready-reuse".into(), Arc::new(DownloadVault)).await.unwrap();
        let _control = fixture_control(&session, keys).await;
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
        let mut connection = Connection::start(keys.clone(), 12, None);
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
            preserve_sender: false,
        });
        assert!(connection.stopped.load(Ordering::SeqCst));
        // This is the exact reconnect path used by logout, without issuing a real sign-out RPC.
        let previous_generation = connection.generation;
        connection.reconnect_if_stopped(12, None).await;
        assert_ne!(connection.generation, previous_generation);
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
    #[tokio::test]
    async fn delete_config_only_matches_one_cleanup_for_the_same_login_target_and_connection() {
        use super::super::{provider::{ProviderType, Scope}, telegram_transfer::Target};
        let target = Target {
            scope: Scope { space_id: "space".into(), library_id: "library".into(), provider_type: ProviderType::Telegram, protocol: 2 },
            account_id: "100".into(), channel_id: "200".into(),
        };
        let publish = || DeleteConfig::save("login", &target, 1, true, 123);
        with_delete_config(async {
            publish();
            let evidence = DeleteConfig::take_for("remote_cleanup").unwrap();
            assert!(evidence.matches("login", &target, 1));
            assert!(evidence.premium);
            assert_eq!(evidence.bytes, 123);
            assert!(!evidence.matches("next-login", &target, 1));
            assert!(!evidence.matches("login", &target, 2));
            for field in ["account", "channel", "space", "library"] {
                let mut other = target.clone();
                match field {
                    "account" => other.account_id.push('1'),
                    "channel" => other.channel_id.push('1'),
                    "space" => other.scope.space_id.push('1'),
                    _ => other.scope.library_id.push('1'),
                }
                assert!(!evidence.matches("login", &other, 1));
            }
            assert!(DeleteConfig::take_for("remote_cleanup").is_none());
            publish();
            assert!(DeleteConfig::take_for("publish_change").is_none());
            // A failed replacement publication cannot leave older successful evidence behind.
            assert!(DeleteConfig::take_for("remote_cleanup").is_none());
        }).await;
        // Drop an in-flight deletion after publication. The next attempt gets a new scope.
        {
            let pending = with_delete_config(async {
                publish();
                std::future::pending::<()>().await;
            });
            tokio::pin!(pending);
            tokio::select! { biased; _ = &mut pending => unreachable!(), _ = std::future::ready(()) => {} }
        }
        with_delete_config(async {assert!(DeleteConfig::take_for("remote_cleanup").is_none());}).await;
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
    #[tokio::test]
    async fn telegram_proxy_routes_login_and_download_pools_and_reconnects_after_change() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("socks5://{}", listener.local_addr().unwrap());
        let profile = serde_json::from_value(serde_json::json!({
            "platform":"telegram", "user_agent":"test", "proxy":address,
            "cookie_configured":false, "cookie":"", "credential_ref":null
        })).unwrap();
        let session = TelegramSession::with_proxy(Arc::new(Mutex::new(Some(profile))));
        let keys = VaultSession::load("offline-proxy".into(), Arc::new(DownloadVault)).await.unwrap();
        let login = fixture_control(&session, keys).await;
        let download = session.register_download(12).unwrap();
        for connection in [&login, download.connection.as_ref()] {
            let observed = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut greeting = [0; 2]; stream.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting[0], 5);
                let mut methods = vec![0; greeting[1] as usize]; stream.read_exact(&mut methods).await.unwrap();
                assert!(methods.contains(&0)); stream.write_all(&[5, 0]).await.unwrap();
                let mut target = [0; 4]; stream.read_exact(&mut target).await.unwrap();
                assert_eq!(target[..3], [5, 1, 0]); // SOCKS CONNECT is sent before any MTProto request.
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                let request = connection.client.get_me();
                tokio::pin!(request);
                tokio::select! { biased; _ = observed => {}, _ = &mut request => panic!("request bypassed local proxy") }
                // Both callers share one pool. Wait for the rejected proxy
                // attempt to finish before requiring a new socket next round.
                assert!(request.await.is_err());
            }).await.unwrap();
        }
        let old_runner = login.runner.lock().unwrap().as_ref().unwrap().abort_handle();
        let pause = session.pause_network().await.unwrap();
        assert!(old_runner.is_finished());
        session.proxy.lock().unwrap().as_mut().unwrap().proxy = None;
        drop(pause);
        session.resume("unused".into()).await.unwrap();
        let next = session.register_download(12).unwrap();
        assert!(next.connection.proxy_url.is_none());
        assert_ne!(next.connection.generation, download.connection.generation);
        assert!(*download.stopped().borrow());
        next.close().await; download.close().await; login.close().await; session.shutdown().await;
    }

    #[tokio::test]
    async fn telegram_proxy_decodes_username_and_password_before_socks_authentication() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("socks5://user%40example%2540:p%40ss%3A%25@{}",listener.local_addr().unwrap());
        let keys = VaultSession::load("offline-proxy-auth".into(),Arc::new(DownloadVault)).await.unwrap();
        let connection = Connection::start(keys,12,Some(proxy));
        let observed = async {
            let (mut stream,_) = listener.accept().await.unwrap();
            let mut greeting=[0;2];stream.read_exact(&mut greeting).await.unwrap();assert_eq!(greeting[0],5);
            let mut methods=vec![0;greeting[1] as usize];stream.read_exact(&mut methods).await.unwrap();assert!(methods.contains(&2));stream.write_all(&[5,2]).await.unwrap();
            let mut header=[0;2];stream.read_exact(&mut header).await.unwrap();assert_eq!(header[0],1);
            let mut username=vec![0;header[1] as usize];stream.read_exact(&mut username).await.unwrap();
            let len=stream.read_u8().await.unwrap();let mut password=vec![0;len as usize];stream.read_exact(&mut password).await.unwrap();
            assert_eq!(username,b"user@example%40");assert_eq!(password,b"p@ss:%");
            stream.write_all(&[1,0]).await.unwrap();
            let mut target=[0;3];stream.read_exact(&mut target).await.unwrap();assert_eq!(target,[5,1,0]);
        };
        tokio::time::timeout(Duration::from_secs(3),async {tokio::select! {_=observed=>{},_=connection.client.get_me()=>panic!("SOCKS authentication failed")}}).await.unwrap();
        connection.close().await;
    }

}
