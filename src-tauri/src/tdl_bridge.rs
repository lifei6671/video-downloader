//! Experimental adapter for the unmodified tdl v0.20.4 HTTP serve protocol.
//! Its server binds all interfaces and is neither authenticated nor message-scoped.
use anyhow::{bail, Context, Result};
use reqwest::{header, Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    sync::{watch, Mutex, MutexGuard},
    time::{sleep, timeout},
};

pub(crate) const ALIAS_IDENTITY_MISMATCH: &str = "TDL_ALIAS_IDENTITY_MISMATCH";
pub const PROTOCOL_REVISION: &str = "tdl-0.20.4-http-v1";
static SESSION: Mutex<()> = Mutex::const_new(());
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TdlSettings {
    pub enabled: bool,
    pub path: Option<PathBuf>,
    // App-owned authentication storage. IPC callers cannot select or replace it.
    #[serde(default, skip_deserializing)]
    pub session_directory: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct TdlStatus {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Debug)]
pub(crate) struct MessageLink {
    pub channel: Option<u64>,
    pub message: u32,
    pub url: String,
}

fn positive(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) || text.starts_with('0') {
        return None;
    }
    text.parse::<u64>()
        .ok()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
}

pub fn is_telegram(input: &str) -> bool {
    crate::parse_cache::share_url(input)
        .and_then(|s| url::Url::parse(&s).ok())
        .is_some_and(|u| matches!(u.host_str(), Some("t.me" | "telegram.me")))
}

pub(crate) fn message_link(input: &str) -> Result<MessageLink> {
    // Extract just one link; keep the original query for validation, unlike cache lookup normalization.
    let start = input.find("https://").context("INVALID_INPUT")?;
    let end = input[start..]
        .find(char::is_whitespace)
        .map_or(input.len(), |n| start + n);
    let text = input[start..end].trim_end_matches(['。', '，', ')', '）']);
    if input[end..].contains("https://") || input[..start].contains("http://") {
        bail!("INVALID_INPUT");
    }
    let u = url::Url::parse(text).context("INVALID_INPUT")?;
    if u.scheme() != "https"
        || !matches!(u.host_str(), Some("t.me" | "telegram.me"))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || text.contains('%')
        || text.contains('\\')
    {
        bail!("INVALID_INPUT");
    }
    let parts: Vec<_> = u.path().trim_start_matches('/').split('/').collect();
    let (channel, msg) = match parts.as_slice() {
        ["c", channel, msg] => (Some(positive(channel).context("INVALID_INPUT")?), *msg),
        [name, msg]
            if (1..=32).contains(&name.len())
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !matches!(*name, "c" | "s" | "joinchat") =>
        {
            (None, *msg)
        }
        _ => bail!("INVALID_INPUT"),
    };
    let message =
        u32::try_from(positive(msg).context("INVALID_INPUT")?).context("INVALID_INPUT")?;
    if message > i32::MAX as u32 {
        bail!("INVALID_INPUT");
    }
    Ok(MessageLink {
        channel,
        message,
        url: format!("https://t.me{}", u.path()),
    })
}

pub fn validate_identity(id: &str, canonical: &str) -> bool {
    let Ok(link) = message_link(canonical) else {
        return false;
    };
    link.channel
        .is_some_and(|channel| id == format!("{channel}_{}", link.message))
}

