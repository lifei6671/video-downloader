//! Bounded file RPCs on the existing authenticated sender. No additional MTProto sessions.
use anyhow::{bail, ensure, Context, Result};
use futures_util::{stream::FuturesUnordered, StreamExt};
use grammers_client::{tl, Client, InvocationError};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore},
};

type Work<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
// grammers-mtproto 0.11 rejects a 1 MiB response in a server message container.
pub(crate) const DOWNLOAD_PART: u64 = 512 * 1024;
pub(crate) const UPLOAD_PART: u64 = 512 * 1024;
const GLOBAL_REQUESTS: usize = 16;
const MEMORY_BUDGET: usize = 64 * 1024 * 1024;
// The sender detects 60s without socket progress and caps each file RPC at 599s.
const RPC_TIMEOUT: Duration = Duration::from_secs(600);
// A chunk can make four RPC attempts; enclosing media reads must not expire first.
pub(crate) const FILE_IO_TIMEOUT: Duration = Duration::from_secs(4 * 600 + 10);

#[derive(Clone)]
pub(crate) struct Settings {
    pub concurrency: usize,
    pub stats: Arc<Stats>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            concurrency: 8,
            stats: Arc::default(),
        }
    }
}
#[cfg(test)]
tokio::task_local! { static TEST_SETTINGS: Settings; }
#[cfg(test)]
pub(crate) async fn with_settings<T>(settings: Settings, work: impl Future<Output = T>) -> T {
    TEST_SETTINGS.scope(settings, work).await
}
fn settings() -> Settings {
    #[cfg(test)]
    if let Ok(settings) = TEST_SETTINGS.try_with(Clone::clone) {
        return settings;
    }
    Settings::default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub index: u64,
    pub offset: u64,
    pub length: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Plan {
    bytes: u64,
    part: u64,
    pub parts: u64,
    pub workers: usize,
}
impl Plan {
    pub fn new(bytes: u64, part: u64, concurrency: usize) -> Result<Self> {
        ensure!(
            bytes > 0 && bytes <= i64::MAX as u64,
            "TELEGRAM_FILE_TOO_LARGE"
        );
        ensure!(part == UPLOAD_PART, "INVALID_INPUT");
        ensure!((1..=16).contains(&concurrency), "INVALID_INPUT");
        let parts = bytes.div_ceil(part);
        let size_limit = match bytes {
            0..1_048_576 => 1,
            1_048_576..5_242_880 => 2,
            5_242_880..20_971_520 => 4,
            20_971_520..52_428_800 => 8,
            _ => concurrency,
        };
        let workers = concurrency
            .min(size_limit)
            .min(parts as usize)
            .min(GLOBAL_REQUESTS)
            .min(MEMORY_BUDGET / part as usize);
        Ok(Self {
            bytes,
            part,
            parts,
            workers,
        })
    }
    pub fn chunk(&self, index: u64) -> Chunk {
        let offset = index * self.part;
        Chunk {
            index,
            offset,
            length: (self.bytes - offset).min(self.part) as usize,
        }
    }
}

struct RequestWindow {
    generation: u64,
    weight: usize,
    successes: usize,
    stable_since: tokio::time::Instant,
}
impl Default for RequestWindow {
    fn default() -> Self {
        Self {
            generation: 0,
            weight: 1,
            successes: 0,
            stable_since: tokio::time::Instant::now(),
        }
    }
}
#[derive(Debug)]
struct RequestPermit {
    _permit: OwnedSemaphorePermit,
    generation: u64,
}

/// Owned by one live authorization session, never shared merely by account id.
#[derive(Default)]
pub(crate) struct Authorization(Mutex<HashMap<i32, Arc<DcAuthorization>>>);
#[derive(Default)]
struct DcAuthorization {
    generation: AtomicU64,
    importing: tokio::sync::Mutex<()>,
}
impl Authorization {
    fn dc(&self, dc: i32) -> Arc<DcAuthorization> {
        self.0
            .lock()
            .expect("DC authorization")
            .entry(dc)
            .or_default()
            .clone()
    }
}

/// Shared across backup/download bindings with the same verified account id.
pub(crate) struct Budget {
    requests: Arc<Semaphore>,
    window: Mutex<RequestWindow>,
    media_connections: Arc<Semaphore>,
    cooldown: Mutex<Option<Instant>>,
}
impl Budget {
    pub fn account(id: &str) -> Arc<Self> {
        static ACCOUNTS: OnceLock<Mutex<HashMap<String, Arc<Budget>>>> = OnceLock::new();
        let mut accounts = ACCOUNTS
            .get_or_init(Mutex::default)
            .lock()
            .expect("transfer budgets");
        // Retain a server cooldown even when the last failed transfer drops its lease.
        accounts.retain(|_, value| {
            Arc::strong_count(value) > 1
                || value
                    .cooldown
                    .lock()
                    .expect("transfer cooldown")
                    .is_some_and(|until| until > Instant::now())
        });
        if let Some(budget) = accounts.get(id) {
            return budget.clone();
        }
        let budget = Arc::new(Self {
            requests: Arc::new(Semaphore::new(GLOBAL_REQUESTS)),
            window: Mutex::new(RequestWindow::default()),
            media_connections: Arc::new(Semaphore::new(8)),
            cooldown: Mutex::new(None),
        });
        accounts.insert(id.into(), budget.clone());
        budget
    }
    pub(crate) fn media_connections(&self) -> Arc<Semaphore> {
        self.media_connections.clone()
    }
    fn check(&self) -> Result<()> {
        if let Some(until) = *self.cooldown.lock().expect("transfer cooldown") {
            let left = until.saturating_duration_since(Instant::now());
            if !left.is_zero() {
                bail!("TELEGRAM_FLOOD_WAIT:{}", left.as_secs().saturating_add(1));
            }
        }
        Ok(())
    }
    async fn acquire(&self) -> Result<RequestPermit> {
        loop {
            self.check()?;
            let (generation, weight) = {
                let window = self.window.lock().expect("transfer window");
                (window.generation, window.weight)
            };
            let permit = self
                .requests
                .clone()
                .acquire_many_owned(weight as u32)
                .await
                .context("TELEGRAM_TRANSFER_CANCELED")?;
            self.check()?;
            // A queued request must enter the current window, including after recovery.
            if generation == self.window.lock().expect("transfer window").generation {
                return Ok(RequestPermit {
                    _permit: permit,
                    generation,
                });
            }
        }
    }
    fn failed(&self, generation: u64, timed_out: bool) {
        let mut window = self.window.lock().expect("transfer window");
        if generation != window.generation {
            return;
        }
        window.successes = 0;
        window.stable_since = tokio::time::Instant::now();
        if timed_out {
            window.weight = (window.weight * 2).min(GLOBAL_REQUESTS);
            window.generation = window.generation.wrapping_add(1);
        }
    }
    fn succeeded(&self, generation: u64) {
        let mut window = self.window.lock().expect("transfer window");
        if generation != window.generation || window.weight == 1 {
            return;
        }
        window.successes += 1;
        // Require both sustained progress and a stable interval before opening one step.
        if window.successes >= 32 && window.stable_since.elapsed() >= Duration::from_secs(30) {
            window.weight /= 2;
            window.generation = window.generation.wrapping_add(1);
            window.successes = 0;
            window.stable_since = tokio::time::Instant::now();
        }
    }
    fn flood(&self, seconds: u32) {
        let until = Instant::now() + Duration::from_secs(u64::from(seconds));
        let mut cooldown = self.cooldown.lock().expect("transfer cooldown");
        *cooldown = Some(cooldown.map_or(until, |old| old.max(until)));
    }
}

#[derive(Default)]
pub(crate) struct Stats {
    pub read_bytes: AtomicU64,
    pub acknowledged: AtomicU64,
    pub attempts: AtomicU64,
    pub retries: AtomicU64,
    pub failures: AtomicU64,
    pub rpc_nanos: AtomicU64,
    active: AtomicUsize,
    pub peak_rpc: AtomicUsize,
    recent_parts: Mutex<VecDeque<PartReport>>,
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct PartReport {
    index: u64,
    offset: u64,
    length: usize,
    request_length: usize,
    status: &'static str,
    elapsed_ms: u64,
    rpc_ms: u64,
    attempts: u64,
    retries: u64,
    last_error: Option<String>,
}
impl Stats {
    #[cfg(test)]
    pub fn recent_parts(&self) -> Vec<PartReport> {
        self.recent_parts
            .lock()
            .expect("part reports")
            .iter()
            .cloned()
            .collect()
    }
}
struct PartMeter<'a> {
    stats: &'a Stats,
    chunk: Chunk,
    request_length: usize,
    start: Instant,
    attempts: AtomicU64,
    rpc_nanos: AtomicU64,
    last_error: Mutex<Option<String>>,
    status: &'static str,
}
impl<'a> PartMeter<'a> {
    fn new(stats: &'a Stats, chunk: Chunk, request_length: usize) -> Self {
        Self {
            stats,
            chunk,
            request_length,
            start: Instant::now(),
            attempts: AtomicU64::new(0),
            rpc_nanos: AtomicU64::new(0),
            last_error: Mutex::new(None),
            status: "canceled",
        }
    }
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        self.status = if result.is_ok() { "complete" } else { "failed" };
        if let Err(error) = &result {
            *self.last_error.get_mut().expect("part error") = Some(error.to_string());
        }
        result
    }
}
impl Drop for PartMeter<'_> {
    fn drop(&mut self) {
        let attempts = self.attempts.load(Ordering::Relaxed);
        let report = PartReport {
            index: self.chunk.index,
            offset: self.chunk.offset,
            length: self.chunk.length,
            request_length: self.request_length,
            status: self.status,
            elapsed_ms: self.start.elapsed().as_millis().min(u64::MAX as u128) as u64,
            rpc_ms: self.rpc_nanos.load(Ordering::Relaxed) / 1_000_000,
            attempts,
            retries: attempts.saturating_sub(1),
            last_error: self.last_error.get_mut().expect("part error").take(),
        };
        let mut reports = self.stats.recent_parts.lock().expect("part reports");
        if reports.len() == 64 {
            reports.pop_front();
        }
        reports.push_back(report);
    }
}
struct Active<'a>(&'a Stats);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}
struct RpcTimer<'a> {
    stats: &'a Stats,
    part: Option<&'a PartMeter<'a>>,
    start: Instant,
}
impl Drop for RpcTimer<'_> {
    fn drop(&mut self) {
        let nanos = self.start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        self.stats.rpc_nanos.fetch_add(nanos, Ordering::Relaxed);
        if let Some(part) = self.part {
            part.rpc_nanos.fetch_add(nanos, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy)]
enum Route {
    File,
    Main,
}

/// Optional photo/thumbnail reads still participate in the account's file budget and cooldown.
pub(crate) async fn cover_chunk<T>(
    budget: &Budget,
    work: impl Future<Output = std::result::Result<T, InvocationError>>,
) -> Result<T> {
    let permit = budget.acquire().await?;
    match tokio::time::timeout(RPC_TIMEOUT, work).await {
        Ok(Ok(value)) => {
            budget.succeeded(permit.generation);
            Ok(value)
        }
        Ok(Err(error)) => {
            budget.failed(permit.generation, matches!(&error, InvocationError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut));
            if let InvocationError::Rpc(ref rpc) = error {
                if rpc.code == 420 {
                    budget.flood(rpc.value.unwrap_or(60));
                }
            }
            Err(crate::backup::telegram_session::rpc_error(error))
        }
        Err(_) => {
            budget.failed(permit.generation, true);
            bail!("TELEGRAM_TRANSFER_TIMEOUT");
        }
    }
}
// Routing controls error handling; callers confirm valid file data separately.
// Return the admitted generation so delayed replies cannot recover a newer window.
async fn rpc<'a, T>(
    budget: &Budget,
    stats: &Stats,
    route: Route,
    part: Option<&PartMeter<'_>>,
    mut call: impl FnMut() -> Pin<
        Box<dyn Future<Output = std::result::Result<T, InvocationError>> + Send + 'a>,
    >,
) -> Result<(T, u64)> {
    for attempt in 0..4 {
        let permit = budget.acquire().await?;
        stats.attempts.fetch_add(1, Ordering::Relaxed);
        if let Some(part) = part {
            part.attempts.fetch_add(1, Ordering::Relaxed);
        }
        let active = stats.active.fetch_add(1, Ordering::Relaxed) + 1;
        stats.peak_rpc.fetch_max(active, Ordering::Relaxed);
        let guard = Active(stats);
        let timer = RpcTimer {
            stats,
            part,
            start: Instant::now(),
        };
        let result = tokio::time::timeout(RPC_TIMEOUT, call()).await;
        drop(timer);
        drop(guard);
        let generation = permit.generation;
        drop(permit);
        let (error, retry) = match result {
            Ok(Ok(value)) => return Ok((value, generation)),
            Ok(Err(error)) => {
                budget.failed(generation, matches!(&error, InvocationError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut));
                if let InvocationError::Rpc(ref e) = error {
                    if e.code == 420 {
                        budget.flood(e.value.unwrap_or(60));
                    }
                    if matches!(route, Route::File) && e.name == "FILE_MIGRATE" {
                        stats.failures.fetch_add(1, Ordering::Relaxed);
                        if let Some(part) = part {
                            *part.last_error.lock().expect("part error") =
                                Some("TELEGRAM_FILE_MIGRATE".into());
                        }
                        bail!(
                            "TELEGRAM_FILE_MIGRATE:{}",
                            e.value.context("TELEGRAM_RPC_ERROR")?
                        );
                    }
                    if matches!(route, Route::File) && e.name == "AUTH_KEY_UNREGISTERED" {
                        stats.failures.fetch_add(1, Ordering::Relaxed);
                        if let Some(part) = part {
                            *part.last_error.lock().expect("part error") =
                                Some("TELEGRAM_DC_AUTH_REQUIRED".into());
                        }
                        bail!("TELEGRAM_DC_AUTH_REQUIRED");
                    }
                }
                let retry = matches!(
                    &error,
                    InvocationError::Io(_)
                        | InvocationError::Transport(_)
                        | InvocationError::Dropped(_)
                ) || matches!(&error, InvocationError::Rpc(e) if e.code >= 500);
                (crate::backup::telegram_session::rpc_error(error), retry)
            }
            Err(_) => {
                budget.failed(generation, true);
                (anyhow::anyhow!("TELEGRAM_TRANSFER_TIMEOUT"), true)
            }
        };
        stats.failures.fetch_add(1, Ordering::Relaxed);
        if let Some(part) = part {
            *part.last_error.lock().expect("part error") = Some(error.to_string());
        }
        if !retry || attempt == 3 {
            return Err(error);
        }
        stats.retries.fetch_add(1, Ordering::Relaxed);
        let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
        tokio::time::sleep(Duration::from_millis((200 << attempt) + jitter)).await;
    }
    unreachable!("bounded retries")
}

