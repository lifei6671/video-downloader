use super::*;
use rusqlite::OptionalExtension;

pub(super) struct ActiveScan {
    id: String,
    pub(super) cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
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
    mode: ScanMode,
    needs_baseline: bool,
    new_posts: usize,
    retry_posts: usize,
    already_local: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    #[default]
    Full,
    Incremental,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineAction {
    ArchiveHistory,
    BaselineOnly,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Batch {
    queued: usize,
    skipped: usize,
    #[serde(default)]
    baseline_only: bool,
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
        "CREATE TABLE IF NOT EXISTS author_scans(
            id TEXT PRIMARY KEY, author_id TEXT, status TEXT NOT NULL, pages INTEGER NOT NULL DEFAULT 0,
            videos INTEGER NOT NULL DEFAULT 0, albums INTEGER NOT NULL DEFAULT 0,
            skipped INTEGER NOT NULL DEFAULT 0, error TEXT, batch TEXT, pause_reason TEXT);
         CREATE TABLE IF NOT EXISTS author_scan_items(
            scan_id TEXT NOT NULL REFERENCES author_scans(id) ON DELETE CASCADE,
            video_id TEXT NOT NULL, ordinal INTEGER NOT NULL, post TEXT NOT NULL,
            PRIMARY KEY(scan_id,video_id));
         CREATE INDEX IF NOT EXISTS author_scan_items_order ON author_scan_items(scan_id,ordinal);
         CREATE TABLE IF NOT EXISTS author_scan_options(
            scan_id TEXT PRIMARY KEY REFERENCES author_scans(id) ON DELETE CASCADE,
            incremental INTEGER NOT NULL CHECK(incremental IN (0,1)), scope TEXT);
         CREATE TABLE IF NOT EXISTS author_targets(
            scope TEXT NOT NULL, author_id TEXT NOT NULL, last_completed_scan_id TEXT NOT NULL,
            PRIMARY KEY(scope,author_id));
         CREATE TABLE IF NOT EXISTS author_posts(
            scope TEXT NOT NULL, author_id TEXT NOT NULL, video_id TEXT NOT NULL, post TEXT NOT NULL,
            wanted INTEGER NOT NULL CHECK(wanted IN (0,1)),
            PRIMARY KEY(scope,author_id,video_id));
         CREATE TABLE IF NOT EXISTS author_batch_items(
            scan_id TEXT NOT NULL REFERENCES author_scans(id) ON DELETE CASCADE,
            task_id TEXT NOT NULL, PRIMARY KEY(scan_id,task_id));
         UPDATE author_scans SET status='partial', error='INTERRUPTED' WHERE status='scanning';",
    ).context("DATABASE_ERROR")
}

fn read_scan(db: &Connection, id: &str) -> Result<Scan> {
    let (mut scan, batch) = db.query_row(
        "SELECT author_id,status,pages,videos,albums,skipped,error,batch,pause_reason,
            (SELECT json_extract(post,'$.author') FROM author_scan_items WHERE scan_id=author_scans.id ORDER BY ordinal LIMIT 1),
            COALESCE((SELECT incremental FROM author_scan_options WHERE scan_id=author_scans.id),0)
         FROM author_scans WHERE id=?",
        [id],
        |row| Ok((Scan {
            id: id.to_owned(), author_id: row.get(0)?, status: row.get(1)?, pages: row.get(2)?,
            videos: row.get(3)?, albums: row.get(4)?, skipped: row.get(5)?, error: row.get(6)?,
            batch: None, pause_reason: row.get(8)?, author_name: row.get(9)?,
            remaining: 0, completed: 0, failed: 0,
            mode: if row.get::<_, bool>(10)? { ScanMode::Incremental } else { ScanMode::Full },
            needs_baseline: false, new_posts: 0, retry_posts: 0, already_local: 0,
        }, row.get::<_, Option<String>>(7)?)),
    ).optional().context("DATABASE_ERROR")?.context("AUTHOR_SCAN_NOT_FOUND")?;
    scan.batch = batch
        .map(|batch| serde_json::from_str(&batch))
        .transpose()?;
    Ok(scan)
}