fn command(path: &Path) -> Command {
    #[cfg(not(test))]
    let mut command = Command::new(path);
    #[cfg(test)]
    let mut command = if path.file_name().is_some_and(|n| n == "tdl_mock.py") {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut cmd = Command::new(root.join(if cfg!(windows) {
            ".venv/Scripts/python.exe"
        } else {
            ".venv/bin/python"
        }));
        cmd.arg(path);
        cmd
    } else {
        Command::new(path)
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    #[cfg(unix)]
    command.process_group(0);
    command
}

struct Process {
    child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    group: i32,
}

impl Process {
    fn spawn(mut command: Command) -> Result<Self> {
        let child = command.spawn().context("TDL_UNAVAILABLE")?;
        #[cfg(windows)]
        let job = crate::python_bridge::process_job(&child).context("TDL_UNAVAILABLE")?;
        #[cfg(unix)]
        let group = child.id().context("TDL_UNAVAILABLE")? as i32;
        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
            #[cfg(unix)]
            group,
        })
    }
    async fn stop(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            // A login terminal launcher owns a tdl descendant. Reap the whole
            // job before removing its authentication storage.
            if unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    self._job.as_raw_handle(),
                    1,
                )
            } == 0
            {
                bail!("TDL_STOP_FAILED");
            }
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        self.child.start_kill().context("TDL_STOP_FAILED")?;
        // Serve is long-lived: exit status is deliberately NOT a download result.
        timeout(Duration::from_secs(5), self.child.wait())
            .await
            .context("TDL_STOP_FAILED")?
            .context("TDL_STOP_FAILED")?;
        #[cfg(windows)]
        timeout(Duration::from_secs(5), async {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::*;
            loop {
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                    unsafe { std::mem::zeroed() };
                if unsafe {
                    QueryInformationJobObject(
                        self._job.as_raw_handle(),
                        JobObjectBasicAccountingInformation,
                        &mut info as *mut _ as *mut _,
                        std::mem::size_of_val(&info) as u32,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    bail!("TDL_STOP_FAILED");
                }
                if info.ActiveProcesses == 0 {
                    return Ok::<_, anyhow::Error>(());
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("TDL_STOP_FAILED")??;
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        // Windows job is kill-on-close, also when the operation future is dropped.
    }
}

async fn output(path: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = command(path);
    cmd.args(args);
    capture(cmd).await
}

async fn capture(mut cmd: Command) -> Result<String> {
    cmd.stdout(Stdio::piped());
    let mut process = Process::spawn(cmd)?;
    let mut bytes = Vec::new();
    let mut stdout = process
        .child
        .stdout
        .take()
        .context("TDL_UNAVAILABLE")?
        .take(65537);
    let result = timeout(Duration::from_secs(5), async {
        stdout
            .read_to_end(&mut bytes)
            .await
            .context("TDL_UNAVAILABLE")?;
        if bytes.len() > 65536 {
            bail!("TDL_UNSUPPORTED");
        }
        if !process
            .child
            .wait()
            .await
            .context("TDL_UNAVAILABLE")?
            .success()
        {
            bail!("TDL_UNSUPPORTED");
        }
        String::from_utf8(bytes).context("TDL_UNSUPPORTED")
    })
    .await
    .context("TDL_CHECK_TIMEOUT")
    .and_then(|r| r);
    if result.is_err() {
        process.stop().await?;
    }
    result
}

fn go_bin_roots(gobin: Option<&std::ffi::OsStr>, gopath: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = gobin.filter(|v| !v.is_empty()).map(PathBuf::from) {
        if path.is_absolute() {
            roots.push(path);
        }
    }
    if let Some(paths) = gopath {
        roots.extend(
            std::env::split_paths(paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("bin")),
        );
    }
    roots
}

async fn configured_go_bins(go: &Path) -> Vec<PathBuf> {
    let mut cmd = command(go);
    cmd.args(["env", "-json", "GOBIN", "GOPATH"])
        .env("GOTOOLCHAIN", "local");
    // Read go env -w configuration too, without switching/downloading toolchains.
    let Ok(text) = capture(cmd).await else {
        return Vec::new();
    };
    let Ok(values) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    go_bin_roots(
        values["GOBIN"].as_str().map(std::ffi::OsStr::new),
        values["GOPATH"].as_str().map(std::ffi::OsStr::new),
    )
}

async fn candidates(custom: Option<&Path>) -> Result<Vec<PathBuf>> {
    if let Some(path) = custom {
        if !path.is_absolute() || !path.is_file() {
            bail!("TDL_UNAVAILABLE");
        }
        return Ok(vec![path.to_owned()]);
    }
    let mut roots: Vec<_> = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    roots.extend(go_bin_roots(
        std::env::var_os("GOBIN").as_deref(),
        std::env::var_os("GOPATH").as_deref(),
    ));
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if let Some(home) = &home {
        roots.extend([
            home.join(".local/bin"),
            home.join("bin"),
            home.join("go/bin"),
        ]);
    }
    #[cfg(windows)]
    {
        // Match existing GUI tool discovery: refresh the PATH sources without changing the environment.
        for key in [
            r"HKCU\Environment",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ] {
            let mut cmd = command(Path::new("reg.exe"));
            cmd.args(["query", key]);
            // reg is not tdl and never opens a Telegram store.
            cmd.stdout(Stdio::piped());
            if let Ok(Ok(out)) = timeout(Duration::from_secs(3), cmd.output()).await {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines().filter(|line| line.contains("REG_")) {
                    let Some(name) = line.split_whitespace().next() else {
                        continue;
                    };
                    if !["path", "gobin", "gopath"].contains(&name.to_ascii_lowercase().as_str()) {
                        continue;
                    }
                    if let Some(value) = line
                        .split_once("REG_EXPAND_SZ")
                        .or_else(|| line.split_once("REG_SZ"))
                        .map(|(_, v)| v.trim())
                    {
                        let mut expanded = value.to_owned();
                        for (name, value) in std::env::vars() {
                            expanded = expanded.replace(&format!("%{name}%"), &value);
                        }
                        match name.to_ascii_lowercase().as_str() {
                            "gobin" => roots.extend(go_bin_roots(Some(expanded.as_ref()), None)),
                            "gopath" => roots.extend(go_bin_roots(None, Some(expanded.as_ref()))),
                            _ => roots.extend(std::env::split_paths(&expanded)),
                        }
                    }
                }
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(local).join("Microsoft/WinGet/Links"));
        }
        if let Some(home) = &home {
            roots.push(home.join("scoop/shims"));
        }
        roots.push(
            PathBuf::from(
                std::env::var_os("ChocolateyInstall")
                    .unwrap_or_else(|| r"C:\ProgramData\chocolatey".into()),
            )
            .join("bin"),
        );
        roots.push(PathBuf::from(r"C:\Go\bin"));
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            roots.push(PathBuf::from(programs).join("Go/bin"));
        }
    }
    #[cfg(unix)]
    {
        roots.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/opt/local/bin",
                "/usr/bin",
            ]
            .map(PathBuf::from),
        );
        let mut files = vec![PathBuf::from("/etc/paths")];
        if let Ok(entries) = std::fs::read_dir("/etc/paths.d") {
            files.extend(entries.flatten().map(|e| e.path()));
        }
        for file in files {
            if let Ok(text) = std::fs::read_to_string(file) {
                roots.extend(
                    text.lines()
                        .filter(|l| l.starts_with('/'))
                        .map(PathBuf::from),
                );
            }
        }
    }
    let go_name = if cfg!(windows) { "go.exe" } else { "go" };
    if let Some(go) = installed_executables(&roots, go_name).first() {
        roots.extend(configured_go_bins(go).await);
    }
    Ok(installed_executables(
        &roots,
        if cfg!(windows) { "tdl.exe" } else { "tdl" },
    ))
}