pub(crate) trait ChunkSource: Send + Sync {
    fn fetch(&self, chunk: Chunk) -> Work<'_, Vec<u8>>;
}
pub(crate) type Refresh = Arc<dyn Fn() -> Work<'static, tl::types::Document> + Send + Sync>;
type RpcWork<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, InvocationError>> + Send + 'a>>;
trait FileClient: Send + Sync {
    fn get<'a>(
        &'a self,
        dc: i32,
        request: &'a tl::functions::upload::GetFile,
    ) -> RpcWork<'a, tl::enums::upload::File>;
    fn home_dc(&self) -> RpcWork<'_, i32>;
    fn export(&self, dc: i32) -> RpcWork<'_, tl::types::auth::ExportedAuthorization>;
    fn import<'a>(
        &'a self,
        dc: i32,
        exported: &'a tl::types::auth::ExportedAuthorization,
    ) -> RpcWork<'a, ()>;
}
struct GrammersFileClient(Client);
impl FileClient for GrammersFileClient {
    fn get<'a>(
        &'a self,
        dc: i32,
        request: &'a tl::functions::upload::GetFile,
    ) -> RpcWork<'a, tl::enums::upload::File> {
        Box::pin(self.0.invoke_in_dc(dc, request))
    }
    fn home_dc(&self) -> RpcWork<'_, i32> {
        Box::pin(async {
            let tl::enums::Config::Config(config) =
                self.0.invoke(&tl::functions::help::GetConfig {}).await?;
            Ok(config.this_dc)
        })
    }
    fn export(&self, dc: i32) -> RpcWork<'_, tl::types::auth::ExportedAuthorization> {
        Box::pin(async move {
            let tl::enums::auth::ExportedAuthorization::Authorization(exported) = self
                .0
                .invoke(&tl::functions::auth::ExportAuthorization { dc_id: dc })
                .await?;
            Ok(exported)
        })
    }
    fn import<'a>(
        &'a self,
        dc: i32,
        exported: &'a tl::types::auth::ExportedAuthorization,
    ) -> RpcWork<'a, ()> {
        Box::pin(async move {
            self.0
                .invoke_in_dc(
                    dc,
                    &tl::functions::auth::ImportAuthorization {
                        id: exported.id,
                        bytes: exported.bytes.clone(),
                    },
                )
                .await?;
            Ok(())
        })
    }
}
pub(crate) struct DocumentSource {
    authorization: Arc<Authorization>,
    client: Arc<dyn FileClient>,
    document: tokio::sync::Mutex<(tl::types::Document, usize)>,
    refresh: Refresh,
    budget: Arc<Budget>,
    stats: Arc<Stats>,
}
pub(crate) fn same_document(old: &tl::types::Document, fresh: &tl::types::Document) -> bool {
    old.id == fresh.id
        && old.access_hash == fresh.access_hash
        && old.size == fresh.size
        && old.mime_type == fresh.mime_type
        && old.attributes == fresh.attributes
}
impl ChunkSource for DocumentSource {
    fn fetch(&self, chunk: Chunk) -> Work<'_, Vec<u8>> {
        Box::pin(async move {
            let mut part = PartMeter::new(&self.stats, chunk, DOWNLOAD_PART as usize);
            let result = self.fetch_part(chunk, &part).await;
            part.finish(result)
        })
    }
}
impl DocumentSource {
    async fn fetch_part(&self, chunk: Chunk, part: &PartMeter<'_>) -> Result<Vec<u8>> {
        let mut authorized = false;
        let mut dc = self.document.lock().await.0.dc_id;
        // Migration, authorization import and reference refresh each have a finite bound.
        for _ in 0..6 {
            let document = self.document.lock().await.0.clone();
            let request = tl::functions::upload::GetFile {
                precise: false,
                cdn_supported: false,
                location: tl::types::InputDocumentFileLocation {
                    id: document.id,
                    access_hash: document.access_hash,
                    file_reference: document.file_reference.clone(),
                    thumb_size: String::new(),
                }
                .into(),
                offset: chunk.offset as i64,
                limit: DOWNLOAD_PART as i32,
            };
            let authorization = self.authorization.dc(dc);
            let generation = authorization.generation.load(Ordering::Acquire);
            // Keep raw routing errors here; retry classification is shared below.
            let result = rpc(&self.budget, &self.stats, Route::File, Some(part), || {
                self.client.get(dc, &request)
            })
            .await;
            match result {
                Ok((tl::enums::upload::File::File(file), request_generation)) => {
                    if file.bytes.len() != chunk.length {
                        self.budget.failed(request_generation, false);
                        bail!("INCOMPLETE_MEDIA");
                    }
                    self.budget.succeeded(request_generation);
                    self.stats
                        .acknowledged
                        .fetch_add(file.bytes.len() as u64, Ordering::Relaxed);
                    return Ok(file.bytes);
                }
                Ok((tl::enums::upload::File::CdnRedirect(_), request_generation)) => {
                    self.budget.failed(request_generation, false);
                    bail!("TELEGRAM_CDN_UNSUPPORTED")
                }
                Err(error) if error.to_string() == "TELEGRAM_FILE_REFERENCE_EXPIRED" => {
                    let mut current = self.document.lock().await;
                    if current.0.file_reference == document.file_reference {
                        ensure!(current.1 < 2, "TELEGRAM_RPC_ERROR");
                        let fresh = (self.refresh)().await?;
                        ensure!(same_document(&current.0, &fresh), "TELEGRAM_MEDIA_CHANGED");
                        current.0 = fresh;
                        current.1 += 1;
                    }
                    dc = current.0.dc_id;
                }
                Err(error) if error.to_string().starts_with("TELEGRAM_FILE_MIGRATE:") => {
                    dc = error
                        .to_string()
                        .split(':')
                        .nth(1)
                        .context("TELEGRAM_RPC_ERROR")?
                        .parse()?;
                    ensure!(dc > 0, "TELEGRAM_RPC_ERROR");
                    self.document.lock().await.0.dc_id = dc;
                }
                Err(error) if error.to_string() == "TELEGRAM_DC_AUTH_REQUIRED" && !authorized => {
                    let _importing = authorization.importing.lock().await;
                    if authorization.generation.load(Ordering::Acquire) != generation {
                        authorized = true;
                        continue;
                    }
                    let (home, _) = rpc(&self.budget, &self.stats, Route::Main, None, || {
                        self.client.home_dc()
                    })
                    .await?;
                    ensure!(dc != home, "TELEGRAM_AUTH_REQUIRED");
                    let (exported, _) = rpc(&self.budget, &self.stats, Route::Main, None, || {
                        self.client.export(dc)
                    })
                    .await?;
                    rpc(&self.budget, &self.stats, Route::Main, None, || {
                        self.client.import(dc, &exported)
                    })
                    .await?;
                    authorization.generation.fetch_add(1, Ordering::Release);
                    authorized = true;
                }
                Err(error) if error.to_string() == "TELEGRAM_DC_AUTH_REQUIRED" => {
                    bail!("TELEGRAM_AUTH_REQUIRED")
                }
                Err(error) => return Err(error),
            }
        }
        bail!("TELEGRAM_RPC_ERROR")
    }
}

