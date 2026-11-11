use super::*;
use rusqlite::OptionalExtension;

pub(super) struct ActiveScan {
    id: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    final_page_saved: bool,
}

impl ActiveScan {
    pub(super) fn request_stop(&self) {
        if !self.final_page_saved {
            self.cancel.send_replace(true);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scan {
    pub id: String,
    author_id: Option<String>,
    author_name: Option<String>,
    status: String,
    pages: i64,
    videos: i64,
    albums: i64,
    skipped: i64,
    error: Option<String>,
    batch: Option<Batch>,
    pause_reason: Option<String>,
    remaining: usize,
    completed: usize,
    failed: usize,
    pending_downloads: usize,
    already_local: usize,
    resume_page: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadMode {
    Full,
    Incremental,
}

#[derive(Serialize)]
pub struct AuthorLink {
    author_id: String,
    canonical_url: String,
    scan: Option<Scan>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum DownloadLink {
    Work(Box<Task>),
    Author(Box<AuthorLink>),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IdentifiedLink {
    Work { canonical_url: String },
    Author { author_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    queued: i64,
    skipped: i64,
}

#[derive(Deserialize)]
struct Page {
    author_id: String,
    page: u64,
    items: Vec<Post>,
    skipped: u64,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Post {
    video_id: String,
    kind: String,
    title: String,
    author: String,
    author_id: String,
    published_at: Option<i64>,
}

impl Post {
    fn identity(&self) -> Result<Value> {
        if !matches!(self.kind.as_str(), "video" | "image_album")
            || self.title.chars().count() > 512
            || self.author.chars().count() > 256
        {
            bail!("ENGINE_PROTOCOL_ERROR");
        }
        let path = if self.kind == "image_album" {
            "note"
        } else {
            "video"
        };
        let video = json!({
            "platform": "douyin", "video_id": self.video_id, "kind": self.kind, "resolver": "dtk",
            "canonical_url": format!("https://www.douyin.com/{path}/{}", self.video_id),
            "title": self.title, "author": self.author, "published_at": self.published_at,
        });
        serde_json::from_value::<Video>(video.clone())?.validate()?;
        Ok(video)
    }
}

pub(super) fn init(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS author_download_batches(
            id TEXT PRIMARY KEY, queued INTEGER NOT NULL, skipped INTEGER NOT NULL, pause_reason TEXT);
         CREATE TABLE IF NOT EXISTS author_download_batch_tasks(
            batch_id TEXT NOT NULL REFERENCES author_download_batches(id) ON DELETE CASCADE,
            task_id TEXT NOT NULL, PRIMARY KEY(batch_id,task_id));",
    ).context("DATABASE_ERROR")?;
    let has_legacy: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='author_scans')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy {
        db.execute_batch(
            "INSERT OR IGNORE INTO author_download_batches(id,queued,skipped,pause_reason)
             SELECT id,json_extract(batch,'$.queued'),COALESCE(json_extract(batch,'$.skipped'),0),pause_reason
             FROM author_scans WHERE json_valid(batch) AND json_extract(batch,'$.queued') > 0;
             INSERT OR IGNORE INTO author_download_batch_tasks(batch_id,task_id)
             SELECT scan_id,task_id FROM author_batch_items
             JOIN author_download_batches ON id=scan_id;
             DROP TABLE IF EXISTS author_batch_items;
             DROP TABLE IF EXISTS author_scan_cursors;
             DROP TABLE IF EXISTS author_scan_items;
             DROP TABLE IF EXISTS author_scan_options;
             DROP TABLE IF EXISTS author_targets;
             DROP TABLE IF EXISTS author_posts;
             DROP TABLE author_scans;",
        ).context("DATABASE_ERROR")?;
    }
    Ok(())
}

impl Scan {
    fn new(id: &str, status: &str) -> Self {
        Self {
            id: id.to_owned(),
            status: status.to_owned(),
            author_id: None,
            author_name: None,
            pages: 0,
            videos: 0,
            albums: 0,
            skipped: 0,
            error: None,
            batch: None,
            pause_reason: None,
            remaining: 0,
            completed: 0,
            failed: 0,
            pending_downloads: 0,
            already_local: 0,
            resume_page: None,
        }
    }
}

pub(super) struct ScanDraft {
    scan: Scan,
    scope: Option<String>,
    writer: ScanWriter,
}

fn draft<'a>(inner: &'a Inner, id: &str) -> Result<&'a ScanDraft> {
    inner
        .author_draft
        .as_ref()
        .filter(|draft| draft.scan.id == id)
        .context("AUTHOR_SCAN_NOT_FOUND")
}

fn read_batch(inner: &Inner, id: &str) -> Result<Scan> {
    let mut scan = Scan::new(id, "completed");
    let (batch, pause_reason) = inner
        .db
        .query_row(
            "SELECT queued,skipped,pause_reason FROM author_download_batches WHERE id=?",
            [id],
            |row| {
                Ok((
                    Batch {
                        queued: row.get(0)?,
                        skipped: row.get(1)?,
                    },
                    row.get(2)?,
                ))
            },
        )
        .optional()?
        .context("AUTHOR_SCAN_NOT_FOUND")?;
    scan.batch = Some(batch);
    scan.pause_reason = pause_reason;
    let mut query = inner
        .db
        .prepare("SELECT task_id FROM author_download_batch_tasks WHERE batch_id=?")?;
    let linked = query
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    for task in inner
        .tasks
        .iter()
        .filter(|task| linked.contains(&task.id) || batch_id(task) == Some(id))
    {
        match task.status {
            _ if download_in_progress(task) => scan.remaining += 1,
            Status::Completed if batch_id(task) != Some(id) && task.error.is_some() => {
                scan.failed += 1
            }
            Status::Completed => scan.completed += 1,
            Status::Failed | Status::Canceled => scan.failed += 1,
            _ => {}
        }
    }
    Ok(scan)
}

fn scan_progress(inner: &Inner, draft: &ScanDraft) -> Scan {
    let mut scan = draft.scan.clone();
    if scan.status == "partial"
        && draft.scope.is_some()
        && draft.writer.cursor.is_some()
        && scan.pages < 1000
    {
        scan.resume_page = Some(scan.pages + 1);
    }
    if scan.status != "scanning" {
        let tasks = indexed_tasks(&inner.tasks);
        for post in &draft.writer.posts {
            let matches = tasks
                .get(post.video_id.as_str())
                .map_or(&[][..], Vec::as_slice);
            if matches.iter().any(|task| archived_post(task, post)) {
                scan.already_local += 1;
            } else if !matches
                .iter()
                .any(|task| download_in_progress(task) || failed_post(task, post))
            {
                scan.pending_downloads += 1;
            }
        }
    }
    scan
}

fn current_scope(db: &Connection) -> Result<String> {
    db.query_row(
        "SELECT scope FROM douyin_author_scope WHERE id=1",
        [],
        |row| row.get(0),
    )
    .context("DATABASE_ERROR")
}

fn profile_scope(db: &Connection, profiles: &Value) -> Result<Option<String>> {
    profiles["douyin"]["cookie"]
        .as_str()
        .is_some_and(|cookie| !cookie.is_empty())
        .then(|| current_scope(db))
        .transpose()
}

fn indexed_tasks(tasks: &[Task]) -> HashMap<&str, Vec<&Task>> {
    let mut existing = HashMap::<_, Vec<_>>::new();
    for task in tasks {
        if let Some(video) = task
            .video
            .as_ref()
            .filter(|video| video["platform"] == "douyin")
        {
            if let Some(key) = video["video_id"].as_str() {
                existing.entry(key).or_default().push(task);
            }
        }
    }
    existing
}

#[derive(Default)]
struct ScanWriter {
    author: Option<String>,
    pages: u64,
    cursors: HashSet<String>,
    cursor: Option<String>,
    ended: bool,
    seen: HashSet<String>,
    posts: Vec<Post>,
}

fn ensure_scan_idle(inner: &Inner) -> Result<()> {
    if inner.closing
        || inner.author_scan.is_some()
        || inner
            .tasks
            .iter()
            .any(|task| batch_id(task).is_some() && download_in_progress(task))
    {
        bail!("TASK_BUSY");
    }
    Ok(())
}

fn valid_author(id: &str) -> bool {
    (10..=256).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl ScanWriter {
    fn page(&mut self, scan: &mut Scan, value: Value) -> Result<()> {
        let page: Page = serde_json::from_value(value).context("ENGINE_PROTOCOL_ERROR")?;
        if self.ended
            || page.page != self.pages + 1
            || !valid_author(&page.author_id)
            || self
                .author
                .as_ref()
                .is_some_and(|author| author != &page.author_id)
            || page.page > 1000
            || page.skipped > 200
            || page.items.len() as u64 + page.skipped > 200
        {
            bail!("ENGINE_PROTOCOL_ERROR");
        }
        match (page.has_more, page.next_cursor.as_deref()) {
            (true, Some(cursor))
                if !cursor.is_empty()
                    && cursor != "0"
                    && cursor.len() <= 32
                    && cursor.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                if self.cursors.contains(cursor) {
                    bail!("AUTHOR_CURSOR_INVALID");
                }
            }
            (false, None) => {}
            _ => bail!("AUTHOR_CURSOR_INVALID"),
        }
        for post in &page.items {
            if post.author_id != page.author_id {
                bail!("ENGINE_PROTOCOL_ERROR");
            }
            post.identity()?;
        }
        for post in page.items {
            if self.seen.insert(post.video_id.clone()) {
                if post.kind == "video" {
                    scan.videos += 1;
                } else {
                    scan.albums += 1;
                }
                if scan.author_name.is_none() {
                    scan.author_name = Some(post.author.clone());
                }
                self.posts.push(post);
            }
        }
        scan.author_id = Some(page.author_id.clone());
        scan.pages = page.page as i64;
        scan.skipped += page.skipped as i64;
        if let Some(cursor) = &page.next_cursor {
            self.cursors.insert(cursor.clone());
        }
        self.author = Some(page.author_id);
        self.pages = page.page;
        self.cursor = page.next_cursor;
        self.ended = !page.has_more;
        Ok(())
    }
}

pub(super) fn batch_id(task: &Task) -> Option<&str> {
    task.video.as_ref()?["author_scan_id"].as_str()
}

pub(super) fn batch_author_id(video: &Value) -> Option<&str> {
    let author = video["batch_author_id"].as_str()?;
    (video["platform"] == "douyin" && valid_author(author)).then_some(author)
}

fn archived(task: &Task) -> bool {
    task.status == Status::Completed
        && task.path.as_deref().is_some_and(|path| {
            if task.files.is_empty() {
                crate::files::video_recovery_valid(path, task.bytes)
            } else {
                validate_album(path, &task.files).is_ok_and(|bytes| bytes == task.bytes)
            }
        })
}

fn archived_post(task: &Task, post: &Post) -> bool {
    task.video
        .as_ref()
        .is_some_and(|video| video["kind"].as_str().unwrap_or("video") == post.kind)
        && (task.files.is_empty() == (post.kind == "video"))
        && archived(task)
}

fn failed_post(task: &Task, post: &Post) -> bool {
    task.download_requested
        && task.status == Status::Failed
        && task
            .video
            .as_ref()
            .is_some_and(|video| video["kind"].as_str().unwrap_or("video") == post.kind)
}

pub(super) fn paused(db: &Connection) -> Result<HashSet<String>> {
    Ok(db
        .prepare("SELECT id FROM author_download_batches WHERE pause_reason IS NOT NULL")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(super) fn downloaded_metadata(previous: &Value, result: &Value) -> Result<Value> {
    let mut video = result["metadata"].clone();
    serde_json::from_value::<Video>(video.clone())
        .context("INVALID_ENGINE_RESULT")?
        .validate()?;
    if ["platform", "video_id", "canonical_url", "resolver", "kind"]
        .iter()
        .any(|key| video[*key] != previous[*key])
        || video["author_profile"]["id"] != previous["batch_author_id"]
    {
        bail!("INVALID_ENGINE_RESULT");
    }
    if video["kind"] == "image_album" {
        let images = video["images"]
            .as_array()
            .context("INVALID_ENGINE_RESULT")?;
        if images.is_empty() || result["files"].as_array().map(Vec::len) != Some(images.len()) {
            bail!("INCOMPLETE_MEDIA");
        }
    } else {
        if !video["selected_format_id"].is_string()
            || video["selected_format_id"] != result["format_id"]
        {
            bail!("INVALID_ENGINE_RESULT");
        }
        video["filesize"] = result["bytes"].clone();
        video["filesize_approx"] = Value::Null;
    }
    video["cover_url"] = Value::Null;
    video["author_scan_id"] = previous["author_scan_id"].clone();
    video["batch_author_id"] = previous["batch_author_id"].clone();
    video["author_scope"] = previous["author_scope"].clone();
    Ok(video)
}

impl Core {
    pub async fn resolve_download_link(
        self: &Arc<Self>,
        input: String,
        resolver: String,
    ) -> Result<DownloadLink> {
        if input.trim().is_empty() || input.len() > 8192 {
            bail!("INVALID_INPUT");
        }
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.author_scan.is_none() {
                inner.author_draft = None;
            }
        }
        let route = parse_cache::share_url(&input)
            .and_then(|url| url::Url::parse(&url).ok())
            .filter(|url| match url.host_str() {
                Some("v.douyin.com") => true,
                Some("douyin.com" | "www.douyin.com") => url.path().starts_with("/user/"),
                Some("www.iesdouyin.com") => url.path().starts_with("/share/user/"),
                _ => false,
            });
        if let Some(url) = route {
            let short = url.host_str() == Some("v.douyin.com");
            if short {
                let cached = parse_cache::find(&self.inner.lock().unwrap().db, &input, &resolver)?;
                if cached.is_some() {
                    return self
                        .resolve_with_resolver(input, resolver)
                        .map(|task| DownloadLink::Work(Box::new(task)));
                }
            }
            let (send, receive) = tokio::sync::oneshot::channel();
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.closing {
                    bail!("CANCELED");
                }
                let profile = if short {
                    let profile = settings::douyin_snapshot(&inner.settings)?;
                    json!({
                        "user_agent": profile["douyin"]["user_agent"],
                        "proxy": profile["douyin"]["proxy"],
                    })
                } else {
                    json!({})
                };
                let params = json!({"input": input, "profile": profile});
                let mut cancel = inner.backup_cancel.subscribe();
                let core = self.clone();
                inner.workers.retain(|worker| !worker.is_finished());
                inner.workers.push(tokio::spawn(async move {
                    let result = PythonBridge::operation_cancellable(
                        &core.program,
                        &core.args,
                        &Uuid::new_v4().to_string(),
                        "identify_douyin_link",
                        params,
                        |_| {},
                        async {
                            let _ = cancel.wait_for(|value| *value).await;
                        },
                    )
                    .await;
                    let _ = send.send(result);
                }));
            }
            let result = receive.await.context("CANCELED")??;
            match serde_json::from_value(result["result"].clone())
                .context("INVALID_ENGINE_RESULT")?
            {
                IdentifiedLink::Work { canonical_url } => {
                    return self
                        .resolve_work(
                            ResolveInput {
                                original: input,
                                verified_url: Some(canonical_url),
                            },
                            resolver,
                        )
                        .map(|task| DownloadLink::Work(Box::new(task)));
                }
                IdentifiedLink::Author { author_id } => {
                    if !valid_author(&author_id) {
                        bail!("INVALID_ENGINE_RESULT");
                    }
                    let mut inner = self.inner.lock().unwrap();
                    if inner.closing {
                        bail!("CANCELED");
                    }
                    let canonical_url = format!("https://www.douyin.com/user/{author_id}");
                    let scan = self.begin_author_scan(
                        &mut inner,
                        canonical_url.clone(),
                        Some(&author_id),
                    )?;
                    return Ok(DownloadLink::Author(Box::new(AuthorLink {
                        canonical_url,
                        author_id,
                        scan: Some(scan),
                    })));
                }
            }
        }
        self.resolve_with_resolver(input, resolver)
            .map(|task| DownloadLink::Work(Box::new(task)))
    }

    #[cfg(test)]
    fn start_author_scan(self: &Arc<Self>, input: String) -> Result<Scan> {
        let mut inner = self.inner.lock().unwrap();
        self.begin_author_scan(&mut inner, input, None)
    }

    fn begin_author_scan(
        self: &Arc<Self>,
        inner: &mut Inner,
        input: String,
        author_id: Option<&str>,
    ) -> Result<Scan> {
        ensure_scan_idle(inner)?;
        let profiles = settings::douyin_snapshot(&inner.settings)?;
        let scope = profile_scope(&inner.db, &profiles)?;
        let mut scan = Scan::new(&Uuid::new_v4().to_string(), "scanning");
        scan.author_id = author_id.map(str::to_owned);
        let writer = ScanWriter {
            author: scan.author_id.clone(),
            ..ScanWriter::default()
        };
        inner.author_draft = Some(ScanDraft {
            scan,
            scope,
            writer,
        });
        Ok(self.launch_author_scan(inner, input, profiles))
    }

    pub fn recover_author_scan(self: &Arc<Self>, id: &str, reset: bool) -> Result<Scan> {
        let mut inner = self.inner.lock().unwrap();
        ensure_scan_idle(&inner)?;
        let session = draft(&inner, id)?;
        if session.scan.status != "partial" {
            bail!("INVALID_STATE");
        }
        let author = session.scan.author_id.clone().context("INVALID_STATE")?;
        let input = format!("https://www.douyin.com/user/{author}");
        if reset {
            return self.begin_author_scan(&mut inner, input, Some(&author));
        }
        let profiles = settings::douyin_snapshot(&inner.settings)?;
        if session.scope != profile_scope(&inner.db, &profiles)? {
            bail!("AUTHOR_SCOPE_CHANGED");
        }
        if session.scope.is_none() || session.writer.cursor.is_none() || session.scan.pages >= 1000
        {
            return self.begin_author_scan(&mut inner, input, Some(&author));
        }
        Ok(self.launch_author_scan(&mut inner, input, profiles))
    }

    pub fn discard_author_scan(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if draft(&inner, id)?.scan.status == "scanning" {
            bail!("INVALID_STATE");
        }
        inner.author_draft = None;
        Ok(())
    }

    fn launch_author_scan(
        self: &Arc<Self>,
        inner: &mut Inner,
        input: String,
        profiles: Value,
    ) -> Scan {
        let session = inner.author_draft.as_mut().expect("scan draft initialized");
        session.scan.status = "scanning".into();
        session.scan.error = None;
        let initial = session.scan.clone();
        let id = initial.id.clone();
        let params = json!({
            "input": input, "profile": profiles["douyin"],
            "cursor": session.writer.cursor, "completed_pages": session.writer.pages,
        });
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, completion) = watch::channel(false);
        inner.author_scan = Some(ActiveScan {
            id: id.clone(),
            cancel: cancel.clone(),
            done: completion,
            final_page_saved: false,
        });
        let core = self.clone();
        inner.workers.retain(|worker| !worker.is_finished());
        inner.workers.push(tokio::spawn(async move {
            let mut page_error = None;
            let result = PythonBridge::operation_cancellable(
                &core.program,
                &core.args,
                &id,
                "scan_author_posts",
                params,
                |value| {
                    if page_error.is_none() {
                        let mut inner = core.inner.lock().unwrap();
                        let session = inner.author_draft.as_mut().expect("active scan draft");
                        if let Err(error) = session.writer.page(&mut session.scan, value) {
                            page_error = Some(error);
                            cancel.send_replace(true);
                        } else {
                            let final_page = session.writer.ended;
                            if let Some(active) = inner.author_scan.as_mut() {
                                active.final_page_saved = final_page;
                            }
                        }
                    }
                },
                async {
                    let _ = cancellation.wait_for(|value| *value).await;
                },
            )
            .await;
            let mut inner = core.inner.lock().unwrap();
            let session = inner.author_draft.as_mut().expect("active scan draft");
            let (status, error) = match (result, page_error) {
                (_, Some(error)) => ("partial", Some(error.to_string())),
                (Ok(result), None)
                    if session.writer.ended
                        && result["result"]["pages"].as_u64() == Some(session.writer.pages)
                        && !*cancel.borrow() =>
                {
                    ("completed", None)
                }
                (Err(error), None) => ("partial", Some(error.to_string())),
                _ if *cancel.borrow() => ("partial", Some("CANCELED".into())),
                _ => ("partial", Some("ENGINE_PROTOCOL_ERROR".into())),
            };
            session.scan.status = status.into();
            session.scan.error = error;
            inner.author_scan = None;
            core.dispatch(&mut inner);
            done.send_replace(true);
        }));
        initial
    }

    pub fn author_scan(&self, id: &str) -> Result<Scan> {
        let inner = self.inner.lock().unwrap();
        if let Ok(session) = draft(&inner, id) {
            return Ok(scan_progress(&inner, session));
        }
        read_batch(&inner, id)
    }

    pub fn latest_author_scan(&self) -> Result<Option<Scan>> {
        let inner = self.inner.lock().unwrap();
        inner
            .tasks
            .iter()
            .filter(|task| task.status == Status::Queued)
            .filter_map(batch_id)
            .find(|id| inner.author_paused.contains(*id))
            .map(|id| read_batch(&inner, id))
            .transpose()
    }

    pub async fn cancel_author_scan(&self, id: &str) -> Result<Scan> {
        let completion = {
            let inner = self.inner.lock().unwrap();
            inner
                .author_scan
                .as_ref()
                .filter(|active| active.id == id)
                .map(|active| {
                    active.request_stop();
                    active.done.clone()
                })
        };
        if let Some(mut completion) = completion {
            let _ = completion.wait_for(|done| *done).await;
        }
        self.author_scan(id)
    }

    pub fn confirm_author_scan(
        self: &Arc<Self>,
        id: &str,
        allow_partial: bool,
        mode: DownloadMode,
    ) -> Result<Batch> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("TASK_BUSY");
        }
        let session = match draft(&inner, id) {
            Ok(session) => session,
            Err(_) => {
                return read_batch(&inner, id)?
                    .batch
                    .context("AUTHOR_SCAN_NOT_FOUND")
            }
        };
        let scan = &session.scan;
        if scan.status != "completed" && !(allow_partial && scan.status == "partial") {
            bail!("INVALID_STATE");
        }
        ensure_scan_idle(&inner)?;
        let author = scan.author_id.as_deref().context("INVALID_STATE")?;
        let scope = &session.scope;
        let profiles = Arc::new(settings::douyin_snapshot(&inner.settings)?);
        if *scope != profile_scope(&inner.db, &profiles)? {
            bail!("AUTHOR_SCOPE_CHANGED");
        }
        let existing = indexed_tasks(&inner.tasks);
        let pending_cleanup = {
            let mut query = inner.db.prepare(
                "SELECT task_id FROM task_staging UNION SELECT task_id FROM commit_intents
                 UNION SELECT task_id FROM download_replacements",
            )?;
            let ids = query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<_>>>()?;
            ids
        };
        let mut queued = Vec::new();
        let mut linked = HashSet::new();
        let mut reused = HashSet::new();
        let mut skipped = 0;
        let base_order = inner
            .tasks
            .iter()
            .filter_map(|task| task.queue_order)
            .max()
            .unwrap_or(0);
        let default_directory = &inner.settings.download_directory;
        let requested_at = chrono::Utc::now().timestamp_millis().max(0) as u64;
        for post in &session.writer.posts {
            let matches = existing
                .get(post.video_id.as_str())
                .map_or(&[][..], Vec::as_slice);
            let archived = matches
                .iter()
                .copied()
                .filter(|task| archived_post(task, post))
                .max_by_key(|task| (task.completed_at, task.insertion_id));
            if mode == DownloadMode::Incremental && archived.is_some() {
                skipped += 1;
                continue;
            }
            if let Some(task) = matches.iter().find(|task| download_in_progress(task)) {
                linked.insert(task.id.clone());
                skipped += 1;
                continue;
            }
            if archived.is_none() && matches.iter().any(|task| failed_post(task, post)) {
                skipped += 1;
                continue;
            }
            let original = archived.cloned();
            let directory = if let Some(original) = original.as_ref() {
                if inner.active.contains_key(&original.id)
                    || inner.transcription.as_ref().is_some_and(|active| {
                        active.snapshot.task_id == original.id
                            || original.asset_id.as_deref()
                                == Some(active.snapshot.task_id.as_str())
                    })
                {
                    bail!("TASK_BUSY");
                }
                super::cleanup_task_staging(&inner.db, &original.id)?;
                redownload::directory(original)?
            } else {
                default_directory.clone()
            };
            let previous = original.as_ref().or_else(|| {
                matches.iter().copied().find(|task| {
                    task.status == Status::Canceled
                        && task.asset_id.is_none()
                        && task.path.is_none()
                        && task.error.as_deref() != Some("REDOWNLOAD_RESTORE_FAILED")
                        && !inner.active.contains_key(&task.id)
                        && !inner.downloads.contains(&task.id)
                        && !pending_cleanup.contains(&task.id)
                        && task.video.as_ref().is_some_and(|video| {
                            video["batch_author_id"] == author
                                && video["kind"] == post.kind
                                && video["author_scope"].as_str() == scope.as_deref()
                        })
                })
            });
            let mut video = post.identity()?;
            video["author_scan_id"] = id.into();
            video["batch_author_id"] = author.into();
            video["author_scope"] = json!(scope);
            let task_id =
                previous.map_or_else(|| Uuid::new_v4().to_string(), |task| task.id.clone());
            if previous.is_some() {
                reused.insert(task_id.clone());
            }
            linked.insert(task_id.clone());
            let task = Task {
                id: task_id,
                insertion_id: previous.map_or(0, |task| task.insertion_id),
                asset_id: original.as_ref().and_then(|task| task.asset_id.clone()),
                revision: previous.map_or(1, |task| task.revision + 1),
                status: Status::Queued,
                resolve_progress: None,
                video: Some(video),
                bytes: 0,
                total_bytes: None,
                total_bytes_estimate: None,
                speed_bps: None,
                eta_seconds: None,
                merging: false,
                error: None,
                path: None,
                created_at: previous.map_or_else(now, |task| task.created_at),
                download_requested_at: Some(requested_at),
                completed_at: None,
                canceling: false,
                file_missing: false,
                queue_order: Some(base_order + queued.len() as u64 + 1),
                download_requested: true,
                duplicate_confirmed: false,
                cover_path: None,
                cover_error: None,
                files: Vec::new(),
                completed_items: None,
                total_items: None,
                cache_hit: false,
                cached_at: None,
                cover_cached: false,
                telegram_link: None,
            };
            queued.push((task, original, directory));
        }
        let batch = Batch {
            queued: queued.len() as i64,
            skipped,
        };
        let tx = inner.db.transaction().context("DATABASE_ERROR")?;
        for (task, original, _) in &mut queued {
            if let Some(original) = original {
                redownload::remember(&tx, original, None)?;
            }
            persist_published(&tx, task)?;
        }
        if !queued.is_empty() {
            tx.execute(
                "INSERT INTO author_download_batches(id,queued,skipped) VALUES(?,?,?)",
                params![id, batch.queued, batch.skipped],
            )?;
            for task in linked {
                tx.execute(
                    "INSERT INTO author_download_batch_tasks(batch_id,task_id) VALUES(?,?)",
                    params![id, task],
                )?;
            }
        }
        tx.commit().context("DATABASE_ERROR")?;
        inner.author_draft = None;
        let template = inner.settings.filename_template.clone();
        let mut published = Vec::with_capacity(queued.len());
        for (task, _, directory) in queued {
            inner.active.insert(
                task.id.clone(),
                Active {
                    telegram: false,
                    download_after_resolve: false,
                    profiles: profiles.clone(),
                    directory,
                    filename_template: template.clone(),
                    cancel: watch::channel(false).0,
                    done: watch::channel(true).1,
                },
            );
            published.push(task);
        }
        inner.tasks.retain(|task| !reused.contains(&task.id));
        inner.tasks.splice(0..0, published.iter().rev().cloned());
        for task in published {
            super::emit_task(&inner.db, task, &*self.emit);
        }
        self.dispatch(&mut inner);
        Ok(batch)
    }

    pub(super) async fn wait_author_detail(&self) {
        let last = *self.author_download_finished.lock().unwrap();
        if let Some(finished) = last {
            tokio::time::sleep_until((finished + Duration::from_millis(3500)).into()).await;
        }
    }

    pub(super) fn pause_author_on_error(&self, id: &str, error: &anyhow::Error) {
        let code = error.to_string();
        if !matches!(code.as_str(), "AUTHOR_LOGIN_REQUIRED" | "AUTHOR_API_LIMIT") {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.author_paused.insert(id.to_owned());
        if inner
            .db
            .execute(
                "INSERT INTO author_download_batches(id,queued,skipped,pause_reason) VALUES(?,0,0,?)\n                 ON CONFLICT(id) DO UPDATE SET pause_reason=excluded.pause_reason",
                params![id, code],
            )
            .is_err()
        {
            eprintln!("author_batch event=pause_save_failed");
        }
    }

    pub fn resume_author_batch(self: &Arc<Self>, id: &str) -> Result<Scan> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("TASK_BUSY");
        }
        if !inner.author_paused.contains(id) {
            return read_batch(&inner, id);
        }
        let profiles = Arc::new(settings::douyin_snapshot(&inner.settings)?);
        let pending = inner
            .tasks
            .iter()
            .filter(|task| batch_id(task) == Some(id) && task.status == Status::Queued)
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        inner.db.execute(
            "UPDATE author_download_batches SET pause_reason=NULL WHERE id=?",
            [id],
        )?;
        inner.author_paused.remove(id);
        for task in pending {
            if let Some(active) = inner.active.get_mut(&task) {
                active.profiles = profiles.clone();
            }
        }
        self.dispatch(&mut inner);
        read_batch(&inner, id)
    }