fn installed_executables(roots: &[PathBuf], name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        let path = root.join(name);
        if path.is_file() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

pub async fn check(custom: Option<&Path>) -> Result<TdlStatus> {
    let paths = candidates(custom).await?;
    if paths.is_empty() {
        bail!("TDL_UNAVAILABLE");
    }
    // version runs root hooks in tdl. Isolate their storage so dependency checks cannot migrate/login to the user's store.
    let scratch = tempfile::tempdir().context("IO_ERROR")?;
    let storage = format!("type=bolt,path={}", scratch.path().join("data").display());
    let mut error = anyhow::anyhow!("TDL_UNAVAILABLE");
    for path in paths {
        let result = async {
            let version = output(&path, &["--storage", &storage, "version"]).await?;
            let version = version
                .lines()
                .find_map(|l| l.trim().strip_prefix("Version: "))
                .context("TDL_UNSUPPORTED")?
                .trim();
            if version.is_empty() {
                bail!("TDL_UNSUPPORTED");
            }
            Ok(TdlStatus {
                path: path.clone(),
                version: version.to_owned(),
            })
        }
        .await;
        match result {
            Ok(status) => return Ok(status),
            Err(e) => error = e,
        }
    }
    Err(error)
}

async fn check_serve(custom: Option<&Path>) -> Result<TdlStatus> {
    let status = check(custom).await?;
    let help = output(&status.path, &["dl", "--help"]).await?;
    if !["--serve", "--port", "--url"]
        .iter()
        .all(|flag| help.contains(flag))
    {
        bail!("TDL_UNSUPPORTED");
    }
    Ok(status)
}

#[derive(Debug, PartialEq)]
struct Media {
    channel: u64,
    message: u32,
    size: u64,
    mime: String,
    name: String,
    container: String,
}

fn media_path(html: &str, link: &MessageLink) -> Result<(String, u64)> {
    if html.to_ascii_lowercase().matches("<a").count() != 1 {
        bail!("TDL_INVALID_RESPONSE");
    }
    let mut hrefs = html.match_indices("href=\"");
    let (start, _) = hrefs.next().context("TDL_INVALID_RESPONSE")?;
    if hrefs.next().is_some() {
        bail!("TDL_INVALID_RESPONSE");
    }
    let value = html[start + 6..]
        .split('"')
        .next()
        .context("TDL_INVALID_RESPONSE")?;
    let (channel, message) = value.split_once('/').context("TDL_INVALID_RESPONSE")?;
    let channel = positive(channel).context("TDL_INVALID_RESPONSE")?;
    let message = positive(message).context("TDL_INVALID_RESPONSE")?;
    if message != u64::from(link.message)
        || link.channel.is_some_and(|expected| expected != channel)
    {
        bail!("TDL_IDENTITY_MISMATCH");
    }
    if value != format!("{channel}/{message}") {
        bail!("TDL_INVALID_RESPONSE");
    }
    Ok((format!("/{value}"), channel))
}

fn filename(disposition: &str) -> Result<String> {
    // tdl 0.20.4 emits attachment; filename="...". No original name is used as a path.
    let value = disposition
        .strip_prefix("attachment; filename=\"")
        .and_then(|v| v.strip_suffix('"'))
        .context("TDL_INVALID_RESPONSE")?;
    let cleaned: String = value
        .chars()
        .take(160)
        .map(|c| {
            if c.is_control() || "/\\<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() {
        bail!("TDL_INVALID_RESPONSE");
    }
    Ok(cleaned.to_owned())
}

fn headers(response: &Response, full_size: Option<u64>) -> Result<(u64, String, String, String)> {
    let h = response.headers();
    if response.status() == StatusCode::PARTIAL_CONTENT {
        let total = full_size.context("TDL_INVALID_RESPONSE")?;
        let expected = format!("bytes 0-{}/{total}", total - 1);
        if h.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()) != Some(expected.as_str()) {
            bail!("INCOMPLETE_MEDIA");
        }
    } else if response.status() != StatusCode::OK {
        bail!("TDL_HTTP_ERROR");
    } else if h.contains_key(header::CONTENT_RANGE) {
        bail!("TDL_INVALID_RESPONSE");
    }
    if h.contains_key(header::CONTENT_ENCODING) {
        bail!("TDL_INVALID_RESPONSE");
    }
    let size = h
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 12 && *v <= i64::MAX as u64)
        .context("TDL_INVALID_RESPONSE")?;
    let mime = h
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .context("TDL_INVALID_RESPONSE")?
        .split(';')
        .next()
        .unwrap()
        .trim()
        .to_ascii_lowercase();
    let container = match mime.as_str() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        _ => bail!("INVALID_MEDIA_CONTENT"),
    };
    let name = filename(
        h.get(header::CONTENT_DISPOSITION)
            .and_then(|v| std::str::from_utf8(v.as_bytes()).ok())
            .context("TDL_INVALID_RESPONSE")?,
    )?;
    Ok((size, mime, name, container.to_owned()))
}