type PendingChunk = Work<'static, (Chunk, Vec<u8>)>;
pub(crate) struct Download {
    source: Arc<dyn ChunkSource>,
    plan: Plan,
    scheduled: u64,
    emitted: u64,
    skip: usize,
    pending: FuturesUnordered<PendingChunk>,
    ready: BTreeMap<u64, Vec<u8>>,
    failed: bool,
}
impl Download {
    pub fn new(source: Arc<dyn ChunkSource>, plan: Plan) -> Self {
        Self {
            source,
            plan,
            scheduled: 0,
            emitted: 0,
            skip: 0,
            pending: FuturesUnordered::new(),
            ready: BTreeMap::new(),
            failed: false,
        }
    }
    pub fn from_offset(source: Arc<dyn ChunkSource>, plan: Plan, offset: u64) -> Result<Self> {
        ensure!(offset <= plan.bytes, "BACKUP_INVALID_RANGE");
        let mut download = Self::new(source, plan);
        let first = if offset == plan.bytes {
            plan.parts
        } else {
            offset / plan.part
        };
        download.scheduled = first;
        download.emitted = first;
        download.skip = if offset == plan.bytes {
            0
        } else {
            (offset % plan.part) as usize
        };
        Ok(download)
    }
    async fn next_inner(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            // A sliding window bounds both in-flight and completed out-of-order chunks.
            while self.scheduled < self.plan.parts
                && self.scheduled < self.emitted + self.plan.workers as u64
            {
                let chunk = self.plan.chunk(self.scheduled);
                let source = self.source.clone();
                self.pending.push(Box::pin(async move {
                    let bytes = source.fetch(chunk).await?;
                    ensure!(bytes.len() == chunk.length, "INCOMPLETE_MEDIA");
                    Ok((chunk, bytes))
                }));
                self.scheduled += 1;
            }
            if let Some(mut bytes) = self.ready.remove(&self.emitted) {
                self.emitted += 1;
                if self.skip != 0 {
                    bytes.drain(..self.skip);
                    self.skip = 0;
                }
                return Ok(Some(bytes));
            }
            let Some(result) = self.pending.next().await else {
                ensure!(self.emitted == self.plan.parts, "INCOMPLETE_MEDIA");
                return Ok(None);
            };
            let (chunk, bytes) = result?;
            ensure!(
                chunk.index >= self.emitted && !self.ready.contains_key(&chunk.index),
                "INCOMPLETE_MEDIA"
            );
            self.ready.insert(chunk.index, bytes);
        }
    }
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>> {
        ensure!(!self.failed, "INCOMPLETE_MEDIA");
        let result = self.next_inner().await;
        if result.is_err() {
            self.failed = true;
            self.pending.clear();
            self.ready.clear();
        }
        result
    }
}