    pub(super) fn retry_author_download(self: &Arc<Self>, id: &str) -> Result<Option<Task>> {
        let mut inner = self.inner.lock().unwrap();
        let Some(task) = inner
            .tasks
            .iter()
            .find(|task| task.id == id && batch_id(task).is_some())
            .cloned()
        else {
            return Ok(None);
        };
        if inner.closing || inner.active.contains_key(id) || inner.downloads.contains(id) {
            bail!("TASK_BUSY");
        }
        if task.error.as_deref() == Some("REDOWNLOAD_RESTORE_FAILED") {
            bail!("REDOWNLOAD_RESTORE_FAILED");
        }
        if redownload::original(&inner.db, id)?.is_some() {
            bail!("TASK_BUSY");
        }
        if !matches!(task.status, Status::Failed | Status::Canceled) {
            bail!("INVALID_STATE");
        }
        let video = task.video.as_ref().context("INVALID_STATE")?;
        if inner.tasks.iter().any(|other| {
            other.id != id
                && other.video.as_ref().is_some_and(|other_video| {
                    other_video["platform"] == "douyin"
                        && other_video["video_id"] == video["video_id"]
                })
                && (download_in_progress(other) || archived(other))
        }) {
            bail!("ALREADY_DOWNLOADED");
        }
        super::cleanup_task_staging(&inner.db, id)?;
        if let Err(error) = self.queue_download(&mut inner, task, true, None) {
            inner.active.remove(id);
            return Err(error);
        }
        self.dispatch(&mut inner);
        Ok(inner
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .map(|task| super::authors::display_task(&inner.db, task)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(root: &Path, script: &str) -> Arc<Core> {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) {
            ".venv/Scripts/python.exe"
        } else {
            ".venv/bin/python"
        });
        Core::open(
            &root.join("tasks.sqlite"),
            root.join("downloads"),
            python.to_string_lossy().into_owned(),
            vec![project.join(script).to_string_lossy().into_owned()],
            |_| {},
        )
        .unwrap()
    }

    async fn scan_ready(core: &Arc<Core>, input: &str) -> Scan {
        scan_with_deadline(core, input, 10).await
    }

    async fn scan_with_deadline(core: &Arc<Core>, input: &str, seconds: u64) -> Scan {
        let initial = core.start_author_scan(input.into()).unwrap();
        wait_for_scan(core, &initial.id, seconds).await
    }

    async fn wait_for_scan(core: &Arc<Core>, id: &str, seconds: u64) -> Scan {
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                let scan = core.author_scan(id).unwrap();
                if scan.status != "scanning" {
                    return scan;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap()
    }

    fn set_cookie(core: &Core, cookie: &str) {
        core.save_profile(
            serde_json::from_value(json!({
                "platform": "douyin", "user_agent": "fixture", "cookie": cookie,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn author_link_starts_a_fresh_scan_after_partial_result() {
        let root = tempfile::tempdir().unwrap();
        let core = open(root.path(), "tests/fixtures/desktop_engine.py");
        set_cookie(&core, "sessionid=fixture");
        let link = "https://v.douyin.com/fixture-author/";
        let DownloadLink::Author(author) = core
            .resolve_download_link(link.into(), "yt-dlp".into())
            .await
            .unwrap()
        else {
            panic!("expected author");
        };
        assert_eq!(
            author.canonical_url,
            "https://www.douyin.com/user/fixture-author"
        );
        let initial = author.scan.unwrap();
        assert_eq!(initial.status, "scanning");
        let completed = wait_for_scan(&core, &initial.id, 10).await;
        assert_eq!(completed.status, "completed");
        let scan = scan_ready(&core, "partial").await;
        let DownloadLink::Author(author) = core
            .resolve_download_link(link.into(), "yt-dlp".into())
            .await
            .unwrap()
        else {
            panic!("expected author");
        };
        let fresh = author.scan.unwrap();
        assert_ne!(fresh.id, scan.id);
        assert_eq!((fresh.status.as_str(), fresh.pages), ("scanning", 0));
        assert!(core.author_scan(&scan.id).is_err());
        assert_eq!(
            wait_for_scan(&core, &fresh.id, 10).await.status,
            "completed"
        );

        set_cookie(&core, "sessionid=other-fixture");
        let DownloadLink::Author(author) = core
            .resolve_download_link(link.into(), "yt-dlp".into())
            .await
            .unwrap()
        else {
            panic!("expected author");
        };
        let next = author.scan.unwrap();
        assert_eq!(next.status, "scanning");
        assert_ne!(next.id, scan.id);
        assert_eq!(wait_for_scan(&core, &next.id, 10).await.status, "completed");

        assert!(matches!(
            core.resolve_download_link(
                "https://v.douyin.com/fixture-work/".into(),
                "yt-dlp".into(),
            )
            .await
            .unwrap(),
            DownloadLink::Work(_)
        ));
        core.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_waits_for_pending_douyin_link_identification() {
        let root = tempfile::tempdir().unwrap();
        let core = open(root.path(), "tests/fixtures/desktop_engine.py");
        let app = core.clone();
        let pending = tokio::spawn(async move {
            app.resolve_download_link("https://v.douyin.com/fixture-block/".into(), "auto".into())
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while core.inner.lock().unwrap().workers.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        core.shutdown().await;
        let error = pending
            .await
            .unwrap()
            .err()
            .expect("canceled identification");
        assert_eq!(error.to_string(), "CANCELED");
    }

    async fn wait_for_failure(core: &Core, id: &str) -> String {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                {
                    let inner = core.inner.lock().unwrap();
                    if let Some(task) = inner.tasks.iter().find(|task| {
                        batch_id(task) == Some(id)
                            && task.status == Status::Failed
                            && !inner.active.contains_key(&task.id)
                            && !inner.downloads.contains(&task.id)
                    }) {
                        assert_eq!(task.error.as_deref(), Some("IO_ERROR"));
                        return task.id.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn author_incremental_uses_current_results_and_failed_tasks_retry_from_list() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        set_cookie(&app, "sessionid=fixture");
        let initial = scan_ready(&app, "full").await;
        assert_eq!(initial.pending_downloads, 2);
        assert_eq!(
            app.confirm_author_scan(&initial.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            2
        );
        assert!(app.inner.lock().unwrap().author_draft.is_none());
        assert_eq!(finished(&app, &initial.id, 15).await.completed, 2);

        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;
        let next = scan_ready(&app, "new-video").await;
        assert_eq!((next.already_local, next.pending_downloads), (2, 1));
        assert_eq!(
            app.confirm_author_scan(&next.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            1
        );
        let failed_id = wait_for_failure(&app, &next.id).await;
        let visible = scan_ready(&app, "new-video").await;
        assert_eq!((visible.already_local, visible.pending_downloads), (2, 0));
        assert_eq!(
            app.confirm_author_scan(&visible.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            0
        );
        assert!(app.author_scan(&visible.id).is_err());
        app.shutdown().await;
        drop(app);

        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let next = scan_ready(&app, "new-album").await;
        assert_eq!((next.already_local, next.pending_downloads), (2, 1));
        assert_eq!(
            app.confirm_author_scan(&next.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            1
        );
        assert_eq!(finished(&app, &next.id, 15).await.completed, 1);
        assert_eq!(
            app.list()
                .iter()
                .find(|task| task.id == failed_id)
                .unwrap()
                .status,
            Status::Failed
        );
        assert_eq!(app.retry(&failed_id).unwrap().id, failed_id);
        tokio::time::timeout(Duration::from_secs(15), async {
            while !app
                .list()
                .iter()
                .any(|task| task.id == failed_id && archived(task))
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        app.shutdown().await;
    }

    #[test]
    fn author_migration_discards_history_and_preserves_confirmed_batches() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db.execute_batch(
            "CREATE TABLE author_scans(id TEXT PRIMARY KEY,batch TEXT,pause_reason TEXT);
             CREATE TABLE author_batch_items(scan_id TEXT REFERENCES author_scans(id),task_id TEXT);
             CREATE TABLE author_scan_items(scan_id TEXT REFERENCES author_scans(id),post TEXT);
             INSERT INTO author_scans VALUES('draft',NULL,NULL);
             INSERT INTO author_scans VALUES('queued','{\"queued\":1,\"skipped\":2}','AUTHOR_LOGIN_REQUIRED');
             INSERT INTO author_batch_items VALUES('queued','task');
             INSERT INTO author_scan_items VALUES('draft','old result');",
        ).unwrap();
        init(&db).unwrap();
        init(&db).unwrap();
        assert_eq!(paused(&db).unwrap(), HashSet::from(["queued".to_owned()]));
        let migrated: (i64, i64, String) = db
            .query_row(
                "SELECT queued,skipped,task_id FROM author_download_batches
             JOIN author_download_batch_tasks ON batch_id=id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(migrated, (1, 2, "task".into()));
        let history: i64 = db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('author_scans','author_scan_items','author_scan_cursors')",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(history, 0);
    }

    #[tokio::test]
    async fn author_download_rejects_cookie_changes_between_scan_and_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let browser_scan = scan_ready(&app, "full").await;
        set_cookie(&app, "sessionid=fixture");
        assert_eq!(
            app.confirm_author_scan(&browser_scan.id, false, DownloadMode::Incremental)
                .unwrap_err()
                .to_string(),
            "AUTHOR_SCOPE_CHANGED"
        );
        let complete = scan_ready(&app, "full").await;
        set_cookie(&app, "sessionid=fixture");
        assert_eq!(
            app.confirm_author_scan(&complete.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, &complete.id, 15).await.completed, 2);
        let current = scan_ready(&app, "full").await;
        assert_eq!(current.already_local, 2);
        set_cookie(&app, "sessionid=different");
        assert_eq!(
            app.confirm_author_scan(&current.id, false, DownloadMode::Full)
                .unwrap_err()
                .to_string(),
            "AUTHOR_SCOPE_CHANGED"
        );
        let fresh = scan_ready(&app, "full").await;
        assert_eq!((fresh.already_local, fresh.pending_downloads), (2, 0));
        app.shutdown().await;
    }

    async fn finished(core: &Core, id: &str, seconds: u64) -> Scan {
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                let scan = core.author_scan(id).unwrap();
                if scan.failed > 0 {
                    let codes = core
                        .list()
                        .into_iter()
                        .filter(|task| batch_id(task) == Some(id))
                        .filter_map(|task| task.error)
                        .collect::<Vec<_>>();
                    panic!("batch failed: {codes:?}");
                }
                if scan.remaining == 0 {
                    return scan;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn author_scan_queues_video_and_album_once_and_skips_valid_local_files() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let scan = scan_ready(&app, "full").await;
        assert_eq!(
            (scan.status.as_str(), scan.pages, scan.videos, scan.albums),
            ("completed", 2, 1, 1)
        );
        assert!(app.list().is_empty());
        assert_eq!(
            app.confirm_author_scan(&scan.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(
            app.confirm_author_scan(&scan.id, false, DownloadMode::Full)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(app.list().len(), 2);
        assert_eq!(finished(&app, &scan.id, 15).await.completed, 2);
        let tasks = app.list();
        let album = tasks.iter().find(|task| !task.files.is_empty()).unwrap();
        assert_eq!(album.files.len(), 2);
        assert!(album.files[0].video.is_some());
        assert!(tasks.iter().all(archived));
        let video = tasks.iter().find(|task| task.files.is_empty()).unwrap();
        let video_task_id = video.id.clone();
        let album_task_id = album.id.clone();
        let avatar_body = b"\x89PNG\r\n\x1a\nbatch-author-fixture";
        for task in &tasks {
            assert_eq!(
                task.video.as_ref().unwrap()["author_profile"]["avatar_cached"],
                true
            );
            assert_eq!(app.author_avatar(&task.id).unwrap(), avatar_body);
        }
        assert!(root.path().join("avatar-requested-501").is_file());
        assert!(!root.path().join("avatar-requested-601").exists());
        assert!(video.video.as_ref().unwrap()["selected_format_id"].is_string());
        assert!(video.cover_cached && video.cover_error.is_none());
        assert_eq!(
            app.cover(&video.id).unwrap(),
            b"\x89PNG\r\n\x1a\ndownloaded-fixture"
        );
        {
            let inner = app.inner.lock().unwrap();
            let preview = crate::parse_cache::get(&inner.db, "douyin", "501", "dtk")
                .unwrap()
                .unwrap();
            assert!(preview.has_cover);
            for field in ["author_scan_id", "batch_author_id", "author_scope"] {
                assert!(preview.video.get(field).is_none());
            }
        }
        let repeat = scan_ready(&app, "full").await;
        let result = app
            .confirm_author_scan(&repeat.id, false, DownloadMode::Incremental)
            .unwrap();
        assert_eq!((result.queued, result.skipped), (0, 2));
        app.shutdown().await;
        drop(app);

        let reopened = open(root.path(), "tests/fixtures/desktop_engine.py");
        assert_eq!(reopened.author_avatar(&video_task_id).unwrap(), avatar_body);
        assert_eq!(reopened.author_avatar(&album_task_id).unwrap(), avatar_body);
        reopened.remove_history(&video_task_id).unwrap();
        assert_eq!(reopened.author_avatar(&album_task_id).unwrap(), avatar_body);
        reopened.remove_history(&album_task_id).unwrap();
        assert!(!authors::has_avatar(
            &reopened.inner.lock().unwrap().db,
            video.video.as_ref().unwrap()
        ));
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn author_full_download_replaces_video_and_album_in_place() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let scan = scan_ready(&app, "full").await;
        app.confirm_author_scan(&scan.id, false, DownloadMode::Incremental)
            .unwrap();
        assert_eq!(finished(&app, &scan.id, 15).await.completed, 2);
        let originals = app.list();
        let old_bytes = originals
            .iter()
            .map(|task| {
                let path = task.path.as_ref().unwrap();
                let media = task
                    .files
                    .first()
                    .map_or_else(|| path.clone(), |file| path.join(&file.filename));
                std::fs::read(media).unwrap()
            })
            .collect::<Vec<_>>();
        std::fs::write(root.path().join("author-replacement-fixture"), b"new").unwrap();
        // Changing the configured folder must not redirect in-place replacements.
        let other_directory = root.path().join("new-downloads");
        app.inner.lock().unwrap().settings.download_directory = other_directory.clone();
        let scan = scan_ready(&app, "full").await;
        assert_eq!((scan.already_local, scan.pending_downloads), (2, 0));
        let original = &originals[0];
        let stale = original
            .path
            .as_ref()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("yingqu-{}-previous", original.id));
        std::fs::create_dir(&stale).unwrap();
        app.inner
            .lock()
            .unwrap()
            .db
            .execute(
                "INSERT INTO task_staging VALUES(?1,?2)",
                params![original.id, stale.to_string_lossy()],
            )
            .unwrap();
        let batch = app
            .confirm_author_scan(&scan.id, false, DownloadMode::Full)
            .unwrap();
        assert_eq!((batch.queued, batch.skipped), (2, 0));
        assert!(!stale.exists());
        assert_eq!(finished(&app, &scan.id, 15).await.completed, 2);
        let tasks = app.list();
        assert_eq!(tasks.len(), 2);
        for (original, body) in originals.iter().zip(old_bytes) {
            let task = tasks.iter().find(|task| task.id == original.id).unwrap();
            assert_eq!(task.path, original.path);
            assert!(archived(task));
            let path = task.path.as_ref().unwrap();
            let media = task
                .files
                .first()
                .map_or_else(|| path.clone(), |file| path.join(&file.filename));
            let replaced = std::fs::read(media).unwrap();
            assert!(replaced.starts_with(&body) && replaced.ends_with(b"replacement"));
            assert!(replaced.len() > body.len());
        }
        assert!(!other_directory.exists());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_retry_cleans_old_staging_before_reusing_task() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;

        let scan = scan_ready(&app, "full").await;
        app.confirm_author_scan(&scan.id, false, DownloadMode::Incremental)
            .unwrap();
        let id = wait_for_failure(&app, &scan.id).await;
        let downloads = root.path().join("downloads");
        let stale = downloads.join(format!("yingqu-{id}-previous"));
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("video.mp4.part"), b"partial").unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            inner.settings.download_directory = downloads;
            inner
                .db
                .execute(
                    "INSERT INTO task_staging VALUES(?1,?2)",
                    params![id, stale.to_string_lossy()],
                )
                .unwrap();
        }

        assert_eq!(app.retry(&id).unwrap().id, id);
        assert!(!stale.exists());
        tokio::time::timeout(Duration::from_secs(15), async {
            while !app
                .list()
                .iter()
                .any(|task| task.id == id && archived(task))
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_retry_preserves_original_when_redownload_rollback_failed() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;
        let scan = scan_ready(&app, "full").await;
        app.confirm_author_scan(&scan.id, false, DownloadMode::Incremental)
            .unwrap();
        let id = wait_for_failure(&app, &scan.id).await;
        let stage = root.path().join(format!("yingqu-{id}-preserved"));
        std::fs::create_dir(&stage).unwrap();
        let original = stage.join("replaced-original");
        std::fs::write(&original, b"original bytes").unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            inner
                .tasks
                .iter_mut()
                .find(|task| task.id == id)
                .unwrap()
                .error = Some("REDOWNLOAD_RESTORE_FAILED".into());
            inner
                .db
                .execute(
                    "INSERT INTO task_staging VALUES(?1,?2)",
                    params![id, stage.to_string_lossy()],
                )
                .unwrap();
        }
        assert_eq!(
            app.retry(&id)
                .err()
                .expect("retry must be blocked")
                .to_string(),
            "REDOWNLOAD_RESTORE_FAILED"
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"original bytes");
        assert!(app
            .inner
            .lock()
            .unwrap()
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_staging WHERE task_id=?)",
                [&id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_partial_scan_requires_explicit_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let scan = scan_ready(&app, "partial").await;
        assert_eq!((scan.status.as_str(), scan.pages), ("partial", 1));
        assert_eq!(scan.error.as_deref(), Some("ACCESS_DENIED"));
        assert!(app
            .confirm_author_scan(&scan.id, false, DownloadMode::Full)
            .is_err());
        assert!(app.list().is_empty());
        assert_eq!(
            app.confirm_author_scan(&scan.id, true, DownloadMode::Incremental)
                .unwrap()
                .queued,
            2
        );
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_stop_after_last_page_waits_for_verified_completion() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        for pages in [1, 2] {
            let gate = root.path().join(format!("finish-scan-{pages}"));
            let initial = app
                .start_author_scan(format!("finish-after-last:{}", gate.display()))
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while app.author_scan(&initial.id).unwrap().pages == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();

            let stop = app.cancel_author_scan(&initial.id);
            tokio::pin!(stop);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), stop.as_mut())
                    .await
                    .is_err()
            );
            std::fs::write(&gate, pages.to_string()).unwrap();
            let scan = tokio::time::timeout(Duration::from_secs(3), stop)
                .await
                .unwrap()
                .unwrap();
            assert_eq!((scan.pages, scan.videos, scan.albums), (1, 1, 1));
            if pages == 1 {
                assert_eq!(scan.status, "completed");
                assert!(scan.error.is_none());
            } else {
                assert_eq!(scan.status, "partial");
                assert_eq!(scan.error.as_deref(), Some("ENGINE_PROTOCOL_ERROR"));
            }
        }
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_scan_continues_only_until_discard_or_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        set_cookie(&app, "sessionid=fixture");
        let initial = app.start_author_scan("stop-after-first".into()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while app.author_scan(&initial.id).unwrap().pages == 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        let stopped = app.cancel_author_scan(&initial.id).await.unwrap();
        assert_eq!((stopped.status.as_str(), stopped.pages), ("partial", 1));
        assert_eq!(stopped.error.as_deref(), Some("CANCELED"));
        assert_eq!(stopped.resume_page, Some(2));
        let continuing = app.recover_author_scan(&stopped.id, false).unwrap();
        assert_eq!(
            (continuing.id.as_str(), continuing.pages),
            (stopped.id.as_str(), 1)
        );
        let result = wait_for_scan(&app, &continuing.id, 10).await;
        assert_eq!((result.status.as_str(), result.pages), ("completed", 2));
        assert_eq!((result.videos, result.albums, result.skipped), (1, 2, 1));
        assert!(app.list().is_empty());
        app.discard_author_scan(&result.id).unwrap();
        assert!(app.author_scan(&result.id).is_err());
        let unconfirmed = scan_ready(&app, "partial").await;
        app.shutdown().await;
        assert!(app.author_scan(&unconfirmed.id).is_err());
        drop(app);
        let reopened = open(root.path(), "tests/fixtures/desktop_engine.py");
        assert!(reopened
            .recover_author_scan(&unconfirmed.id, false)
            .is_err());
        assert_eq!(reopened.start_author_scan("full".into()).unwrap().pages, 0);
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn author_scan_reset_starts_clean_and_continue_checks_cookie_scope() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        set_cookie(&app, "sessionid=fixture");
        let stopped = scan_ready(&app, "partial").await;
        set_cookie(&app, "sessionid=changed");
        assert_eq!(
            app.recover_author_scan(&stopped.id, false)
                .unwrap_err()
                .to_string(),
            "AUTHOR_SCOPE_CHANGED"
        );
        let reset = app.recover_author_scan(&stopped.id, true).unwrap();
        assert_ne!(reset.id, stopped.id);
        assert_eq!((reset.pages, reset.videos, reset.albums), (0, 0, 0));
        let result = wait_for_scan(&app, &reset.id, 10).await;
        assert_eq!((result.videos, result.albums, result.skipped), (1, 1, 0));
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_scan_without_verified_cookie_scope_restarts_safely() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let stopped = scan_ready(&app, "partial").await;
        assert_eq!((stopped.pages, stopped.resume_page), (1, None));
        let reset = app.recover_author_scan(&stopped.id, false).unwrap();
        assert_ne!(reset.id, stopped.id);
        assert_eq!(reset.pages, 0);
        assert_eq!(wait_for_scan(&app, &reset.id, 10).await.status, "completed");
        app.shutdown().await;
    }

    #[test]
    fn author_writer_keeps_valid_pages_and_rejects_repeated_cursors() {
        let mut scan = Scan::new("test", "scanning");
        let mut writer = ScanWriter::default();
        for page in 1..=41 {
            writer
                .page(
                    &mut scan,
                    json!({"author_id":"fixture-author", "page":page,
                "items":[], "skipped":0, "has_more":true, "next_cursor":page.to_string()}),
                )
                .unwrap();
        }
        assert!(writer
            .page(
                &mut scan,
                json!({"author_id":"fixture-author","page":42,
            "items":[],"skipped":0,"has_more":true,"next_cursor":"41"})
            )
            .is_err());
        assert_eq!(scan.pages, 41);
    }

    #[tokio::test]
    #[ignore = "requires explicitly authorized existing Douyin settings and two verified sample IDs"]
    async fn author_live_video_and_album_commit() {
        let root = tempfile::tempdir().unwrap();
        let app = open_author_live(root.path());
        let author = std::env::var("YINGQU_AUTHOR_TEST_ID").unwrap();
        let video = std::env::var("YINGQU_AUTHOR_TEST_VIDEO").unwrap();
        let album = std::env::var("YINGQU_AUTHOR_TEST_ALBUM").unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            let mut scan = Scan::new("sample", "completed");
            let mut writer = ScanWriter::default();
            writer.page(&mut scan, json!({
                "author_id":author,"page":1,"skipped":0,"has_more":false,"next_cursor":null,
                "items":[
                    {"video_id":video,"kind":"video","title":"Sample video","author":"Sample","author_id":author},
                    {"video_id":album,"kind":"image_album","title":"Sample album","author":"Sample","author_id":author}
                ]
            })).unwrap();
            inner.author_draft = Some(ScanDraft {
                scan,
                writer,
                scope: Some(current_scope(&inner.db).unwrap()),
            });
        }
        assert_eq!(
            app.confirm_author_scan("sample", false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, "sample", 120).await.completed, 2);
        let (bio, links): (Option<String>, i64) = app.inner.lock().unwrap().db.query_row(
            "SELECT a.bio,count(t.task_id) FROM authors a JOIN task_authors t USING(platform,author_id)
             WHERE a.platform='douyin' AND a.author_id=? GROUP BY a.author_id",
            [&author],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(links, 2);
        println!(
            "author_bio_chars={}",
            bio.as_deref().map_or(0, |text| text.chars().count())
        );
        for task in app.list() {
            assert!(archived(&task));
            let video = task.video.unwrap();
            assert_eq!(video["author_profile"]["bio"].as_str(), bio.as_deref());
            println!(
                "{}: {} bytes, {} images",
                video["kind"],
                task.bytes,
                task.files.len()
            );
        }
        app.shutdown().await;
    }

    fn open_author_live(root: &Path) -> Arc<Core> {
        let app = open(root, "python-engine/bridge.py");
        let settings_path = std::env::var_os("YINGQU_AUTHOR_TEST_SETTINGS").unwrap();
        let db =
            Connection::open_with_flags(settings_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let profile = settings::read_profile(&db, "douyin").unwrap();
        assert!(profile.cookie_configured);
        *app.inner
            .lock()
            .unwrap()
            .settings
            .profiles
            .iter_mut()
            .find(|profile| profile.platform == "douyin")
            .unwrap() = profile;
        app
    }

    fn replay_sample(app: &Core, item: &Post) -> Scan {
        let mut inner = app.inner.lock().unwrap();
        let mut scan = Scan::new(&Uuid::new_v4().to_string(), "completed");
        let mut writer = ScanWriter::default();
        writer
            .page(
                &mut scan,
                json!({
                    "author_id":item.author_id, "page":1, "skipped":0, "has_more":false,
                    "next_cursor":null, "items":[item],
                }),
            )
            .unwrap();
        let session = ScanDraft {
            scan,
            writer,
            scope: Some(current_scope(&inner.db).unwrap()),
        };
        let scan = scan_progress(&inner, &session);
        inner.author_draft = Some(session);
        scan
    }

    #[tokio::test]
    #[ignore = "requires explicitly authorized existing Douyin settings, author link, and two sample IDs"]
    async fn author_live_incremental_sample_and_retry() {
        let root = tempfile::tempdir().unwrap();
        let app = open_author_live(root.path());
        let link = std::env::var("YINGQU_AUTHOR_TEST_LINK").unwrap();
        let video = std::env::var("YINGQU_AUTHOR_TEST_VIDEO").unwrap();
        let album = std::env::var("YINGQU_AUTHOR_TEST_ALBUM").unwrap();
        let scan = scan_with_deadline(&app, &link, 240).await;
        assert_eq!(scan.status, "completed", "{:?}", scan.error);
        assert!(app.list().is_empty());
        println!(
            "Scan: {} pages, {} videos, {} albums, 0 queued",
            scan.pages, scan.videos, scan.albums
        );

        let posts = app
            .inner
            .lock()
            .unwrap()
            .author_draft
            .take()
            .unwrap()
            .writer
            .posts;
        let blocker = root.path().join("blocked-download-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;
        let sample = replay_sample(
            &app,
            posts.iter().find(|post| post.video_id == video).unwrap(),
        );
        assert_eq!(sample.pending_downloads, 1);
        assert_eq!(
            app.confirm_author_scan(&sample.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            1
        );
        let failed_id = wait_for_failure(&app, &sample.id).await;

        app.inner.lock().unwrap().settings.download_directory = root.path().join("downloads");
        let sample = replay_sample(
            &app,
            posts.iter().find(|post| post.video_id == album).unwrap(),
        );
        assert_eq!(sample.pending_downloads, 1);
        assert_eq!(
            app.confirm_author_scan(&sample.id, false, DownloadMode::Incremental)
                .unwrap()
                .queued,
            1
        );
        assert_eq!(finished(&app, &sample.id, 120).await.completed, 1);
        assert_eq!(app.retry(&failed_id).unwrap().id, failed_id);
        tokio::time::timeout(Duration::from_secs(120), async {
            while !app
                .list()
                .iter()
                .any(|task| task.id == failed_id && archived(task))
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        assert!(app
            .list()
            .iter()
            .any(|task| task.id == failed_id && archived(task)));
        println!("Controlled replay: 1 new album + 1 failed video, both committed; failed task ID reused");
        app.shutdown().await;
    }
}