fn http_client() -> Result<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .read_timeout(IDLE_TIMEOUT)
        .build()
        .context("TDL_HTTP_ERROR")
}

struct Service {
    process: Process,
    client: Client,
    origin: url::Url,
    path: Option<String>,
}

impl Service {
    // This is the only HTTP request boundary; callers cannot supply an arbitrary URL.
    async fn request(&self, method: Method, path: &str) -> Result<Response> {
        if path != "/" && self.path.as_deref() != Some(path) {
            bail!("TDL_INVALID_RESPONSE");
        }
        let mut url = self.origin.clone();
        url.set_path(path);
        if url.host_str() != Some("127.0.0.1")
            || url.port() != self.origin.port()
            || url.query().is_some()
        {
            bail!("TDL_INVALID_RESPONSE");
        }
        self.client
            .request(method, url)
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .context("TDL_HTTP_ERROR")
    }

    async fn inspect(&mut self, link: &MessageLink, startup: Duration) -> Result<Media> {
        let started = Instant::now();
        let body = loop {
            if self
                .process
                .child
                .try_wait()
                .context("TDL_SERVICE_FAILED")?
                .is_some()
            {
                if std::net::TcpStream::connect_timeout(
                    &format!("127.0.0.1:{}", self.origin.port().unwrap())
                        .parse()
                        .unwrap(),
                    Duration::from_millis(100),
                )
                .is_ok()
                {
                    bail!("TDL_PORT_CONFLICT");
                }
                bail!("TDL_SERVICE_FAILED");
            }
            if started.elapsed() >= startup {
                bail!("TDL_START_TIMEOUT");
            }
            if let Ok(Ok(mut response)) =
                timeout(Duration::from_millis(500), self.request(Method::GET, "/")).await
            {
                if response.status() != StatusCode::OK {
                    bail!("TDL_HTTP_ERROR");
                }
                let mut body = Vec::new();
                while let Some(bytes) = timeout(Duration::from_secs(2), response.chunk())
                    .await
                    .context("TDL_START_TIMEOUT")?
                    .context("TDL_HTTP_ERROR")?
                {
                    if body.len() + bytes.len() > 65536 {
                        bail!("TDL_INVALID_RESPONSE");
                    }
                    body.extend_from_slice(&bytes);
                }
                break String::from_utf8(body).context("TDL_INVALID_RESPONSE")?;
            }
            sleep(Duration::from_millis(100)).await;
        };
        let (path, channel) = media_path(&body, link)?;
        self.path = Some(path.clone());
        let response = self.request(Method::HEAD, &path).await?;
        let (size, mime, name, container) = headers(&response, None)?;
        if self
            .process
            .child
            .try_wait()
            .context("TDL_SERVICE_FAILED")?
            .is_some()
        {
            bail!("TDL_SERVICE_FAILED");
        }
        Ok(Media {
            channel,
            message: link.message,
            size,
            mime,
            name,
            container,
        })
    }