fn read_current_scan(inner: &Inner, id: &str) -> Result<Scan> {
    let mut scan = read_scan(&inner.db, id)?;
    if scan.status == "scanning"
        && !inner
            .author_scan
            .as_ref()
            .is_some_and(|active| active.id == id)
    {
        scan.status = "partial".into();
        scan.error = Some("DATABASE_ERROR".into());
    }
    Ok(scan)
}

fn read_with_tasks(inner: &Inner, id: &str) -> Result<Scan> {
    let mut scan = read_current_scan(inner, id)?;
    if scan.batch.is_some() {
        let mut query = inner
            .db
            .prepare("SELECT task_id FROM author_batch_items WHERE scan_id=?")?;
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
                Status::Completed => scan.completed += 1,
                Status::Failed | Status::Canceled => scan.failed += 1,
                _ => {}
            }
        }
    } else if scan.mode == ScanMode::Incremental
        && scan.status != "scanning"
        && scan.author_id.is_some()
    {
        let scope = scan_scope(&inner.db, id)?;
        let author = scan.author_id.as_deref().unwrap();
        scan.needs_baseline = completed_baseline(&inner.db, scope.as_deref(), author)?.is_none();
        if !scan.needs_baseline {
            let tasks = indexed_tasks(&inner.tasks);
            for candidate in candidates(&inner.db, &scan, scope.as_deref(), author, false)? {
                let matches = tasks
                    .get(candidate.post.video_id.as_str())
                    .map_or(&[][..], Vec::as_slice);
                if matches.iter().any(|task| archived(task)) {
                    scan.already_local += 1;
                } else if !matches.iter().any(|task| download_in_progress(task)) {
                    if candidate.known {
                        scan.retry_posts += 1;
                    } else {
                        scan.new_posts += 1;
                    }
                }
            }
        }
    }
    Ok(scan)
}

fn current_scope(db: &Connection) -> Result<String> {
    db.query_row(
        "SELECT scope FROM douyin_author_scope WHERE id=1",
        [],
        |row| row.get(0),
    )
    .context("DATABASE_ERROR")
}

fn scan_scope(db: &Connection, id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT scope FROM author_scan_options WHERE scan_id=?",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

fn completed_baseline(
    db: &Connection,
    scope: Option<&str>,
    author: &str,
) -> Result<Option<String>> {
    let Some(scope) = scope else { return Ok(None) };
    Ok(db
        .query_row(
            "SELECT last_completed_scan_id FROM author_targets WHERE scope=? AND author_id=?",
            params![scope, author],
            |row| row.get(0),
        )
        .optional()?)
}

struct Candidate {
    post: Post,
    known: bool,
}

fn candidates(
    db: &Connection,
    scan: &Scan,
    scope: Option<&str>,
    author: &str,
    all: bool,
) -> Result<Vec<Candidate>> {
    let mut query = db.prepare(
        "SELECT item.post, remembered.wanted FROM author_scan_items AS item
         LEFT JOIN author_posts AS remembered
           ON remembered.scope=?2 AND remembered.author_id=?3 AND remembered.video_id=item.video_id
         WHERE item.scan_id=?1 ORDER BY item.ordinal",
    )?;
    let mut result = Vec::new();
    for row in query.query_map(params![scan.id, scope, author], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<bool>>(1)?))
    })? {
        let (post, wanted) = row?;
        if all || wanted != Some(false) {
            result.push(Candidate {
                post: serde_json::from_str(&post)?,
                known: wanted.is_some(),
            });
        }
    }
    if scan.mode == ScanMode::Incremental && scan.status == "completed" {
        // Previously confirmed work may be absent from the current feed.
        let mut missing = db.prepare(
            "SELECT post FROM author_posts AS remembered
             WHERE scope=?2 AND author_id=?3 AND wanted=1 AND NOT EXISTS (
                SELECT 1 FROM author_scan_items WHERE scan_id=?1 AND video_id=remembered.video_id
             ) ORDER BY video_id DESC",
        )?;
        for row in missing.query_map(params![scan.id, scope, author], |row| {
            row.get::<_, String>(0)
        })? {
            result.push(Candidate {
                post: serde_json::from_str(&row?)?,
                known: true,
            });
        }
    }
    Ok(result)
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
    ended: bool,
}

