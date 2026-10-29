use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const MAX_LINE: u64 = 1024 * 1024;
pub const ENGINE_VERSION: &str = "0.17.0";
pub const YTDLP_VERSION: &str = "2026.08.19";

async fn cancellation() -> Result<()> {
    #[cfg(windows)]
    {
        let mut signal = tokio::signal::windows::ctrl_break().context("ENGINE_UNAVAILABLE")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("ENGINE_UNAVAILABLE")?,
            _ = signal.recv() => {},
        }
    }
    #[cfg(not(windows))]
    tokio::signal::ctrl_c()
        .await
        .context("ENGINE_UNAVAILABLE")?;
    Ok(())
}

pub struct PythonBridge {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    process_group: i32,
}

#[cfg(windows)]
pub(crate) fn process_job(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
    process_job_handle(child.raw_handle().context("ENGINE_UNAVAILABLE")?)
}

#[cfg(windows)]
pub(crate) fn process_job_handle(
    process: std::os::windows::io::RawHandle,
) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            bail!("ENGINE_UNAVAILABLE");
        }
        let job = OwnedHandle::from_raw_handle(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
            || AssignProcessToJobObject(job.as_raw_handle(), process) == 0
        {
            bail!("ENGINE_UNAVAILABLE");
        }
        Ok(job)
    }
}

impl Drop for PythonBridge {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
        // Windows OwnedHandle closes the kill-on-close job, including FFmpeg/JS descendants.
    }
}