    fn target(&self, channel: u64, message: u32) -> Target {
        Target {
            client: self.client.clone(),
            origin: self.origin.clone(),
            path: format!("/{channel}/{message}"),
            channel,
            message,
        }
    }
}

struct Target {
    client: Client,
    origin: url::Url,
    path: String,
    channel: u64,
    message: u32,
}

impl Target {
    async fn request(&self, method: Method) -> Result<Response> {
        let mut url = self.origin.clone();
        url.set_path(&self.path);
        // Neither callers nor response headers can replace this request-owned target.
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port() != self.origin.port()
            || url.query().is_some()
            || self.path != format!("/{}/{}", self.channel, self.message)
        {
            bail!("TDL_INVALID_RESPONSE");
        }
        self.client
            .request(method, url)
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .context("TDL_HTTP_ERROR")
    }

    async fn inspect(&self) -> Result<Media> {
        let response = self.request(Method::HEAD).await?;
        let (size, mime, name, container) = headers(&response, None)?;
        Ok(Media {
            channel: self.channel,
            message: self.message,
            size,
            mime,
            name,
            container,
        })
    }

    async fn download(
        &self,
        media: &Media,
        directory: &Path,
        mut progress: impl FnMut(Value),
    ) -> Result<Value> {
        let mut response = self.request(Method::GET).await?;
        let (size, mime, name, container) = headers(&response, Some(media.size))?;
        if (size, &mime, &name, &container)
            != (media.size, &media.mime, &media.name, &media.container)
        {
            bail!("TDL_MEDIA_CHANGED");
        }
        let partial = directory.join(format!("video.{}.part", media.container));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .await
            .context("IO_ERROR")?;
        let mut received = 0u64;
        let started = Instant::now();
        let mut samples = VecDeque::from([(started, 0u64)]);
        let mut last = started - Duration::from_secs(1);
        let mut head = Vec::new();
        while let Some(bytes) = response.chunk().await.context("TDL_HTTP_ERROR")? {
            if received + bytes.len() as u64 > size {
                bail!("INCOMPLETE_MEDIA");
            }
            file.write_all(&bytes).await.context("IO_ERROR")?;
            // Tokio File buffers writes on a blocking worker. Flush waits for that
            // write to finish (without fsync per chunk), so progress never leads the file.
            file.flush().await.context("IO_ERROR")?;
            // Progress accounts only for bytes after the actual write succeeds.
            received += bytes.len() as u64;
            if head.len() < 12 {
                head.extend_from_slice(&bytes[..bytes.len().min(12 - head.len())]);
            }
            let now = Instant::now();
            if last.elapsed() >= Duration::from_millis(250) || received == size {
                samples.push_back((now, received));
                while samples.len() > 2 && now.duration_since(samples[1].0) > Duration::from_secs(3)
                {
                    samples.pop_front();
                }
                let (at, count) = samples.front().unwrap();
                let speed =
                    (received - count) as f64 / now.duration_since(*at).as_secs_f64().max(0.001);
                progress(
                    json!({"bytes":received,"total_bytes":size,"speed_bps":speed,"eta_seconds":if speed > 0.0 { Some((size - received) as f64 / speed) } else { None }}),
                );
                last = now;
            }
        }
        if received != size {
            bail!("INCOMPLETE_MEDIA");
        }
        if !crate::files::video_header_valid(&media.container, &head) {
            bail!("INVALID_MEDIA_CONTENT");
        }
        file.sync_all().await.context("IO_ERROR")?;
        drop(file);
        let filename = format!("video.{}", media.container);
        tokio::fs::rename(&partial, directory.join(&filename))
            .await
            .context("IO_ERROR")?;
        Ok(
            json!({"platform":"telegram","video_id":format!("{}_{}",media.channel,media.message),"container":media.container,"filename":filename,"bytes":received,"format_id":"original"}),
        )
    }
}

