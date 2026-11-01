//! Local Telegram media inspection/extraction without the Python sidecar.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

struct Process {
    child: Child,
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    group: i32,
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }
}
async fn run(
    program: &Path,
    args: &[OsString],
    limit: Duration,
    cancel: impl Future<Output = ()>,
) -> Result<Vec<u8>> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().context("VIDEO_METADATA_UNAVAILABLE")?;
    #[cfg(windows)]
    let job = crate::python_bridge::process_job(&child)?;
    #[cfg(unix)]
    let group = child.id().context("VIDEO_METADATA_UNAVAILABLE")? as i32;
    let mut process = Process {
        child,
        #[cfg(windows)]
        job,
        #[cfg(unix)]
        group,
    };
    let output = process
        .child
        .stdout
        .take()
        .context("VIDEO_METADATA_UNAVAILABLE")?;
    let result = tokio::select! {biased;
        _=cancel=>Err(anyhow::anyhow!("CANCELED")),
        result=tokio::time::timeout(limit,async {
            let mut bytes=vec![];
            output.take(1024*1024+1).read_to_end(&mut bytes).await.context("VIDEO_METADATA_UNAVAILABLE")?;
            anyhow::ensure!(bytes.len()<=1024*1024,"VIDEO_METADATA_UNAVAILABLE");
            let status=process.child.wait().await.context("VIDEO_METADATA_UNAVAILABLE")?;
            anyhow::ensure!(status.success(),"VIDEO_METADATA_UNAVAILABLE");
            Ok(bytes)
        })=>result.context("TIMEOUT").and_then(|r|r),
    };
    let _ = process.child.kill().await;
    let _ = process.child.wait().await;
    #[cfg(windows)]
    crate::python_bridge::reap_job(&process.job).await?;
    result
}
fn directories(root: &Path) -> Vec<PathBuf> {
    let mut paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    paths.extend([home.join(".local/bin"), home.join("bin")]);
    if let Some(prefix) = std::env::var_os("CONDA_PREFIX") {
        paths.push(PathBuf::from(prefix).join(if cfg!(windows) { "Library/bin" } else { "bin" }));
    }
    #[cfg(windows)]
    {
        paths.extend(registry_paths());
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        paths.extend([
            local.join("Microsoft/WinGet/Links"),
            home.join("scoop/shims"),
            PathBuf::from(
                std::env::var_os("ChocolateyInstall")
                    .unwrap_or_else(|| "C:/ProgramData/chocolatey".into()),
            )
            .join("bin"),
            PathBuf::from(
                std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:/Program Files".into()),
            )
            .join("ffmpeg/bin"),
        ]);
        for directory in children(&local.join("Microsoft/WinGet/Packages")) {
            if directory
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains("FFmpeg"))
            {
                paths.push(directory.join("bin"));
                paths.extend(children(&directory).into_iter().map(|p| p.join("bin")));
            }
        }
        paths.extend(
            children(&home.join("scoop/apps"))
                .into_iter()
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().contains("ffmpeg"))
                })
                .map(|p| p.join("current/bin")),
        );
    }
    #[cfg(target_os = "macos")]
    {
        paths.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/opt/local/bin",
                "/usr/bin",
            ]
            .map(PathBuf::from),
        );
        let mut files = vec![PathBuf::from("/etc/paths")];
        files.extend(children(Path::new("/etc/paths.d")));
        paths.extend([
            home.join(".nix-profile/bin"),
            PathBuf::from("/nix/var/nix/profiles/default/bin"),
            home.join("miniconda3/bin"),
            home.join("anaconda3/bin"),
        ]);
        for file in files {
            if let Ok(body) = std::fs::read_to_string(file) {
                paths.extend(
                    body.lines()
                        .map(str::trim)
                        .filter(|s| s.starts_with('/'))
                        .map(PathBuf::from),
                );
            }
        }
        for prefix in [Path::new("/opt/homebrew"), Path::new("/usr/local")] {
            for formula in children(&prefix.join("Cellar")).into_iter().filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("ffmpeg"))
            }) {
                paths.extend(children(&formula).into_iter().map(|p| p.join("bin")));
            }
        }
        if let Some(prefix) = std::env::var_os("HOMEBREW_PREFIX") {
            paths.push(PathBuf::from(prefix).join("bin"));
        }
    }
    paths.push(root.join("ffmpeg"));
    paths.retain(|p| p.is_absolute());
    paths
}
#[cfg(any(windows, target_os = "macos"))]
fn children(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect()
}
#[cfg(windows)]
fn registry_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::*;
    let mut paths = vec![];
    for (hive, key) in [
        (HKEY_CURRENT_USER, "Environment"),
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
    ] {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let name: Vec<u16> = "Path".encode_utf16().chain(Some(0)).collect();
        let mut body = vec![0u16; 32768];
        let mut length = (body.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                std::ptr::null_mut(),
                body.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if status == 0 {
            let text = String::from_utf16_lossy(&body[..(length as usize / 2).saturating_sub(1)]);
            paths.extend(text.split(';').map(|s| PathBuf::from(s.trim_matches('"'))));
        }
    }
    paths
}
async fn tool(name: &str, runtime: &Path, cancel: impl Future<Output = ()>) -> Result<PathBuf> {
    tokio::pin!(cancel);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut seen = std::collections::HashSet::new();
    for directory in directories(runtime) {
        let path = directory.join(format!("{name}{}", if cfg!(windows) { ".exe" } else { "" }));
        if !seen.insert(path.clone()) || !path.is_file() {
            continue;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match run(
            &path,
            &["-version".into()],
            Duration::from_secs(5).min(remaining),
            &mut cancel,
        )
        .await
        {
            Ok(bytes) if bytes.starts_with(format!("{name} version ").as_bytes()) => {
                return Ok(path)
            }
            Err(error) if error.to_string() == "CANCELED" => return Err(error),
            _ => {}
        }
    }
    bail!(
        "{}",
        if name == "ffprobe" {
            "FFPROBE_UNAVAILABLE"
        } else {
            "COVER_UNAVAILABLE"
        }
    )
}

fn attributes(value: &Value) -> Result<Value> {
    for stream in value["streams"]
        .as_array()
        .context("VIDEO_METADATA_UNAVAILABLE")?
    {
        if stream["codec_type"] != "video" || stream["disposition"]["attached_pic"] == 1 {
            continue;
        }
        let width = stream["width"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= i32::MAX as u64);
        let height = stream["height"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= i32::MAX as u64);
        let raw = stream["duration"]
            .as_str()
            .filter(|s| *s != "N/A")
            .or_else(|| value["format"]["duration"].as_str());
        let seconds = raw
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n > 0.0 && *n <= (u64::MAX / 1000) as f64);
        if let (Some(mut width), Some(mut height), Some(seconds)) = (width, height, seconds) {
            let rotation = stream["side_data_list"]
                .as_array()
                .and_then(|a| a.iter().find_map(|i| i["rotation"].as_f64()))
                .unwrap_or(0.0);
            if rotation.is_finite() && (rotation.round().abs() % 180.0) == 90.0 {
                std::mem::swap(&mut width, &mut height);
            }
            return Ok(
                json!({"width":width,"height":height,"duration_ms":((seconds*1000.0).round() as u64).max(1)}),
            );
        }
    }
    bail!("VIDEO_METADATA_UNAVAILABLE")
}
fn local_file(source: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(source).context("VIDEO_METADATA_UNAVAILABLE")?;
    anyhow::ensure!(
        source.is_absolute() && m.is_file() && !m.file_type().is_symlink() && m.len() > 0,
        "VIDEO_METADATA_UNAVAILABLE"
    );
    Ok(())
}
pub(crate) async fn probe(
    source: &Path,
    runtime: &Path,
    cancel: impl Future<Output = ()>,
) -> Result<Value> {
    local_file(source)?;
    tokio::pin!(cancel);
    let program = tool("ffprobe", runtime, &mut cancel).await?;
    let mut args:Vec<OsString>=["-v","error","-protocol_whitelist","file","-show_entries","stream=codec_type,width,height,duration:stream_disposition=attached_pic:stream_side_data=rotation:format=duration","-of","json","-i"].into_iter().map(Into::into).collect();
    args.push(source.as_os_str().into());
    let bytes = run(&program, &args, Duration::from_secs(20), cancel).await?;
    attributes(&serde_json::from_slice(&bytes).context("VIDEO_METADATA_UNAVAILABLE")?)
}
pub(crate) async fn extract(
    source: &Path,
    runtime: &Path,
    cancel: impl Future<Output = ()>,
) -> Result<Value> {
    local_file(source)?;
    tokio::pin!(cancel);
    let program = tool("ffmpeg", runtime, &mut cancel).await?;
    let cover = source.parent().context("COVER_UNAVAILABLE")?.join("cover");
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-n",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "mov,matroska,webm",
        "-t",
        "30",
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.push(source.as_os_str().into());
    args.extend(["-map","0:v:0","-frames:v","1","-vf","select='isnan(prev_selected_t)+gte(t-prev_selected_t,0.5)',format=yuv420p,blackframe=amount=0:threshold=24,metadata=mode=select:key=lavfi.blackframe.pblack:value=98:function=less,scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease","-c:v","mjpeg","-q:v","3","-f","image2","-update","1"].into_iter().map(OsString::from));
    args.push(cover.as_os_str().into());
    run(&program, &args, Duration::from_secs(10), cancel).await?;
    let metadata = std::fs::symlink_metadata(&cover).context("COVER_UNAVAILABLE")?;
    anyhow::ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= crate::telegram_cover::MAX_BYTES as u64,
        "COVER_UNAVAILABLE"
    );
    let body = std::fs::read(&cover).context("COVER_UNAVAILABLE")?;
    anyhow::ensure!(crate::telegram_cover::valid(&body), "COVER_UNAVAILABLE");
    Ok(json!({"cover_filename":"cover","cover_bytes":body.len()}))
}