#[cfg(windows)]
async fn reap_job(job: &std::os::windows::io::OwnedHandle) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;
    // Kill-on-close alone is asynchronous. Native ASR descendants can still hold
    // WAV/GGUF files when the caller immediately removes its staging directory.
    unsafe {
        if TerminateJobObject(job.as_raw_handle(), 1) == 0 {
            bail!("ENGINE_PROTOCOL_ERROR");
        }
    }
    timeout(Duration::from_secs(5), async {
        loop {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            let queried = unsafe {
                QueryInformationJobObject(job.as_raw_handle(), JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _, std::mem::size_of_val(&info) as u32, std::ptr::null_mut())
            };
            if queried == 0 { bail!("ENGINE_PROTOCOL_ERROR"); }
            if info.ActiveProcesses == 0 { return Ok(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.context("TIMEOUT")?
}

async fn read_message(reader: &mut (impl AsyncBufReadExt + Unpin)) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE + 1)
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_LINE || bytes.last() != Some(&b'\n') {
        bail!("ENGINE_PROTOCOL_ERROR");
    }
    Ok(Some(
        serde_json::from_slice(&bytes).context("ENGINE_PROTOCOL_ERROR")?,
    ))
}

pub(crate) fn error_code(value: &Value) -> &'static str {
    match value["error"]["code"].as_str() {
        Some("INVALID_INPUT") => "INVALID_INPUT",
        Some("UNSUPPORTED_PLATFORM") => "UNSUPPORTED_PLATFORM",
        Some("UNSUPPORTED_CONTENT") => "UNSUPPORTED_CONTENT",
        Some("CONFIG_INVALID") => "CONFIG_INVALID",
        Some("ACCESS_DENIED") => "ACCESS_DENIED",
        Some("NOT_FOUND") => "NOT_FOUND",
        Some("RISK_CONTROL") => "RISK_CONTROL",
        Some("NETWORK_ERROR") => "NETWORK_ERROR",
        Some("UPSTREAM_CHANGED") => "UPSTREAM_CHANGED",
        Some("ENGINE_UNAVAILABLE") => "ENGINE_UNAVAILABLE",
        Some("TIMEOUT") => "TIMEOUT",
        Some("IO_ERROR") => "IO_ERROR",
        Some("INVALID_MEDIA_CONTENT") => "INVALID_MEDIA_CONTENT",
        Some("LOCAL_VIDEO_INVALID") => "LOCAL_VIDEO_INVALID",
        Some("FILE_MISSING") => "FILE_MISSING",
        Some("KEYFRAME_RANGE_INVALID") => "KEYFRAME_RANGE_INVALID",
        Some("KEYFRAME_RANGE_UNAVAILABLE") => "KEYFRAME_RANGE_UNAVAILABLE",
        Some("KEYFRAME_FAILED") => "KEYFRAME_FAILED",
        Some("FORMAT_UNAVAILABLE") => "FORMAT_UNAVAILABLE",
        Some("FFMPEG_UNAVAILABLE") => "FFMPEG_UNAVAILABLE",
        Some("JS_RUNTIME_UNAVAILABLE") => "JS_RUNTIME_UNAVAILABLE",
        Some("BROWSER_COOKIES_UNAVAILABLE") => "BROWSER_COOKIES_UNAVAILABLE",
        Some("MEDIA_MERGE_FAILED") => "MEDIA_MERGE_FAILED",
        Some("DEPENDENCY_UNSUPPORTED_SYSTEM") => "DEPENDENCY_UNSUPPORTED_SYSTEM",
        Some("DEPENDENCY_NETWORK_ERROR") => "DEPENDENCY_NETWORK_ERROR",
        Some("DEPENDENCY_TIMEOUT") => "DEPENDENCY_TIMEOUT",
        Some("DEPENDENCY_CHECKSUM_MISMATCH") => "DEPENDENCY_CHECKSUM_MISMATCH",
        Some("DEPENDENCY_INVALID_ARCHIVE") => "DEPENDENCY_INVALID_ARCHIVE",
        Some("DEPENDENCY_VALIDATION_FAILED") => "DEPENDENCY_VALIDATION_FAILED",
        Some("TRANSCRIPTION_ENGINE_UNAVAILABLE") => "TRANSCRIPTION_ENGINE_UNAVAILABLE",
        Some("TRANSCRIPTION_MODEL_UNAVAILABLE") => "TRANSCRIPTION_MODEL_UNAVAILABLE",
        Some("TRANSCRIPTION_NO_AUDIO") => "TRANSCRIPTION_NO_AUDIO",
        Some("TRANSCRIPTION_AUDIO_FAILED") => "TRANSCRIPTION_AUDIO_FAILED",
        Some("TRANSCRIPTION_FAILED") => "TRANSCRIPTION_FAILED",
        Some("TRANSCRIPTION_VAD_FAILED") => "TRANSCRIPTION_VAD_FAILED",
        Some("TRANSCRIPTION_CUDA_FAILED") => "TRANSCRIPTION_CUDA_FAILED",
        Some("TRANSCRIPTION_CUDA_UNAVAILABLE") => "TRANSCRIPTION_CUDA_UNAVAILABLE",
        Some("CUDA_DRIVER_UNAVAILABLE") => "CUDA_DRIVER_UNAVAILABLE",
        Some("CUDA_DRIVER_TOO_OLD") => "CUDA_DRIVER_TOO_OLD",
        Some("CUDA_UNSUPPORTED_GPU") => "CUDA_UNSUPPORTED_GPU",
        Some("CUDA_UNSUPPORTED_SYSTEM") => "CUDA_UNSUPPORTED_SYSTEM",
        Some("TRANSCRIPTION_TIMEOUT") => "TRANSCRIPTION_TIMEOUT",
        Some("TRANSCRIPTION_OUTPUT_TOO_LARGE") => "TRANSCRIPTION_OUTPUT_TOO_LARGE",
        _ => "ENGINE_PROTOCOL_ERROR",
    }
}

impl PythonBridge {
    fn start(program: &str, args: &[String]) -> Result<Self> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().context("ENGINE_UNAVAILABLE")?;
        #[cfg(windows)]
        let job = process_job(&child)?;
        #[cfg(unix)]
        let process_group = child.id().context("ENGINE_UNAVAILABLE")? as i32;
        Ok(Self {
            input: child.stdin.take().context("ENGINE_UNAVAILABLE")?,
            output: BufReader::new(child.stdout.take().context("ENGINE_UNAVAILABLE")?),
            child,
            #[cfg(windows)]
            _job: job,
            #[cfg(unix)]
            process_group,
        })
    }

    async fn execute(
        &mut self,
        id: &str,
        method: &str,
        params: Value,
        mut progress: impl FnMut(Value),
    ) -> Result<Value> {
        let dtk: Value = serde_json::from_str(include_str!("../../vendor/dtk/manifest.json"))
            .context("ENGINE_UNAVAILABLE")?;
        let ready = timeout(Duration::from_secs(15), read_message(&mut self.output))
            .await
            .context("TIMEOUT")??
            .context("ENGINE_PROTOCOL_ERROR")?;
        if ready["protocol_version"] != 3
            || ready["type"] != "ready"
            || ready["engine_version"] != ENGINE_VERSION
            || ready["yt_dlp_version"] != YTDLP_VERSION
            || ready["dtk_version"] != dtk["upstream_version"]
            || ready["dtk_commit"] != dtk["upstream_commit"]
        {
            bail!("ENGINE_PROTOCOL_ERROR");
        }
        let mut bytes = serde_json::to_vec(&json!({"protocol_version":3,"id":id,
                                                   "method":method,"params":params}))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_LINE {
            bail!("ENGINE_PROTOCOL_ERROR");
        }
        timeout(Duration::from_secs(15), async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await
        })
        .await
        .context("TIMEOUT")??;
        let mut downloaded = 0;
        let mut known_total = None;
        let mut saw_progress = false;
        let youtube = params["expected"]["platform"] == "youtube";
        let mut merging = false;
        let mut recognizing = false;
        let mut transcription_segments: Option<(u64, u64)> = None;
        let mut phase_deadline = tokio::time::Instant::now() + Duration::from_secs(1800);
        loop {
            let wait = if method == "transcribe_video" {
                phase_deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .min(Duration::from_secs(65))
            } else {
                Duration::from_secs(if merging { 600 } else { 65 })
            };
            let value = timeout(wait, read_message(&mut self.output))
                .await
                .context("TIMEOUT")??
                .context("ENGINE_PROTOCOL_ERROR")?;
            if value["id"] != id || value["protocol_version"] != 3 {
                bail!("ENGINE_PROTOCOL_ERROR");
            }
            if method == "extract_keyframes" && value["type"] == "keyframe_progress" {
                let completed = value["completed"].as_u64().context("ENGINE_PROTOCOL_ERROR")?;
                let total = value["total"].as_u64().context("ENGINE_PROTOCOL_ERROR")?;
                if total != params["positions_ms"].as_array().map(|p|p.len() as u64).unwrap_or(0) || total > 9 || completed != downloaded + 1 || completed > total { bail!("ENGINE_PROTOCOL_ERROR"); }
                downloaded = completed; progress(json!({"completed":completed,"total":total})); continue;
            }
            if method == "transcribe_video" && value["type"] == "transcription_progress" {
                match value["phase"].as_str() {
                    Some("extracting_audio") if !recognizing => {}
                    Some("segmenting" | "loading_model" | "recognizing") => {
                        let duration = value["duration_seconds"]
                            .as_f64()
                            .filter(|n| n.is_finite() && *n > 0.0 && *n <= 604800.0)
                            .context("ENGINE_PROTOCOL_ERROR")?;
                        if !recognizing {
                            phase_deadline = tokio::time::Instant::now()
                                + Duration::from_secs_f64((3.0 * duration + 120.0).max(600.0));
                            recognizing = true;
                        }
                    }
                    _ => bail!("ENGINE_PROTOCOL_ERROR"),
                }
                if !matches!(value["backend"].as_str(), Some("cpu" | "cuda")) {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                let completed = &value["completed_segments"];
                let total = &value["total_segments"];
                if !completed.is_null() || !total.is_null() {
                    let completed = completed.as_u64().context("ENGINE_PROTOCOL_ERROR")?;
                    let total = total.as_u64().context("ENGINE_PROTOCOL_ERROR")?;
                    if completed > total || total > u32::MAX as u64
                        || !matches!(value["phase"].as_str(), Some("loading_model" | "recognizing"))
                        || (value["phase"] == "loading_model" && completed != 0)
                        || transcription_segments.is_some_and(|(old, size)| completed < old || total != size)
                    {
                        bail!("ENGINE_PROTOCOL_ERROR");
                    }
                    transcription_segments = Some((completed, total));
                } else if value["phase"] == "recognizing" || transcription_segments.is_some() {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                progress(json!({"phase":value["phase"], "backend":value["backend"],
                    "completed_segments":completed,"total_segments":total}));
                continue;
            }
            if value["type"] == "dependency_progress" && method == "install_dependency" {
                let count = value["downloaded_bytes"]
                    .as_u64()
                    .context("ENGINE_PROTOCOL_ERROR")?;
                let total = value["total_bytes"]
                    .as_u64()
                    .context("ENGINE_PROTOCOL_ERROR")?;
                if value["tool"] != params["tool"]
                    || count < downloaded
                    || count > total
                    || (total == 0 && value["phase"] != "installing")
                    || !matches!(
                        value["phase"].as_str(),
                        Some("downloading" | "verifying" | "installing")
                    )
                {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                downloaded = count;
                progress(
                    json!({"tool":value["tool"], "phase":value["phase"], "downloaded_bytes":count, "total_bytes":total}),
                );
                continue;
            }
            if value["type"] == "progress" && method == "download_video" {
                if !value["phase"].is_null() {
                    if !youtube || value["phase"] != "merging" || merging {
                        bail!("ENGINE_PROTOCOL_ERROR");
                    }
                    merging = true;
                }
                let count = value["downloaded_bytes"]
                    .as_u64()
                    .context("ENGINE_PROTOCOL_ERROR")?;
                let total = if value["total_bytes"].is_null() {
                    None
                } else {
                    Some(
                        value["total_bytes"]
                            .as_u64()
                            .context("ENGINE_PROTOCOL_ERROR")?,
                    )
                };
                let speed = &value["speed_bps"];
                let estimate = &value["total_bytes_estimate"];
                let eta = &value["eta_seconds"];
                if count < downloaded
                    || count > 9_007_199_254_740_991
                    || total.is_some_and(|n| n == 0 || n > 9_007_199_254_740_991)
                    || total.is_some_and(|n| n < count)
                    || (!estimate.is_null()
                        && (total.is_some()
                            || !estimate
                                .as_u64()
                                .is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)))
                    || (!speed.is_null()
                        && !speed
                            .as_f64()
                            .is_some_and(|n| n.is_finite() && (0.0..=1e12).contains(&n)))
                    || (!eta.is_null()
                        && !eta
                            .as_f64()
                            .is_some_and(|n| n.is_finite() && (0.0..=604800.0).contains(&n)))
                {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                downloaded = count;
                known_total = total.or(known_total);
                saw_progress = true;
                let completed_items = value["completed_items"].as_u64();
                let total_items = value["total_items"].as_u64();
                if completed_items.is_some() != total_items.is_some()
                    || total_items
                        .is_some_and(|n| n == 0 || n > 64 || completed_items.unwrap_or(0) > n)
                {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                progress(json!({"id":id,"event":"progress","bytes":count,
                               "total_bytes":total,"speed_bps":speed,
                               "total_bytes_estimate":estimate,"eta_seconds":eta,
                               "phase":value["phase"],
                               "completed_items":completed_items,"total_items":total_items}));
                continue;
            }
            let failed = value["type"] == "error";
            if !failed
                && value["type"]
                    != match method {
                        "identify_input" => "identified",
                        "resolve_video" => "resolved",
                        "check_dependencies" | "install_dependency" => "dependencies",
                        "extract_cover" => "cover_extracted",
                        "probe_local_video" => "local_video_probed",
                        "extract_keyframes" => "keyframes_extracted",
                        "transcribe_video" => "transcribed",
                        _ => "download_finished",
                    }
            {
                bail!("ENGINE_PROTOCOL_ERROR");
            }
            // A terminal message alone is not success: require EOF, no duplicate,
            // and a clean process exit. Keep stdin open until the operation exits.
            let status = timeout(Duration::from_secs(5), async {
                if read_message(&mut self.output).await?.is_some() {
                    bail!("ENGINE_PROTOCOL_ERROR");
                }
                Ok(self.child.wait().await?)
            })
            .await
            .context("TIMEOUT")??;
            if failed {
                bail!("{}", error_code(&value));
            }
            if !status.success() {
                bail!("ENGINE_PROTOCOL_ERROR");
            }
            if method == "extract_keyframes" && downloaded != params["positions_ms"].as_array().map(|p|p.len() as u64).unwrap_or(0) { bail!("ENGINE_PROTOCOL_ERROR"); }
            if method == "download_video" {
                let size = value["bytes"].as_u64().context("ENGINE_PROTOCOL_ERROR")?;
                let transferred = if youtube {
                    value["transferred_bytes"]
                        .as_u64()
                        .context("ENGINE_PROTOCOL_ERROR")?
                } else {
                    size
                };
                if size == 0
                    || !saw_progress
                    || downloaded != transferred
                    || known_total.is_some_and(|n| n != transferred)
                {
                    bail!("INCOMPLETE_MEDIA");
                }
            }
            return Ok(value);
        }
    }

    pub async fn operation(
        program: &str,
        args: &[String],
        id: &str,
        method: &str,
        params: Value,
        progress: impl FnMut(Value),
    ) -> Result<Value> {
        Self::operation_cancellable(program, args, id, method, params, progress, async {
            let _ = cancellation().await;
        })
        .await
    }

    pub async fn operation_cancellable(
        program: &str,
        args: &[String],
        id: &str,
        method: &str,
        params: Value,
        progress: impl FnMut(Value),
        cancel: impl std::future::Future<Output = ()>,
    ) -> Result<Value> {
        let mut bridge = Self::start(program, args)?;
        let result = tokio::select! {
            result = async {
                if !matches!(method, "download_video" | "transcribe_video") {
                    let limit = match method { "install_dependency" if params["tool"] == "fun-asr-nano" => 2700, "install_dependency" => 600, "extract_cover" => 20, "probe_local_video" => 40, "extract_keyframes" => 240, _ => 80 };
                    timeout(Duration::from_secs(limit), bridge.execute(id, method, params, progress))
                        .await.context("TIMEOUT")?
                } else {
                    bridge.execute(id, method, params, progress).await
                }
            } => result,
            _ = cancel => Err(anyhow::anyhow!("CANCELED")),
        };
        // Also runs on protocol errors/timeouts/cancel, before caller cleans staging.
        #[cfg(unix)]
        unsafe {
            libc::kill(-bridge.process_group, libc::SIGKILL);
        }
        let _ = bridge.child.kill().await;
        let _ = bridge.child.wait().await;
        #[cfg(unix)]
        timeout(Duration::from_secs(5), async {
            // Killing the group is asynchronous. Do not publish a terminal state
            // while native ASR descendants can still hold audio/model files.
            loop {
                if unsafe { libc::kill(-bridge.process_group, 0) } == -1 {
                    let errno = std::io::Error::last_os_error().raw_os_error();
                    if errno == Some(libc::ESRCH) {
                        return Ok::<_, anyhow::Error>(());
                    }
                    // EPERM still denotes an existing group, not completed cleanup.
                    if errno != Some(libc::EPERM) {
                        bail!("ENGINE_PROTOCOL_ERROR");
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("TIMEOUT")??;
        #[cfg(windows)]
        reap_job(&bridge._job).await?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_waits_for_engine_and_native_descendant_exit() {
        let root = tempfile::tempdir().unwrap();
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let stage = root.path().join("stage");
        std::fs::create_dir(&stage).unwrap();
        let marker = root.path().join("mcp-transcription-started.json");
        let program = project.join(".venv/bin/python");
        let result = PythonBridge::operation_cancellable(
            program.to_str().unwrap(),
            &[project.join("tests/fixtures/desktop_engine.py").to_string_lossy().into_owned()],
            "cancel-descendants", "transcribe_video",
            json!({"source":root.path().join("hold-mcp.mp4"), "staging_dir":stage,
                   "runtime_directory":root.path().join("runtime")}),
            |_| {},
            async {
                timeout(Duration::from_secs(5), async {
                    while !marker.exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.unwrap();
            },
        ).await;
        assert_eq!(result.unwrap_err().to_string(), "CANCELED");
        let processes: Value = serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
        for key in ["engine", "child"] {
            assert_eq!(unsafe { libc::kill(processes[key].as_i64().unwrap() as i32, 0) }, -1, "{key} survived cancellation");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        }
    }

    #[tokio::test]
    async fn frames_and_limits() {
        let mut reader = BufReader::new(&b"{\"id\":1}\n{\"id\":2}\n"[..]);
        assert_eq!(read_message(&mut reader).await.unwrap().unwrap()["id"], 1);
        assert_eq!(read_message(&mut reader).await.unwrap().unwrap()["id"], 2);
        assert!(read_message(&mut reader).await.unwrap().is_none());
        let data = vec![b'x'; MAX_LINE as usize + 1];
        assert!(read_message(&mut BufReader::new(data.as_slice()))
            .await
            .is_err());
        assert!(read_message(&mut BufReader::new(&b"not json\n"[..]))
            .await
            .is_err());
    }
}