// One Core owns the service; only startup/admission and final reaping take this lock.
// HTTP bodies are read outside it. The global session guard still excludes login
// and other Core instances for the entire lifetime of the shared process.
#[derive(Default)]
pub struct Session {
    running: Mutex<Option<Running>>,
}

struct Running {
    service: Service,
    config: TdlSettings,
    public_peer: Option<String>,
    channel: u64,
    users: usize,
    // Upstream caches by peer + message with no separator. Never admit two
    // numeric paths that would collide in that cache.
    paths: HashMap<String, (u64, u32)>,
    _session: MutexGuard<'static, ()>,
}

fn public_peer(link: &MessageLink) -> Option<String> {
    (link.channel.is_none()).then(|| link.url.split('/').nth(3).unwrap().to_ascii_lowercase())
}

impl Session {
    async fn acquire(
        &self,
        config: &TdlSettings,
        link: &MessageLink,
        known: Option<&MessageLink>,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<(Target, Option<Media>)> {
        let mut running = tokio::select! { biased;
            _ = cancel.wait_for(|v| *v) => bail!("CANCELED"),
            guard = timeout(STARTUP_TIMEOUT, self.running.lock()) => guard.context("TDL_START_TIMEOUT")?,
        };
        if let Some(current) = running.as_mut() {
            if current.users == 0 {
                bail!("TDL_STOP_FAILED");
            }
            if current.config != *config {
                bail!("TDL_BUSY");
            }
            if current
                .service
                .process
                .child
                .try_wait()
                .context("TDL_SERVICE_FAILED")?
                .is_some()
            {
                bail!("TDL_SERVICE_FAILED");
            }
            let channel = match known.and_then(|link| link.channel).or(link.channel) {
                Some(channel) => channel,
                None if public_peer(link) == current.public_peer => current.channel,
                None => bail!("TDL_CHANNEL_ID_REQUIRED"),
            };
            let key = format!("{channel}{}", link.message);
            if current
                .paths
                .get(&key)
                .is_some_and(|identity| *identity != (channel, link.message))
            {
                bail!("TDL_IDENTITY_MISMATCH");
            }
            current.paths.insert(key, (channel, link.message));
            current.users += 1;
            return Ok((current.service.target(channel, link.message), None));
        }
        let session = tokio::select! { biased;
            _ = cancel.wait_for(|v| *v) => bail!("CANCELED"),
            guard = timeout(STARTUP_TIMEOUT, SESSION.lock()) => guard.context("TDL_BUSY")?,
        };
        let status = tokio::select! { biased;
            _ = cancel.wait_for(|v| *v) => bail!("CANCELED"),
            status = check_serve(config.path.as_deref()) => status?,
        };
        let reservation = TcpListener::bind(("0.0.0.0", 0)).context("TDL_PORT_CONFLICT")?;
        let port = reservation
            .local_addr()
            .context("TDL_PORT_CONFLICT")?
            .port();
        let mut cmd = command(&status.path);
        cmd.args([
            "dl",
            "-u",
            &link.url,
            "--serve",
            "--port",
            &port.to_string(),
        ]);
        if let Some(directory) = &config.session_directory {
            cmd.arg("--storage")
                .arg(format!(
                    "type=bolt,\"path={}\"",
                    directory.to_string_lossy().replace('"', "\"\"")
                ))
                .args(["--ns", "yingqu"]);
        }
        drop(reservation);
        let client = http_client()?;
        let process = Process::spawn(cmd)?;
        let mut service = Service {
            process,
            client,
            origin: url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
            path: None,
        };
        let inspected = tokio::select! { biased;
            _ = cancel.wait_for(|v| *v) => Err(anyhow::anyhow!("CANCELED")),
            result = timeout(STARTUP_TIMEOUT, service.inspect(link, STARTUP_TIMEOUT)) => result.context("TDL_START_TIMEOUT").and_then(|r| r),
        };
        let media = match inspected {
            Ok(media) => media,
            Err(error) => {
                service.process.stop().await?;
                return Err(error);
            }
        };
        let target = service.target(media.channel, media.message);
        *running = Some(Running {
            service,
            config: config.clone(),
            public_peer: public_peer(link),
            channel: media.channel,
            users: 1,
            paths: HashMap::from([(
                format!("{}{}", media.channel, media.message),
                (media.channel, media.message),
            )]),
            _session: session,
        });
        Ok((target, Some(media)))
    }

    pub async fn operation(
        &self,
        config: &TdlSettings,
        input: &str,
        expected: Option<&Value>,
        directory: Option<&Path>,
        progress: impl FnMut(Value),
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Value> {
        if !config.enabled {
            bail!("TDL_EXPERIMENTAL_DISABLED");
        }
        let link = message_link(input)?;
        let known = expected
            .map(|video| {
                let canonical = video["canonical_url"]
                    .as_str()
                    .context("TDL_IDENTITY_MISMATCH")?;
                let id = video["video_id"]
                    .as_str()
                    .context("TDL_IDENTITY_MISMATCH")?;
                if !validate_identity(id, canonical) {
                    bail!("TDL_IDENTITY_MISMATCH");
                }
                let known = message_link(canonical)?;
                if known.message != link.message
                    || link.channel.is_some_and(|id| Some(id) != known.channel)
                {
                    bail!("TDL_IDENTITY_MISMATCH");
                }
                Ok(known)
            })
            .transpose()?;
        let (target, initial) = self
            .acquire(config, &link, known.as_ref(), &mut cancel)
            .await?;
        let bootstrap_public = initial.is_some() && link.channel.is_none();
        let result = tokio::select! { biased;
            _ = cancel.wait_for(|v| *v) => Err(anyhow::anyhow!("CANCELED")),
            result = timeout(Duration::from_secs(6 * 60 * 60), async {
                let media = match initial {
                    Some(media) => media,
                    None => timeout(STARTUP_TIMEOUT, target.inspect()).await.context("TDL_START_TIMEOUT")??,
                };
                if let Some(expected) = expected {
                    if expected["video_id"] != format!("{}_{}",media.channel,media.message) {
                        if bootstrap_public {
                            return Err(anyhow::anyhow!(ALIAS_IDENTITY_MISMATCH).context("TDL_IDENTITY_MISMATCH"));
                        }
                        bail!("TDL_IDENTITY_MISMATCH");
                    }
                    if expected["selected_format_id"] != "original" { bail!("FORMAT_UNAVAILABLE"); }
                    if expected["filesize"] != media.size || expected["mime_type"] != media.mime || expected["original_filename"] != media.name { bail!("TDL_MEDIA_CHANGED"); }
                    target.download(&media, directory.context("IO_ERROR")?, progress).await
                } else {
                    Ok(json!({"platform":"telegram","resolver":"tdl","kind":"video","video_id":format!("{}_{}",media.channel,media.message),
                        "canonical_url":format!("https://t.me/c/{}/{}",media.channel,media.message),"title":media.name,"original_filename":media.name,"mime_type":media.mime,
                        "filesize":media.size,"duration_ms":null,"width":null,"height":null,"selected_format_id":"original","tdl_revision":PROTOCOL_REVISION,
                        "formats":[{"format_id":"original","container":media.container,"codec":null,"width":null,"height":null,"filesize":media.size}]}))
                }
            }) => result.context("TIMEOUT").and_then(|r| r),
        };
        // Dropping this request stops only its HTTP stream. Reap the process
        // before the final borrower publishes a terminal state or commits a file.
        let mut running = self.running.lock().await;
        let current = running.as_mut().expect("admitted service borrower");
        current.users -= 1;
        if current.users == 0 {
            current.service.process.stop().await?;
            *running = None;
        }
        result
    }
}

#[cfg(test)]
#[path = "tdl_bridge_tests.rs"]
mod tests;

#[path = "tdl_login.rs"]
mod login;
pub use login::{login, LoginControl, LoginStatus};
