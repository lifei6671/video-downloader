use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const MAX_LINE: u64 = 1024 * 1024;
pub const ENGINE_VERSION: &str = "0.6.0";
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
fn process_job(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
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
            || AssignProcessToJobObject(
                job.as_raw_handle(),
                child.raw_handle().context("ENGINE_UNAVAILABLE")?,
            ) == 0
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

fn error_code(value: &Value) -> &'static str {
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
        Some("FORMAT_UNAVAILABLE") => "FORMAT_UNAVAILABLE",
        Some("FFMPEG_UNAVAILABLE") => "FFMPEG_UNAVAILABLE",
        Some("JS_RUNTIME_UNAVAILABLE") => "JS_RUNTIME_UNAVAILABLE",
        Some("BROWSER_COOKIES_UNAVAILABLE") => "BROWSER_COOKIES_UNAVAILABLE",
        Some("MEDIA_MERGE_FAILED") => "MEDIA_MERGE_FAILED",
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
        loop {
            let value = timeout(
                Duration::from_secs(if merging { 600 } else { 65 }),
                read_message(&mut self.output),
            )
            .await
            .context("TIMEOUT")??
            .context("ENGINE_PROTOCOL_ERROR")?;
            if value["id"] != id || value["protocol_version"] != 3 {
                bail!("ENGINE_PROTOCOL_ERROR");
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
                if count < downloaded
                    || total.is_some_and(|n| n < count)
                    || (!speed.is_null()
                        && !speed.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
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
                if method != "download_video" {
                    timeout(Duration::from_secs(80), bridge.execute(id, method, params, progress))
                        .await.context("TIMEOUT")?
                } else {
                    bridge.execute(id, method, params, progress).await
                }
            } => result,
            _ = cancel => Err(anyhow::anyhow!("CANCELED")),
        };
        // Also runs on protocol errors/timeouts/cancel, before caller cleans staging.
        let _ = bridge.child.kill().await;
        let _ = bridge.child.wait().await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