/// File sinks can consume completed chunks immediately. A slow/retried early
/// chunk must not pin the ordered reader's entire window of completed chunks.
pub(crate) struct FileDownload {
    source: Arc<dyn ChunkSource>,
    plan: Plan,
    scheduled: u64,
    completed: u64,
    active: HashSet<u64>,
    pending: FuturesUnordered<PendingChunk>,
    failed: bool,
}
impl FileDownload {
    fn new(source: Arc<dyn ChunkSource>, plan: Plan) -> Self {
        Self {
            source,
            plan,
            scheduled: 0,
            completed: 0,
            active: HashSet::new(),
            pending: FuturesUnordered::new(),
            failed: false,
        }
    }
    async fn next_inner(&mut self) -> Result<Option<(Chunk, Vec<u8>)>> {
        while self.scheduled < self.plan.parts && self.pending.len() < self.plan.workers {
            let chunk = self.plan.chunk(self.scheduled);
            let source = self.source.clone();
            self.active.insert(chunk.index);
            self.pending.push(Box::pin(
                async move { Ok((chunk, source.fetch(chunk).await?)) },
            ));
            self.scheduled += 1;
        }
        let Some(result) = self.pending.next().await else {
            ensure!(
                self.completed == self.plan.parts && self.active.is_empty(),
                "INCOMPLETE_MEDIA"
            );
            return Ok(None);
        };
        let (chunk, bytes) = result?;
        ensure!(
            self.active.remove(&chunk.index)
                && chunk == self.plan.chunk(chunk.index)
                && bytes.len() == chunk.length,
            "INCOMPLETE_MEDIA"
        );
        self.completed += 1;
        Ok(Some((chunk, bytes)))
    }
    pub async fn next(&mut self) -> Result<Option<(Chunk, Vec<u8>)>> {
        ensure!(!self.failed, "INCOMPLETE_MEDIA");
        let result = self.next_inner().await;
        if result.is_err() {
            self.failed = true;
            self.pending.clear();
            self.active.clear();
        }
        result
    }
}

