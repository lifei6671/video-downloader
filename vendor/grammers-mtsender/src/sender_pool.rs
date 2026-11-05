// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::ops::{ControlFlow, Deref};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, panic};

use grammers_mtproto::{mtp, transport};
use grammers_session::Session;
use grammers_session::storages::{ErasedSession, erase};
use grammers_session::types::DcOption;
use grammers_session::updates::UpdatesLike;
use grammers_tl_types::{self as tl, Deserializable, Serializable, Identifiable, enums};
use tokio::task::AbortHandle;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::sleep,
};
use tracing::{debug, info, warn};

use crate::configuration::{ConnectionParams, RetryContext};
use crate::errors::{InvocationError, ReadError};
use crate::{Sender, ServerAddr, connect, connect_with_auth};

#[path = "media_recovery.rs"]
mod media_recovery;
use media_recovery::Recovery;

fn next_connection_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn connection_log(pool: u64, dc: i32, connection: u64, role: &str, event: &str, reason: &str, details: fmt::Arguments<'_>) {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    // Static reason codes and local counters only. Never log keys, peer IDs,
    // RPC bodies, proxy addresses or media URLs. The desktop launcher captures stderr.
    eprintln!("telegram_pool at_ms={at} pool={pool} dc={dc} connection={connection} role={role} event={event} reason={reason} {details}");
}

fn read_error_kind(error: &ReadError) -> &'static str {
    match error {
        ReadError::Io(error) => match error.kind() {
            std::io::ErrorKind::ConnectionReset => "io_reset",
            std::io::ErrorKind::ConnectionAborted => "io_aborted",
            std::io::ErrorKind::UnexpectedEof => "io_eof",
            std::io::ErrorKind::TimedOut => "io_timeout",
            std::io::ErrorKind::BrokenPipe => "io_broken_pipe",
            std::io::ErrorKind::NotConnected => "io_not_connected",
            _ => "io_other",
        },
        ReadError::Transport(transport::Error::BadStatus { .. }) => "transport_status",
        ReadError::Transport(_) => "transport_frame",
        ReadError::Deserialize(_) => "mtproto_decode",
    }
}