#[cfg(test)]
pub(crate) type Fixture = std::sync::Arc<dyn Fn(&str, &Path) -> Result<Value> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rotation_and_rejects_unknown_or_invalid_metadata() {
        let metadata = json!({"streams":[{"codec_type":"video","width":1920,"height":1080,"duration":"N/A","side_data_list":[{"rotation":-90}]}],"format":{"duration":"2.5"}});
        assert_eq!(
            attributes(&metadata).unwrap(),
            json!({"width":1080,"height":1920,"duration_ms":2500})
        );
        for duration in ["NaN", "0", "-1", "inf"] {
            let mut bad = metadata.clone();
            bad["format"]["duration"] = duration.into();
            assert!(attributes(&bad).is_err());
        }
        let mut attached = metadata.clone();
        attached["streams"][0]["disposition"] = json!({"attached_pic":1});
        assert!(attributes(&attached).is_err());
    }
    #[tokio::test]
    #[ignore = "requires existing FFmpeg/FFprobe with libx264/libvpx; run explicitly for local media acceptance"]
    async fn real_tools_preserve_original_bytes_and_extract_visible_frames_on_unicode_paths() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("中文 空格");
        std::fs::create_dir(&directory).unwrap();
        let ffmpeg = tool("ffmpeg", root.path(), std::future::pending())
            .await
            .expect("existing FFmpeg required for this verification");
        for (extension, codec) in [
            ("mp4", "libx264"),
            ("mov", "libx264"),
            ("mkv", "libx264"),
            ("webm", "libvpx"),
        ] {
            let stage = directory.join(extension);
            std::fs::create_dir(&stage).unwrap();
            let source = stage.join(format!("视频.{extension}"));
            let mut args: Vec<OsString> = [
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x48:r=5:d=0.4",
                "-an",
                "-c:v",
                codec,
                "-pix_fmt",
                "yuv420p",
            ]
            .into_iter()
            .map(Into::into)
            .collect();
            args.push(source.as_os_str().into());
            run(
                &ffmpeg,
                &args,
                Duration::from_secs(20),
                std::future::pending(),
            )
            .await
            .unwrap();
            let hash = Sha256::digest(std::fs::read(&source).unwrap());
            let value = probe(&source, root.path(), std::future::pending())
                .await
                .unwrap();
            assert_eq!(value["width"], 64);
            assert_eq!(value["height"], 48);
            let cover = extract(&source, root.path(), std::future::pending())
                .await
                .unwrap();
            assert_eq!(cover["cover_filename"], "cover");
            assert!(crate::telegram_cover::valid(
                &std::fs::read(stage.join("cover")).unwrap()
            ));
            assert_eq!(hash, Sha256::digest(std::fs::read(&source).unwrap()));
            assert!(
                extract(&source, root.path(), std::future::pending())
                    .await
                    .is_err(),
                "never overwrite an existing cover"
            );
        }
    }
    #[test]
    #[ignore = "subprocess fixture invoked only by native process lifecycle tests"]
    fn blocking_child() {
        let marker = std::env::args()
            .find(|arg| arg.ends_with("child.pid"))
            .unwrap();
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }
    #[tokio::test]
    async fn cancellation_and_timeout_reap_native_process_before_returning() {
        for cancel_requested in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let marker = root.path().join("child.pid");
            let args: Vec<OsString> = vec![
                "--ignored".into(),
                "--exact".into(),
                "local_media::tests::blocking_child".into(),
                "--nocapture".into(),
                marker.as_os_str().into(),
            ];
            let cancel = async {
                if cancel_requested {
                    while !marker.exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } else {
                    std::future::pending::<()>().await;
                }
            };
            let result = run(
                &std::env::current_exe().unwrap(),
                &args,
                if cancel_requested {
                    Duration::from_secs(10)
                } else {
                    Duration::from_secs(2)
                },
                cancel,
            )
            .await;
            assert_eq!(
                result.unwrap_err().to_string(),
                if cancel_requested {
                    "CANCELED"
                } else {
                    "TIMEOUT"
                }
            );
            let pid: u32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::System::Threading::*;
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !process.is_null() {
                    let mut exit = 0;
                    assert_ne!(GetExitCodeProcess(process, &mut exit), 0);
                    windows_sys::Win32::Foundation::CloseHandle(process);
                    assert_ne!(exit, 259);
                }
            }
            #[cfg(unix)]
            assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0);
        }
    }
}