fn document_download(
    client: Client,
    document: tl::types::Document,
    refresh: Refresh,
    budget: Arc<Budget>,
    authorization: Arc<Authorization>,
) -> Result<(Arc<dyn ChunkSource>, Plan)> {
    let settings = settings();
    let plan = Plan::new(
        u64::try_from(document.size).context("INCOMPLETE_MEDIA")?,
        DOWNLOAD_PART,
        settings.concurrency,
    )?;
    Ok((
        Arc::new(DocumentSource {
            authorization,
            client: Arc::new(GrammersFileClient(client)),
            document: tokio::sync::Mutex::new((document, 0)),
            refresh,
            budget,
            stats: settings.stats,
        }),
        plan,
    ))
}

pub(crate) fn download(
    client: Client,
    document: tl::types::Document,
    refresh: Refresh,
    budget: Arc<Budget>,
    authorization: Arc<Authorization>,
    offset: u64,
) -> Result<Download> {
    let (source, plan) = document_download(client, document, refresh, budget, authorization)?;
    Download::from_offset(source, plan, offset)
}
pub(crate) fn download_file(
    client: Client,
    document: tl::types::Document,
    refresh: Refresh,
    budget: Arc<Budget>,
    authorization: Arc<Authorization>,
) -> Result<FileDownload> {
    let (source, plan) = document_download(client, document, refresh, budget, authorization)?;
    Ok(FileDownload::new(source, plan))
}

type ReadNext = Work<'static, (Download, Option<Vec<u8>>)>;
pub(crate) struct Reader {
    download: Option<Download>,
    next: Option<ReadNext>,
    buffer: Vec<u8>,
    offset: usize,
}
impl Reader {
    pub fn new(download: Download) -> Self {
        Self {
            download: Some(download),
            next: None,
            buffer: vec![],
            offset: 0,
        }
    }
}
impl AsyncRead for Reader {
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
            self.buffer.clear();
            if self.next.is_none() {
                let Some(mut download) = self.download.take() else {
                    return Poll::Ready(Ok(()));
                };
                self.next = Some(Box::pin(async move {
                    let bytes = download.next().await?;
                    Ok((download, bytes))
                }));
            }
            let result = match self.next.as_mut().expect("pending read").as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.next = None;
            match result {
                Ok((download, Some(bytes))) => {
                    self.download = Some(download);
                    self.buffer = bytes;
                    self.offset = 0;
                }
                Ok((_, None)) => return Poll::Ready(Ok(())),
                Err(error) => return Poll::Ready(Err(std::io::Error::other(error))),
            }
        }
    }
}

#[cfg(test)]
mod tests;
mod upload;
pub(crate) use upload::upload;