enum ProbeOutcome { Healthy, Failed(&'static str), Canceled }
enum PoolEvent {
    Connection { id: u64, dc: i32, media: bool, reason: &'static str, result: Result<(), ReadError> },
    Probe { id: u64, dc: i32, outcome: ProbeOutcome },
}

/// How long to wait after warning the user that the updates was dropped.
const UPDATES_DROPPED_LOG_COOLDOWN: Duration = Duration::from_secs(300);

fn is_media_request(body: &[u8]) -> bool {
    body.get(..4).is_some_and(|id| {
        matches!(
            u32::from_le_bytes(id.try_into().expect("four bytes")),
            tl::functions::upload::GetFile::CONSTRUCTOR_ID
                | tl::functions::upload::SaveFilePart::CONSTRUCTOR_ID
                | tl::functions::upload::SaveBigFilePart::CONSTRUCTOR_ID
        )
    })
}

pub(crate) type Transport = transport::Full;

type InvokeResponse = Vec<u8>;

enum Request {
    Invoke {
        dc_id: i32,
        body: Vec<u8>,
        tx: oneshot::Sender<Result<InvokeResponse, InvocationError>>,
    },
    CheckRetry {
        error: InvocationError,
        fail_count: NonZeroU32,
        slept_so_far: Duration,
        tx: oneshot::Sender<ControlFlow<InvocationError, Duration>>,
    },
    Disconnect {
        dc_id: i32,
    },
    Quit,
    MediaPool { budget: Arc<tokio::sync::Semaphore> },
}

struct Rpc {
    body: Vec<u8>,
    tx: oneshot::Sender<Result<InvokeResponse, InvocationError>>,
}

struct ConnectionInfo {
    id: u64,
    dc_id: i32,
    media: bool,
    rpc_tx: mpsc::UnboundedSender<Rpc>,
    abort_handle: AbortHandle,
    uses: u64,
    last_reuse_log: tokio::time::Instant,
}

/// A fat [`SenderPoolHandle`] with additional metadata from its attached [`SenderPoolRunner`].
#[derive(Clone)]
pub struct SenderPoolFatHandle {
    /// The inner thin handle that self can be derefed into.
    ///
    /// The rest of fields can be dropped if they are no longer needed.
    pub thin: SenderPoolHandle,
    /// The session in use by the attached [`SenderPoolRunner`].
    ///
    /// The runner will read and persist datacenter options in it.
    pub session: Arc<ErasedSession>,
    /// Developer's [Application Identifier](https://core.telegram.org/myapp).
    ///
    /// The [`SenderPoolRunner`] will make use of this value when it needs
    /// to invoke [`tl::functions::InitConnection`] after creating a new connection.
    pub api_id: i32,
}

/// Cheaply cloneable handle to interact with its [`SenderPoolRunner`].
#[derive(Clone)]
pub struct SenderPoolHandle(mpsc::UnboundedSender<Request>);

/// Builder to configure the runner to drive I/O and linked handles.
pub struct SenderPool {
    /// The single mutable instance responsible for driving I/O.
    ///
    /// Connections are created on-demand, so any errors while the pool
    /// is running can only be retrieved with one of the [`SenderPool::handle`]s.
    pub runner: SenderPoolRunner,
    /// Starting fat handle attached to the [`SenderPool::runner`].
    ///
    /// Handles are the only way to interact with the runner once it's running.
    pub handle: SenderPoolFatHandle,
    /// The single mutable channel through which updates received
    /// from the network by the [`SenderPool::runner`] are delivered.
    ///
    /// Update handling must be processed in a sequential manner,
    /// so this is a separate instance with no way to clone it.
    pub updates: mpsc::Receiver<UpdatesLike>,
}

/// Manages and runs a pool of zero or more [`Sender`]s.
///
/// Use [`SenderPool::new`] to create an instance of this type and associated channels.
pub struct SenderPoolRunner {
    session: Arc<ErasedSession>,
    api_id: i32,
    connection_params: ConnectionParams,
    request_rx: mpsc::UnboundedReceiver<Request>,
    updates_tx: mpsc::Sender<UpdatesLike>,
    connections: Vec<ConnectionInfo>,
    connection_pool: JoinSet<PoolEvent>,
    pool_id: u64,
    media_recovery: HashMap<i32, Recovery>,
    fallback_logs: HashMap<i32, (&'static str, tokio::time::Instant)>,
    media_options: HashMap<i32, DcOption>,
    media_budget: Option<Arc<tokio::sync::Semaphore>>,
    media_cursor: HashMap<i32, usize>,
}

impl Deref for SenderPoolFatHandle {
    type Target = SenderPoolHandle;

    fn deref(&self) -> &Self::Target {
        &self.thin
    }
}

impl SenderPoolHandle {
    /// Enable at most two file-only TCP sessions per DC, sharing an account-wide
    /// connection budget supplied by the application. RPC sessions are unaffected.
    pub fn enable_media_pool(&self, budget: Arc<tokio::sync::Semaphore>) -> bool {
        self.0.send(Request::MediaPool { budget }).is_ok()
    }
    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to invoke the request in the specified datacenter.
    pub async fn invoke_in_dc<R: tl::RemoteCall>(
        &self,
        dc_id: i32,
        request: &R,
    ) -> Result<R::Return, InvocationError> {
        self.do_invoke_in_dc(dc_id, request.to_bytes())
            .await
            .and_then(|body| R::Return::from_bytes(&body).map_err(|e| e.into()))
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to invoke the serialized request body in the specified datacenter.
    pub async fn raw_invoke_in_dc(
        &self,
        dc_id: i32,
        body: Vec<u8>,
    ) -> Result<InvokeResponse, InvocationError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(Request::Invoke { dc_id, body, tx })
            .map_err(|_| InvocationError::Dropped("SenderPoolRunner was dropped at send"))?;
        rx.await
            .map_err(|_| InvocationError::Dropped("SenderPoolRunner was dropped during receive"))?
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to drop any active connections to the given datacenter.
    ///
    /// Has no effect if there was no connection to the datacenter.
    ///
    /// This is useful after datacenter migrations during sign in,
    /// when the old connection is known to not be needed anymore.
    pub fn disconnect_from_dc(&self, dc_id: i32) -> bool {
        self.0.send(Request::Disconnect { dc_id }).is_ok()
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to drop all active connections and gracefully stop running.
    pub fn quit(&self) -> bool {
        self.0.send(Request::Quit).is_ok()
    }

    /// do [`Self::raw_invoke_in_dc`] with [`Self::check_retry`]
    pub(crate) async fn do_invoke_in_dc(
        &self,
        dc_id: i32,
        request_body: Vec<u8>,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut fail_count = NonZeroU32::new(1).unwrap();
        let mut slept_so_far = Duration::default();

        loop {
            match self.raw_invoke_in_dc(dc_id, request_body.clone()).await {
                Ok(response) => break Ok(response),
                Err(e) => {
                    let error_info = format!("{}", e);
                    match self.check_retry(e, fail_count, slept_so_far).await {
                        ControlFlow::Continue(delay) => {
                            info!(?delay, "sleeping on {} before retrying", error_info);
                            sleep(delay).await;
                            fail_count = fail_count.saturating_add(1);
                            slept_so_far += delay;
                            continue;
                        }
                        ControlFlow::Break(final_error) => break Err(final_error),
                    }
                }
            }
        }
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to check whether the request needs to retry.
    async fn check_retry(
        &self,
        error: InvocationError,
        fail_count: NonZeroU32,
        slept_so_far: Duration,
    ) -> ControlFlow<InvocationError, Duration> {
        let (tx, rx) = oneshot::channel();
        if self
            .0
            .send(Request::CheckRetry {
                error,
                fail_count,
                slept_so_far,
                tx,
            })
            .is_err()
        {
            return ControlFlow::Break(InvocationError::Dropped(
                "SenderPoolRunner was dropped during retry at send",
            ));
        }

        match rx.await {
            Ok(flow) => flow,
            Err(_) => ControlFlow::Break(InvocationError::Dropped(
                "SenderPooLRunner was dropped during retry during receive",
            )),
        }
    }
}

impl SenderPool {
    /// Creates a new sender pool instance with default configuration,
    /// attached to the given session and using the provided
    /// [Application Identifier](https://core.telegram.org/myapp)
    /// belonging to the developer.
    ///
    /// Session instance **should not** be reused by multiple pools at the same time.
    /// The session instance will only be used to query datacenter options and persist
    /// any permanent Authorization Keys generated for previously-unconncected datacenters.
    pub fn new<S>(session: Arc<S>, api_id: i32) -> Self
    where
        S: Session + Sized,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        Self::with_configuration(session, api_id, Default::default())
    }

    /// Creates a new sender pool with non-[`ConnectionParams::default`] configuration.
    pub fn with_configuration<S>(
        session: Arc<S>,
        api_id: i32,
        connection_params: ConnectionParams,
    ) -> Self
    where
        S: Session + Sized,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let session = erase(session);
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) =
            mpsc::channel(connection_params.updates_channel_capacity.get());

        Self {
            runner: SenderPoolRunner {
                session: Arc::clone(&session),
                api_id,
                connection_params,
                request_rx,
                updates_tx,
                connections: Vec::new(),
                connection_pool: JoinSet::new(),
                media_options: HashMap::new(),
                pool_id: next_connection_id(),
                media_recovery: HashMap::new(),
                fallback_logs: HashMap::new(),
                media_budget: None,
                media_cursor: HashMap::new(),
            },
            handle: SenderPoolFatHandle {
                thin: SenderPoolHandle(request_tx),
                session: Arc::clone(&session),
                api_id,
            },
            updates: updates_rx,
        }
    }
}

impl SenderPoolRunner {
    /// Run the sender pool until [`SenderPoolHandle::quit`] is called or the returned future is dropped.
    ///
    /// Connections will be initiated on-demand whenever the first request to a datacenter is made.
    pub async fn run(mut self) {
        let shutdown_reason = loop {
            tokio::select! {
                biased;
                completion = self.connection_pool.join_next(), if !self.connection_pool.is_empty() => {
                    match completion.unwrap() {
                        Err(err) => if let Ok(reason) = err.try_into_panic() { panic::resume_unwind(reason); },
                        Ok(event) => self.connection_event(event),
                    }
                }
                request = self.request_rx.recv() => {
                    let Some(request) = request else { break "handles_dropped"; };
                    let flow = self.process_request(request).await;
                    match flow {
                        ControlFlow::Continue(_) => continue,
                        ControlFlow::Break(_) => break "pool_quit",
                    }
                }
            }
        };
        for connection in self.connections.drain(..) {
            connection_log(self.pool_id, connection.dc_id, connection.id, if connection.media {"media"} else {"control"}, "retired", shutdown_reason, format_args!("requests={}", connection.uses));
        }
        // Probe response monitors are owned here too; quitting cannot wait for
        // a caller's timeout or leave a probe alive beyond its connection pool.
        self.connection_pool.abort_all();
        while let Some(result) = self.connection_pool.join_next().await {
            if let Err(error) = result && error.is_panic() { panic::resume_unwind(error.into_panic()); }
        }
    }

    fn connection_event(&mut self, event: PoolEvent) {
        match event {
            PoolEvent::Connection { id, dc, media, reason, result } => {
                let Some(index) = self.connections.iter().position(|c| c.id == id) else { return; };
                let connection = self.connections.remove(index);
                let rejected_key = matches!(result, Err(ReadError::Transport(transport::Error::BadStatus {status: 404})));
                let reason = if rejected_key { "transport_404" } else { reason };
                let error_kind = result.as_ref().err().map(read_error_kind).unwrap_or("none");
                let os_error = match &result { Err(ReadError::Io(error)) => error.raw_os_error().unwrap_or(0), _ => 0 };
                let status = match &result { Err(ReadError::Transport(transport::Error::BadStatus { status })) => *status, _ => 0 };
                connection_log(self.pool_id, dc, id, if media {"media"} else {"control"}, "retired", reason, format_args!("requests={} error_kind={error_kind} os_error={os_error} transport_status={status}", connection.uses));
                if media && (rejected_key || matches!(reason, "connect_failed" | "connect_timeout") || self.media_recovery.get(&dc).is_some_and(|r| r.probing == Some(id))) {
                    self.defer_media(dc, reason);
                }
            }
            PoolEvent::Probe { id, dc, outcome } => {
                if !self.media_recovery.get(&dc).is_some_and(|r| r.probing == Some(id)) { return; }
                match outcome {
                    ProbeOutcome::Healthy => {
                        self.media_recovery.remove(&dc);
                        self.fallback_logs.remove(&dc);
                        connection_log(self.pool_id, dc, id, "media", "recovered", "file_rpc_response", format_args!(""));
                    }
                    ProbeOutcome::Failed(reason) => self.defer_media(dc, reason),
                    ProbeOutcome::Canceled => self.defer_media(dc, "probe_caller_dropped"),
                }
            }
        }
    }

    fn defer_media(&mut self, dc: i32, reason: &'static str) {
        let recovery = self.media_recovery.entry(dc).or_insert_with(Recovery::new);
        let attempts = recovery.attempts;
        let delay = recovery.defer();
        connection_log(self.pool_id, dc, 0, "media", "degraded", reason, format_args!("attempts={attempts} retry_after_s={}", delay.as_secs()));
        self.connections.retain(|connection| {
            if connection.media && connection.dc_id == dc {
                connection_log(self.pool_id, dc, connection.id, "media", "retired", "dc_degraded", format_args!("requests={}", connection.uses));
                connection.abort_handle.abort();
                false
            } else { true }
        });
    }

    fn fallback(&mut self, dc: i32, reason: &'static str) {
        let now = tokio::time::Instant::now();
        if self.fallback_logs.get(&dc).is_none_or(|(previous, at)| *previous != reason || now.duration_since(*at) >= Duration::from_secs(30)) {
            connection_log(self.pool_id, dc, 0, "media", "fallback", reason, format_args!(""));
            self.fallback_logs.insert(dc, (reason, now));
        }
    }

    fn reused(&mut self, index: usize) {
        let connection = &mut self.connections[index];
        connection.uses += 1;
        let now = tokio::time::Instant::now();
        if connection.uses == 2 || connection.uses > 2 && now.duration_since(connection.last_reuse_log) >= Duration::from_secs(1) {
            connection.last_reuse_log = now;
            connection_log(self.pool_id, connection.dc_id, connection.id, if connection.media {"media"} else {"control"}, "reused", "rpc_request", format_args!("requests={}", connection.uses));
        }
    }

    fn probe_request(&mut self, index: usize, body: Vec<u8>, mut tx: oneshot::Sender<Result<InvokeResponse, InvocationError>>) {
        let connection = &self.connections[index];
        let (proxy, result) = oneshot::channel();
        let id = connection.id;
        let dc = connection.dc_id;
        let _ = connection.rpc_tx.send(Rpc { body, tx: proxy });
        self.connection_pool.spawn(async move {
            let response = tokio::select! { biased;
                _ = tx.closed() => return PoolEvent::Probe { id, dc, outcome: ProbeOutcome::Canceled },
                response = tokio::time::timeout(Duration::from_secs(25), result) => response,
            };
            let (response, failure) = match response {
                Ok(Ok(response)) => (response, "probe_rpc_failed"),
                Ok(Err(_)) => (Err(InvocationError::Dropped("media probe connection closed")), "probe_connection_closed"),
                Err(_) => (Err(InvocationError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "media probe timeout"))), "probe_timeout"),
            };
            // A valid RPC rejection (including FLOOD_WAIT) proves the MTProto
            // route works; the original rejection still reaches the caller.
            // Authentication/permission failures never count as recovery.
            let healthy = response.is_ok() || matches!(&response, Err(InvocationError::Rpc(error)) if error.code != 401 && error.code != 403);
            let auth_rejected = matches!(&response, Err(InvocationError::Rpc(error)) if error.code == 401 || error.code == 403);
            let outcome = if healthy { ProbeOutcome::Healthy } else { ProbeOutcome::Failed(if auth_rejected { "probe_auth_rejected" } else { failure }) };
            let _ = tx.send(response);
            PoolEvent::Probe { id, dc, outcome }
        });
    }

    async fn process_request(&mut self, request: Request) -> ControlFlow<()> {
        match request {
            Request::Invoke { dc_id, body, tx } => {
                if tx.is_closed() {
                    return ControlFlow::Continue(());
                }
                if is_media_request(&body) {
                    match self.media_connection(dc_id) {
                        Ok(Some((index, probe))) => {
                            self.reused(index);
                            if probe { self.probe_request(index, body, tx); }
                            else { let _ = self.connections[index].rpc_tx.send(Rpc {body, tx}); }
                            return ControlFlow::Continue(());
                        }
                        Ok(None) => {},
                        Err(error) => { let _ = tx.send(Err(error)); return ControlFlow::Continue(()); }
                    }
                }
                let index = match self
                    .connections
                    .iter()
                    .position(|connection| connection.dc_id == dc_id && !connection.media && !connection.abort_handle.is_finished() && !connection.rpc_tx.is_closed())
                {
                    Some(connection) => connection,
                    None => match self.create_connection(dc_id).await {
                        Ok(x) => x,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return ControlFlow::Continue(());
                        }
                    },
                };
                self.reused(index);
                let _ = self.connections[index].rpc_tx.send(Rpc { body, tx });
                ControlFlow::Continue(())
            }
            Request::CheckRetry {
                error,
                fail_count,
                slept_so_far,
                tx,
            } => {
                let retry_context = RetryContext {
                    fail_count,
                    slept_so_far,
                    error,
                };
                let flow = match self
                    .connection_params
                    .retry_policy
                    .should_retry(&retry_context)
                {
                    ControlFlow::Continue(delay) => ControlFlow::Continue(delay),
                    ControlFlow::Break(()) => ControlFlow::Break(retry_context.error),
                };
                let _ = tx.send(flow);
                ControlFlow::Continue(())
            }
            Request::Disconnect { dc_id } => {
                self.media_recovery.remove(&dc_id);
                self.connections.retain(|connection| {
                    if connection.dc_id == dc_id {
                        connection_log(self.pool_id, dc_id, connection.id, if connection.media {"media"} else {"control"}, "retired", "explicit_disconnect", format_args!("requests={}", connection.uses));
                        connection.abort_handle.abort();
                        false
                    } else {
                        true
                    }
                });
                ControlFlow::Continue(())
            }
            Request::Quit => ControlFlow::Break(()),
            Request::MediaPool { budget } => {
                if self.media_budget.is_none() { self.media_budget = Some(budget); }
                ControlFlow::Continue(())
            }
        }
    }

    fn media_connection(&mut self, dc_id: i32) -> Result<Option<(usize, bool)>, InvocationError> {
        if self.media_recovery.get(&dc_id).is_some_and(|r| !r.ready()) {
            let reason = if self.media_recovery[&dc_id].probing.is_some() { "probe_in_flight" } else { "recovery_cooldown" };
            self.fallback(dc_id, reason);
            return Ok(None);
        }
        let probing = self.media_recovery.contains_key(&dc_id);
        let Some(budget) = self.media_budget.clone() else {
            return Ok(None);
        };
        let Some(option) = self.media_options.get(&dc_id).cloned() else {
            self.fallback(dc_id, "no_media_endpoint");
            return Ok(None);
        };
        let Some(key) = self
            .session
            .dc_option(dc_id)?
            .and_then(|option| option.auth_key)
        else {
            self.fallback(dc_id, "no_dc_auth_key");
            return Ok(None);
        };
        let address: std::net::SocketAddr = if !option.ipv6.ip().is_unspecified()
            && (self.connection_params.use_ipv6 || option.ipv4.ip().is_unspecified())
        {
            option.ipv6.into()
        } else if !option.ipv4.ip().is_unspecified() {
            option.ipv4.into()
        } else {
            self.fallback(dc_id, "no_usable_address");
            return Ok(None);
        };
        let mut slots: Vec<_> = self
            .connections
            .iter()
            .enumerate()
            .filter(|(_, c)| c.dc_id == dc_id && c.media && !c.abort_handle.is_finished() && !c.rpc_tx.is_closed())
            .map(|(i, _)| i)
            .collect();
        if slots.len() < if probing { 1 } else { 2 }
            && let Ok(permit) = budget.clone().try_acquire_owned()
        {
            let id = next_connection_id();
            let pool_id = self.pool_id;
            if probing {
                let recovery = self.media_recovery.get_mut(&dc_id).expect("due recovery");
                recovery.begin(id);
                connection_log(pool_id, dc_id, id, "media", "probe_started", "cooldown_elapsed", format_args!("attempt={}", recovery.attempts));
            }
            connection_log(pool_id, dc_id, id, "media", "creating", if probing {"recovery_probe"} else {"file_demand"}, format_args!(""));
            #[cfg(feature = "proxy")]
            let address = if let Some(proxy) = self.connection_params.proxy_url.clone() {
                ServerAddr::Proxied { address, proxy }
            } else {
                ServerAddr::Tcp { address }
            };
            #[cfg(not(feature = "proxy"))]
            let address = ServerAddr::Tcp { address };
            // Only InitConnection headers are used; replace the placeholder
            // query with the actual allowlisted file RPC before sending.
            let mut prefix = tl::functions::InvokeWithLayer {
                layer: tl::LAYER,
                query: tl::functions::InitConnection {
                    api_id: self.api_id,
                    device_model: self.connection_params.device_model.clone(),
                    system_version: self.connection_params.system_version.clone(),
                    app_version: self.connection_params.app_version.clone(),
                    system_lang_code: self.connection_params.system_lang_code.clone(),
                    lang_pack: String::new(),
                    lang_code: self.connection_params.lang_code.clone(),
                    proxy: None,
                    params: None,
                    query: tl::functions::InvokeWithoutUpdates { query: tl::functions::help::GetConfig {} },
                },
            }
            .to_bytes();
            prefix.truncate(prefix.len() - tl::functions::help::GetConfig {}.to_bytes().len());
            let updates = self.updates_tx.clone();
            let (rpc_tx, mut rpc_rx) = mpsc::unbounded_channel::<Rpc>();
            let abort_handle = self.connection_pool.spawn(async move {
                let _permit = permit;
                let (result, reason) = match tokio::time::timeout(
                    Duration::from_secs(15),
                    connect_with_auth(transport::Full::new(), address, key),
                )
                .await
                {
                    Ok(Ok(sender)) => {
                        connection_log(pool_id, dc_id, id, "media", "created", "tcp_connected", format_args!(""));
                        run_sender(sender, rpc_rx, updates, false, Some(prefix)).await
                    }
                    result => {
                        let (kind, reason) = match result {
                            Ok(Err(error)) => (error.kind(), "connect_failed"),
                            _ => (std::io::ErrorKind::TimedOut, "connect_timeout"),
                        };
                        rpc_rx.close();
                        while let Some(rpc) = rpc_rx.recv().await {
                            let _ = rpc.tx.send(Err(InvocationError::Io(std::io::Error::new(
                                kind,
                                "media connection failed",
                            ))));
                        }
                        (Ok(()), reason)
                    }
                };
                PoolEvent::Connection { id, dc: dc_id, media: true, reason, result }
            });
            self.connections.push(ConnectionInfo {
                id,
                dc_id,
                media: true,
                rpc_tx,
                abort_handle,
                uses: 0,
                last_reuse_log: tokio::time::Instant::now(),
            });
            slots.push(self.connections.len() - 1);
        }
        if slots.is_empty() {
            self.fallback(dc_id, "connection_budget_exhausted");
            return Ok(None);
        }
        let cursor = self.media_cursor.entry(dc_id).or_default();
        let index = slots[*cursor % slots.len()];
        *cursor = cursor.wrapping_add(1);
        Ok(Some((index, probing)))
    }

    async fn create_connection(&mut self, dc_id: i32) -> Result<usize, InvocationError> {
        let mut dc_option = match self.session.dc_option(dc_id)? {
            Some(x) => x,
            None => return Err(InvocationError::InvalidDc),
        };

        let id = next_connection_id();
        let pool_id = self.pool_id;
        connection_log(pool_id, dc_id, id, "control", "creating", "rpc_demand", format_args!(""));
        let sender = match self.connect_sender(&dc_option).await {
            Ok(sender) => sender,
            Err(error) => {
                connection_log(pool_id, dc_id, id, "control", "retired", "connect_failed", format_args!(""));
                return Err(error);
            }
        };
        connection_log(pool_id, dc_id, id, "control", "created", "initialized", format_args!(""));

        dc_option.auth_key = Some(sender.auth_key());
        self.session.set_dc_option(&dc_option).await?;

        let (rpc_tx, rpc_rx) = mpsc::unbounded_channel();
        let updates = self.updates_tx.clone();
        let home = dc_option.id == self.session.home_dc_id()?;
        let abort_handle = self.connection_pool.spawn(async move {
            let (result, reason) = run_sender(sender, rpc_rx, updates, home, None).await;
            PoolEvent::Connection { id, dc: dc_id, media: false, reason, result }
        });
        self.connections.push(ConnectionInfo {
            id,
            dc_id,
            media: false,
            rpc_tx,
            abort_handle,
            uses: 0,
            last_reuse_log: tokio::time::Instant::now(),
        });
        Ok(self.connections.len() - 1)
    }

    async fn connect_sender(
        &mut self,
        dc_option: &DcOption,
    ) -> Result<Sender<transport::Full, mtp::Encrypted>, InvocationError> {
        let transport = transport::Full::new;

        let address = if self.connection_params.use_ipv6 {
            dc_option.ipv6.into()
        } else {
            dc_option.ipv4.into()
        };

        #[cfg(feature = "proxy")]
        let addr = || {
            if let Some(proxy) = self.connection_params.proxy_url.clone() {
                ServerAddr::Proxied { address, proxy }
            } else {
                ServerAddr::Tcp { address }
            }
        };
        #[cfg(not(feature = "proxy"))]
        let addr = || ServerAddr::Tcp { address };

        let init_connection = tl::functions::InvokeWithLayer {
            layer: tl::LAYER,
            query: tl::functions::InitConnection {
                api_id: self.api_id,
                device_model: self.connection_params.device_model.clone(),
                system_version: self.connection_params.system_version.clone(),
                app_version: self.connection_params.app_version.clone(),
                system_lang_code: self.connection_params.system_lang_code.clone(),
                lang_pack: "".into(),
                lang_code: self.connection_params.lang_code.clone(),
                proxy: None,
                params: None,
                query: tl::functions::help::GetConfig {},
            },
        };

        let mut sender = if let Some(auth_key) = dc_option.auth_key {
            connect_with_auth(transport(), addr(), auth_key)
                .await
                .map_err(InvocationError::Io)?
        } else {
            connect(transport(), addr()).await?
        };

        let enums::Config::Config(remote_config) = match sender.invoke(&init_connection).await {
            Ok(config) => config,
            Err(InvocationError::Transport(transport::Error::BadStatus { status: 404 })) => {
                sender = connect(transport(), addr()).await?;
                sender.invoke(&init_connection).await?
            }
            Err(e) => return Err(e),
        };

        self.update_config(remote_config).await?;

        Ok(sender)
    }

    async fn update_config(
        &mut self,
        config: tl::types::Config,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.media_options.clear();
        for enums::DcOption::Option(option) in &config.dc_options {
            if !option.media_only || option.tcpo_only || option.cdn { continue; }
            let dc = self.media_options.entry(option.id).or_insert(DcOption {
                id: option.id, ipv4: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
                ipv6: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0), auth_key: None,
            });
            if option.ipv6 { dc.ipv6 = SocketAddrV6::new(option.ip_address.parse()?, option.port.try_into()?, 0, 0); }
            else { dc.ipv4 = SocketAddrV4::new(option.ip_address.parse()?, option.port.try_into()?); }
        }
        for option in config
            .dc_options
            .iter()
            .map(|tl::enums::DcOption::Option(option)| option)
            .filter(|option| !option.media_only && !option.tcpo_only && option.r#static)
        {
            let mut dc_option = self
                .session
                .dc_option(option.id)?
                .unwrap_or_else(|| DcOption {
                    id: option.id,
                    ipv4: SocketAddrV4::new(Ipv4Addr::from_bits(0), 0),
                    ipv6: SocketAddrV6::new(Ipv6Addr::from_bits(0), 0, 0, 0),
                    auth_key: None,
                });
            if option.ipv6 {
                dc_option.ipv6 = SocketAddrV6::new(
                    option
                        .ip_address
                        .parse()
                        .expect("Telegram to return a valid IPv6 address"),
                    option.port as _,
                    0,
                    0,
                );
            } else {
                dc_option.ipv4 = SocketAddrV4::new(
                    option
                        .ip_address
                        .parse()
                        .expect("Telegram to return a valid IPv4 address"),
                    option.port as _,
                );
                if dc_option.ipv6.ip().to_bits() == 0 {
                    dc_option.ipv6 = SocketAddrV6::new(
                        dc_option.ipv4.ip().to_ipv6_mapped(),
                        dc_option.ipv4.port(),
                        0,
                        0,
                    )
                }
            }
            self.session.set_dc_option(&dc_option).await?;
        }
        Ok(())
    }
}

async fn run_sender(
    mut sender: Sender<Transport, grammers_mtproto::mtp::Encrypted>,
    mut rpc_rx: mpsc::UnboundedReceiver<Rpc>,
    updates: mpsc::Sender<UpdatesLike>,
    home_sender: bool,
    media_prefix: Option<Vec<u8>>,
) -> (Result<(), ReadError>, &'static str) {
    let mut dropped: usize = 0;
    let mut last_update_limit_warn = Instant::now();
    let mut updates_closed = false;
    let mut last_request = tokio::time::Instant::now();

    loop {
        tokio::select! {
            step = sender.step() => match step {
                Ok(all_new_updates) => {
                    if updates_closed || media_prefix.is_some() {
                        continue;
                    }
                    for new_updates in all_new_updates {
                        if let Err(e) = updates.try_send(new_updates) {
                            match e {
                                mpsc::error::TrySendError::Full(_) => {
                                    dropped += 1;
                                    if last_update_limit_warn.elapsed() >= UPDATES_DROPPED_LOG_COOLDOWN {
                                        warn!(
                                            capacity = updates.max_capacity(),
                                            dropped_count = dropped,
                                            last_time = ?last_update_limit_warn.elapsed(),
                                            "updates channel full",
                                        );
                                        dropped = 0;
                                        last_update_limit_warn = Instant::now();
                                    }
                                }
                                mpsc::error::TrySendError::Closed(_) => {
                                    debug!("updates channel closed, stopping update forwarding");
                                    updates_closed = true;
                                    break;
                                }
                            }
                        }
                    }
                },
                Err(err) => {
                    if home_sender {
                        let _ = updates.try_send(UpdatesLike::ConnectionClosed);
                    }
                    break (Err(err), "transport_error")
                },
            },
            rpc = rpc_rx.recv() => match rpc {
                Some(rpc) => {
                    if rpc.tx.is_closed() { continue; }
                    last_request = tokio::time::Instant::now();
                    if let Some(prefix) = &media_prefix {
                        let mut body = prefix.clone(); body.extend_from_slice(&rpc.body);
                        sender.enqueue_media_body(body, rpc.tx);
                    } else { sender.enqueue_body(rpc.body, rpc.tx); }
                },
                None => break (Ok(()), "rpc_queue_closed"),
            },
            _ = tokio::time::sleep_until(last_request + Duration::from_secs(60)), if media_prefix.is_some() && !sender.has_pending_file_requests() && rpc_rx.is_empty() => break (Ok(()), "idle_60s"),
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invoke { dc_id, body, tx } => f
                .debug_struct("Invoke")
                .field("dc_id", dc_id)
                .field(
                    "request",
                    &body[..4]
                        .try_into()
                        .map(|constructor_id| tl::name_for_id(u32::from_le_bytes(constructor_id)))
                        .unwrap_or("?"),
                )
                .field("tx", tx)
                .finish(),
            Self::CheckRetry {
                error,
                fail_count,
                slept_so_far,
                tx,
            } => f
                .debug_struct("CheckRetry")
                .field("error", error)
                .field("fail_count", fail_count)
                .field("slept_so_far", slept_so_far)
                .field("tx", tx)
                .finish(),
            Self::Disconnect { dc_id } => {
                f.debug_struct("Disconnect").field("dc_id", dc_id).finish()
            }
            Self::Quit => write!(f, "Quit"),
            Self::MediaPool { .. } => write!(f, "MediaPool"),
        }
    }
}

#[cfg(test)]
#[path = "media_pool_tests.rs"]
mod media_pool_tests;