fn valid_author(id: &str) -> bool {
    (10..=256).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl ScanWriter {
    fn page(&mut self, db: &mut Connection, id: &str, value: Value) -> Result<()> {
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
        match (page.has_more, page.next_cursor) {
            (true, Some(cursor))
                if !cursor.is_empty()
                    && cursor != "0"
                    && cursor.len() <= 32
                    && cursor.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                if !self.cursors.insert(cursor) {
                    bail!("AUTHOR_CURSOR_INVALID");
                }
            }
            (false, None) => {}
            _ => bail!("AUTHOR_CURSOR_INVALID"),
        }
        let tx = db.transaction().context("DATABASE_ERROR")?;
        let mut videos = 0;
        let mut albums = 0;
        {
            let mut insert = tx.prepare(
                "INSERT INTO author_scan_items(scan_id,video_id,ordinal,post) VALUES(?,?,?,?)
                 ON CONFLICT(scan_id,video_id) DO NOTHING",
            )?;
            for (ordinal, post) in page.items.into_iter().enumerate() {
                if post.author_id != page.author_id {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                post.identity()?;
                let added = insert.execute(params![
                    id,
                    post.video_id,
                    page.page as i64 * 200 + ordinal as i64,
                    serde_json::to_string(&post)?
                ])?;
                if post.kind == "video" {
                    videos += added;
                } else {
                    albums += added;
                }
            }
        }
        tx.execute(
            "UPDATE author_scans SET author_id=?,pages=?,videos=videos+?,albums=albums+?,skipped=skipped+? WHERE id=?",
            params![page.author_id, page.page as i64, videos as i64, albums as i64, page.skipped as i64, id],
        )?;
        tx.commit().context("DATABASE_ERROR")?;
        self.author = Some(page.author_id);
        self.pages = page.page;
        self.ended = !page.has_more;
        Ok(())
    }
}

pub(super) fn batch_id(task: &Task) -> Option<&str> {
    task.video.as_ref()?["author_scan_id"].as_str()
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

pub(super) fn paused(db: &Connection) -> Result<HashSet<String>> {
    Ok(db
        .prepare("SELECT id FROM author_scans WHERE pause_reason IS NOT NULL")?
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
    pub fn start_author_scan(self: &Arc<Self>, input: String, mode: ScanMode) -> Result<Scan> {
        if input.trim().is_empty() || input.len() > 4096 {
            bail!("INVALID_AUTHOR_LINK");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.closing
            || inner.author_scan.is_some()
            || inner
                .tasks
                .iter()
                .any(|task| batch_id(task).is_some() && download_in_progress(task))
        {
            bail!("TASK_BUSY");
        }
        let profiles = settings::douyin_snapshot(&inner.settings)?;
        let has_cookie = profiles["douyin"]["cookie"]
            .as_str()
            .is_some_and(|cookie| !cookie.is_empty());
        if mode == ScanMode::Incremental && !has_cookie {
            bail!("AUTHOR_BASELINE_COOKIE_REQUIRED");
        }
        let scope = has_cookie.then(|| current_scope(&inner.db)).transpose()?;
        let id = Uuid::new_v4().to_string();
        let tx = inner.db.transaction()?;
        tx.execute(
            "INSERT INTO author_scans(id,status) VALUES(?,'scanning')",
            [&id],
        )?;
        tx.execute(
            "INSERT INTO author_scan_options(scan_id,incremental,scope) VALUES(?,?,?)",
            params![id, mode == ScanMode::Incremental, scope],
        )?;
        tx.commit()?;
        let initial = read_scan(&inner.db, &id)?;
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, completion) = watch::channel(false);
        inner.author_scan = Some(ActiveScan {
            id: id.clone(),
            cancel: cancel.clone(),
            done: completion,
        });
        let core = self.clone();
        inner.workers.retain(|worker| !worker.is_finished());
        inner.workers.push(tokio::spawn(async move {
            let mut writer = ScanWriter::default();
            let mut page_error = None;
            let result = PythonBridge::operation_cancellable(
                &core.program,
                &core.args,
                &id,
                "scan_author_posts",
                json!({"input": input, "profile": profiles["douyin"]}),
                |value| {
                    if page_error.is_none() {
                        if let Err(error) =
                            writer.page(&mut core.inner.lock().unwrap().db, &id, value)
                        {
                            page_error = Some(error);
                            cancel.send_replace(true);
                        }
                    }
                },
                async {
                    let _ = cancellation.wait_for(|value| *value).await;
                },
            )
            .await;
            let mut inner = core.inner.lock().unwrap();
            let (status, error) = match (result, page_error) {
                (_, Some(error)) => ("partial", Some(error.to_string())),
                (Ok(result), None)
                    if writer.ended
                        && result["result"]["pages"].as_u64() == Some(writer.pages)
                        && !*cancel.borrow() =>
                {
                    ("completed", None)
                }
                (Err(error), None) => ("partial", Some(error.to_string())),
                _ if *cancel.borrow() => ("partial", Some("CANCELED".into())),
                _ => ("partial", Some("ENGINE_PROTOCOL_ERROR".into())),
            };
            if inner
                .db
                .execute(
                    "UPDATE author_scans SET status=?,error=? WHERE id=?",
                    params![status, error, id],
                )
                .is_err()
            {
                eprintln!("author_scan event=state_save_failed");
            }
            inner.author_scan = None;
            core.dispatch(&mut inner);
            done.send_replace(true);
        }));
        Ok(initial)
    }

    pub fn author_scan(&self, id: &str) -> Result<Scan> {
        read_with_tasks(&self.inner.lock().unwrap(), id)
    }

    pub fn latest_author_scan(&self) -> Result<Option<Scan>> {
        let inner = self.inner.lock().unwrap();
        if let Some(active) = &inner.author_scan {
            return read_with_tasks(&inner, &active.id).map(Some);
        }
        if let Some(id) = inner
            .tasks
            .iter()
            .filter(|task| task.status == Status::Queued)
            .filter_map(batch_id)
            .find(|id| inner.author_paused.contains(*id))
        {
            return read_with_tasks(&inner, id).map(Some);
        }
        let id = inner
            .db
            .query_row(
                "SELECT id FROM author_scans ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        id.map(|id| read_with_tasks(&inner, &id)).transpose()
    }

    pub async fn cancel_author_scan(&self, id: &str) -> Result<Scan> {
        let completion = {
            let inner = self.inner.lock().unwrap();
            inner
                .author_scan
                .as_ref()
                .filter(|active| active.id == id)
                .map(|active| {
                    active.cancel.send_replace(true);
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
        baseline_action: Option<BaselineAction>,
    ) -> Result<Batch> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("TASK_BUSY");
        }
        let scan = read_current_scan(&inner, id)?;
        if let Some(batch) = scan.batch {
            return Ok(batch);
        }
        if scan.status != "completed" && !(allow_partial && scan.status == "partial") {
            bail!("INVALID_STATE");
        }
        if inner.author_scan.is_some()
            || inner
                .tasks
                .iter()
                .any(|task| batch_id(task).is_some() && download_in_progress(task))
        {
            bail!("TASK_BUSY");
        }
        let author = scan.author_id.as_deref().context("INVALID_STATE")?;
        let scope = scan_scope(&inner.db, id)?;
        if let Some(scope) = scope.as_deref() {
            if current_scope(&inner.db)? != scope {
                bail!("AUTHOR_SCOPE_CHANGED");
            }
        }
        let latest = completed_baseline(&inner.db, scope.as_deref(), author)?;
        if let Some(latest) = &latest {
            let stale = inner.db.query_row(
                "SELECT (SELECT rowid FROM author_scans WHERE id=?1) > (SELECT rowid FROM author_scans WHERE id=?2)",
                params![latest, id], |row| row.get::<_, bool>(0),
            )?;
            if stale {
                bail!("AUTHOR_SCAN_STALE");
            }
        }
        let first = scan.mode == ScanMode::Incremental && latest.is_none();
        if first && baseline_action.is_none() {
            bail!("AUTHOR_BASELINE_ACTION_REQUIRED");
        }
        if !first && baseline_action.is_some() {
            bail!("INVALID_STATE");
        }
        let baseline_only = baseline_action == Some(BaselineAction::BaselineOnly);
        if baseline_only && scan.status != "completed" {
            bail!("AUTHOR_BASELINE_INCOMPLETE");
        }
        let profiles = Arc::new(settings::douyin_snapshot(&inner.settings)?);
        let posts = candidates(
            &inner.db,
            &scan,
            scope.as_deref(),
            author,
            scan.mode == ScanMode::Full || first,
        )?;
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
        for candidate in posts.iter().filter(|_| !baseline_only) {
            let post = &candidate.post;
            let matches = existing
                .get(post.video_id.as_str())
                .map_or(&[][..], Vec::as_slice);
            if matches.iter().any(|task| archived(task)) {
                skipped += 1;
                continue;
            }
            if let Some(task) = matches.iter().find(|task| download_in_progress(task)) {
                linked.insert(task.id.clone());
                skipped += 1;
                continue;
            }
            let previous = (scan.mode == ScanMode::Incremental && candidate.known)
                .then(|| {
                    matches.iter().copied().find(|task| {
                        matches!(task.status, Status::Failed | Status::Canceled)
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
                })
                .flatten();
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
            queued.push(Task {
                id: task_id,
                insertion_id: previous.map_or(0, |task| task.insertion_id),
                asset_id: None,
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
                download_requested_at: Some(chrono::Utc::now().timestamp_millis().max(0) as u64),
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
            });
        }
        let batch = Batch {
            queued: queued.len(),
            skipped,
            baseline_only,
        };
        let tx = inner.db.transaction().context("DATABASE_ERROR")?;
        if let Some(scope) = scope.as_deref() {
            let mut remember = tx.prepare(
                "INSERT INTO author_posts(scope,author_id,video_id,post,wanted) VALUES(?,?,?,?,?)
                 ON CONFLICT(scope,author_id,video_id) DO UPDATE
                 SET post=excluded.post,wanted=MAX(author_posts.wanted,excluded.wanted)",
            )?;
            for candidate in &posts {
                let post = &candidate.post;
                remember.execute(params![
                    scope,
                    author,
                    post.video_id,
                    serde_json::to_string(post)?,
                    !baseline_only
                ])?;
            }
            if scan.status == "completed" {
                tx.execute(
                    "INSERT INTO author_targets(scope,author_id,last_completed_scan_id) VALUES(?,?,?)
                     ON CONFLICT(scope,author_id) DO UPDATE SET last_completed_scan_id=excluded.last_completed_scan_id",
                    params![scope, author, id],
                )?;
            }
        }
        for task in &mut queued {
            persist_published(&tx, task)?;
        }
        for task in linked {
            tx.execute(
                "INSERT INTO author_batch_items(scan_id,task_id) VALUES(?,?)",
                params![id, task],
            )?;
        }
        tx.execute(
            "UPDATE author_scans SET batch=?,status=?,error=? WHERE id=?",
            params![serde_json::to_string(&batch)?, scan.status, scan.error, id],
        )?;
        tx.commit().context("DATABASE_ERROR")?;
        let directory = inner.settings.download_directory.clone();
        let template = inner.settings.filename_template.clone();
        let mut published = Vec::with_capacity(queued.len());
        for task in queued {
            inner.active.insert(
                task.id.clone(),
                Active {
                    telegram: false,
                    download_after_resolve: false,
                    profiles: profiles.clone(),
                    directory: directory.clone(),
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
            (self.emit)(TaskEvent::Updated(Box::new(task)));
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
                "UPDATE author_scans SET pause_reason=? WHERE id=?",
                params![code, id],
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
            return read_with_tasks(&inner, id);
        }
        let profiles = Arc::new(settings::douyin_snapshot(&inner.settings)?);
        let pending = inner
            .tasks
            .iter()
            .filter(|task| batch_id(task) == Some(id) && task.status == Status::Queued)
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        inner
            .db
            .execute("UPDATE author_scans SET pause_reason=NULL WHERE id=?", [id])?;
        inner.author_paused.remove(id);
        for task in pending {
            if let Some(active) = inner.active.get_mut(&task) {
                active.profiles = profiles.clone();
            }
        }
        self.dispatch(&mut inner);
        read_with_tasks(&inner, id)
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
        if let Err(error) = self.queue_download(&mut inner, task, true, None) {
            inner.active.remove(id);
            return Err(error);
        }
        self.dispatch(&mut inner);
        Ok(inner.tasks.iter().find(|task| task.id == id).cloned())
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
        scan_mode_ready(core, input, ScanMode::Full, 10).await
    }

    async fn scan_mode_ready(core: &Arc<Core>, input: &str, mode: ScanMode, seconds: u64) -> Scan {
        let initial = core.start_author_scan(input.into(), mode).unwrap();
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                let scan = core.author_scan(&initial.id).unwrap();
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
    async fn author_incremental_baseline_new_work_and_missing_failed_work() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        set_cookie(&app, "sessionid=fixture");
        let mode = ScanMode::Incremental;
        let initial = scan_mode_ready(&app, "full", mode, 10).await;
        assert!(initial.needs_baseline);
        assert_eq!(
            app.confirm_author_scan(&initial.id, false, None)
                .unwrap_err()
                .to_string(),
            "AUTHOR_BASELINE_ACTION_REQUIRED"
        );
        assert!(
            app.confirm_author_scan(&initial.id, false, Some(BaselineAction::BaselineOnly))
                .unwrap()
                .baseline_only
        );
        assert!(app.list().is_empty());

        let unchanged = scan_mode_ready(&app, "full", mode, 10).await;
        assert!(!unchanged.needs_baseline);
        assert_eq!((unchanged.new_posts, unchanged.retry_posts), (0, 0));

        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;
        let next = scan_mode_ready(&app, "new-video", mode, 10).await;
        assert_eq!((next.new_posts, next.retry_posts), (1, 0));
        assert_eq!(
            app.confirm_author_scan(&next.id, false, None)
                .unwrap()
                .queued,
            1
        );
        let failed_id = wait_for_failure(&app, &next.id).await;
        assert_eq!(
            app.confirm_author_scan(&unchanged.id, false, None)
                .unwrap_err()
                .to_string(),
            "AUTHOR_SCAN_STALE"
        );
        app.shutdown().await;
        drop(app);

        // The failed work is absent from the next feed. Its confirmed intent survives restart.
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let next = scan_mode_ready(&app, "new-album", mode, 10).await;
        assert_eq!((next.new_posts, next.retry_posts), (1, 1));
        assert_eq!(
            app.confirm_author_scan(&next.id, false, None)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(
            app.confirm_author_scan(&next.id, false, None)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, &next.id, 15).await.completed, 2);
        let tasks = app.list();
        assert_eq!(tasks.len(), 2);
        assert!(tasks
            .iter()
            .any(|task| task.id == failed_id && archived(task)));
        assert_eq!(app.author_scan(&next.id).unwrap().completed, 2);

        // Full archive can still promote the historical baseline-only posts.
        let full = scan_ready(&app, "full").await;
        assert_eq!(
            app.confirm_author_scan(&full.id, false, None)
                .unwrap()
                .queued,
            2
        );
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_incremental_initial_archive_and_scope_change() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let mode = ScanMode::Incremental;
        assert_eq!(
            app.start_author_scan("full".into(), mode)
                .unwrap_err()
                .to_string(),
            "AUTHOR_BASELINE_COOKIE_REQUIRED"
        );
        set_cookie(&app, "sessionid=fixture");
        let partial = scan_mode_ready(&app, "partial", mode, 10).await;
        assert!(partial.needs_baseline);
        assert_eq!(
            app.confirm_author_scan(&partial.id, true, Some(BaselineAction::BaselineOnly))
                .unwrap_err()
                .to_string(),
            "AUTHOR_BASELINE_INCOMPLETE"
        );
        let complete = scan_mode_ready(&app, "full", mode, 10).await;
        set_cookie(&app, "sessionid=fixture");
        assert_eq!(
            app.confirm_author_scan(&complete.id, false, Some(BaselineAction::ArchiveHistory))
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, &complete.id, 15).await.completed, 2);
        let current = scan_mode_ready(&app, "full", mode, 10).await;
        assert!(!current.needs_baseline);
        assert_eq!(current.already_local, 2);
        set_cookie(&app, "sessionid=different");
        assert_eq!(
            app.confirm_author_scan(&current.id, false, None)
                .unwrap_err()
                .to_string(),
            "AUTHOR_SCOPE_CHANGED"
        );
        let fresh = scan_mode_ready(&app, "full", mode, 10).await;
        assert!(fresh.needs_baseline);
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
            app.confirm_author_scan(&scan.id, false, None)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(
            app.confirm_author_scan(&scan.id, false, None)
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
        let result = app.confirm_author_scan(&repeat.id, false, None).unwrap();
        assert_eq!((result.queued, result.skipped), (0, 2));
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_partial_scan_requires_explicit_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let app = open(root.path(), "tests/fixtures/desktop_engine.py");
        let scan = scan_ready(&app, "partial").await;
        assert_eq!((scan.status.as_str(), scan.pages), ("partial", 1));
        assert_eq!(scan.error.as_deref(), Some("ACCESS_DENIED"));
        assert!(app.confirm_author_scan(&scan.id, false, None).is_err());
        assert!(app.list().is_empty());
        assert_eq!(
            app.confirm_author_scan(&scan.id, true, None)
                .unwrap()
                .queued,
            2
        );
        app.shutdown().await;
    }

    #[test]
    fn author_writer_keeps_valid_pages_and_rejects_repeated_cursors() {
        let mut db = Connection::open_in_memory().unwrap();
        init(&db).unwrap();
        db.execute(
            "INSERT INTO author_scans(id,status) VALUES('test','scanning')",
            [],
        )
        .unwrap();
        let mut writer = ScanWriter::default();
        for page in 1..=41 {
            writer
                .page(
                    &mut db,
                    "test",
                    json!({"author_id":"fixture-author", "page":page,
                "items":[], "skipped":0, "has_more":true, "next_cursor":page.to_string()}),
                )
                .unwrap();
        }
        assert!(writer
            .page(
                &mut db,
                "test",
                json!({"author_id":"fixture-author","page":42,
            "items":[],"skipped":0,"has_more":true,"next_cursor":"41"})
            )
            .is_err());
        assert_eq!(read_scan(&db, "test").unwrap().pages, 41);
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
            inner
                .db
                .execute(
                    "INSERT INTO author_scans(id,status) VALUES('sample','scanning')",
                    [],
                )
                .unwrap();
            ScanWriter::default().page(&mut inner.db, "sample", json!({
                "author_id":author,"page":1,"skipped":0,"has_more":false,"next_cursor":null,
                "items":[
                    {"video_id":video,"kind":"video","title":"Sample video","author":"Sample","author_id":author},
                    {"video_id":album,"kind":"image_album","title":"Sample album","author":"Sample","author_id":author}
                ]
            })).unwrap();
            inner
                .db
                .execute(
                    "UPDATE author_scans SET status='completed' WHERE id='sample'",
                    [],
                )
                .unwrap();
        }
        assert_eq!(
            app.confirm_author_scan("sample", false, None)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, "sample", 120).await.completed, 2);
        for task in app.list() {
            assert!(archived(&task));
            println!(
                "{}: {} bytes, {} images",
                task.video.unwrap()["kind"],
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

    fn replay_sample(app: &Core, source: &Scan, video_id: &str) -> Scan {
        let mut inner = app.inner.lock().unwrap();
        let id = Uuid::new_v4().to_string();
        let scope = scan_scope(&inner.db, &source.id).unwrap().unwrap();
        let author = source.author_id.as_deref().unwrap();
        let item: String = inner
            .db
            .query_row(
                "SELECT post FROM author_scan_items WHERE scan_id=? AND video_id=?",
                params![source.id, video_id],
                |row| row.get(0),
            )
            .unwrap();
        // In the isolated DB, model a previously unseen item using genuine scan metadata.
        assert_eq!(
            inner
                .db
                .execute(
                    "DELETE FROM author_posts WHERE scope=? AND author_id=? AND video_id=?",
                    params![scope, author, video_id]
                )
                .unwrap(),
            1
        );
        inner
            .db
            .execute(
                "INSERT INTO author_scans(id,status) VALUES(?,'scanning')",
                [&id],
            )
            .unwrap();
        inner
            .db
            .execute(
                "INSERT INTO author_scan_options VALUES(?,1,?)",
                params![id, scope],
            )
            .unwrap();
        ScanWriter::default()
            .page(
                &mut inner.db,
                &id,
                json!({
                    "author_id":author, "page":1, "skipped":0, "has_more":false, "next_cursor":null,
                    "items":[serde_json::from_str::<Value>(&item).unwrap()]
                }),
            )
            .unwrap();
        inner
            .db
            .execute(
                "UPDATE author_scans SET status='completed' WHERE id=?",
                [&id],
            )
            .unwrap();
        read_with_tasks(&inner, &id).unwrap()
    }

    #[tokio::test]
    #[ignore = "requires explicitly authorized existing Douyin settings, author link, and two sample IDs"]
    async fn author_live_incremental_baseline_and_retry() {
        let root = tempfile::tempdir().unwrap();
        let app = open_author_live(root.path());
        let link = std::env::var("YINGQU_AUTHOR_TEST_LINK").unwrap();
        let video = std::env::var("YINGQU_AUTHOR_TEST_VIDEO").unwrap();
        let album = std::env::var("YINGQU_AUTHOR_TEST_ALBUM").unwrap();
        let mode = ScanMode::Incremental;
        let first = scan_mode_ready(&app, &link, mode, 240).await;
        assert_eq!(first.status, "completed", "{:?}", first.error);
        assert!(first.needs_baseline);
        assert!(
            app.confirm_author_scan(&first.id, false, Some(BaselineAction::BaselineOnly))
                .unwrap()
                .baseline_only
        );
        assert!(app.list().is_empty());
        println!(
            "Baseline: {} pages, {} videos, {} albums, 0 queued",
            first.pages, first.videos, first.albums
        );

        let second = scan_mode_ready(&app, &link, mode, 240).await;
        assert_eq!(second.status, "completed", "{:?}", second.error);
        assert!(!second.needs_baseline);
        assert_eq!(second.retry_posts, 0);
        println!(
            "Rescan: {} pages, {} videos, {} albums, {} new candidates",
            second.pages, second.videos, second.albums, second.new_posts
        );

        let blocker = root.path().join("blocked-download-directory");
        std::fs::write(&blocker, b"fixture").unwrap();
        app.inner.lock().unwrap().settings.download_directory = blocker;
        let sample = replay_sample(&app, &second, &video);
        assert_eq!(sample.new_posts, 1);
        assert_eq!(
            app.confirm_author_scan(&sample.id, false, None)
                .unwrap()
                .queued,
            1
        );
        let failed_id = wait_for_failure(&app, &sample.id).await;

        app.inner.lock().unwrap().settings.download_directory = root.path().join("downloads");
        let sample = replay_sample(&app, &second, &album);
        assert_eq!((sample.new_posts, sample.retry_posts), (1, 1));
        assert_eq!(
            app.confirm_author_scan(&sample.id, false, None)
                .unwrap()
                .queued,
            2
        );
        assert_eq!(finished(&app, &sample.id, 120).await.completed, 2);
        assert!(app
            .list()
            .iter()
            .any(|task| task.id == failed_id && archived(task)));
        println!("Controlled replay: 1 new album + 1 failed video, both committed; failed task ID reused");
        app.shutdown().await;
    }
}
