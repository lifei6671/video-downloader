use crate::updates::{self, InstallerAsset, UpdateCheck};
use reqwest::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
    task::JoinHandle,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Downloading,
    Verifying,
    Canceling,
    Completed,
    Canceled,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct DownloadStatus {
    pub id: String,
    pub revision: u64,
    pub release: UpdateCheck,
    pub phase: Phase,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub filename: Option<String>,
    pub error: Option<String>,
}

struct Installer {
    id: String,
    path: PathBuf,
    size: u64,
    sha256: String,
}
struct Downloaded {
    file: NamedTempFile,
    filename: String,
    size: u64,
    sha256: String,
}
#[derive(Default)]
struct Inner {
    closing: bool,
    checked: Option<UpdateCheck>,
    status: Option<DownloadStatus>,
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
    installer: Option<Installer>,
    revision: u64,
}

pub struct UpdateDownloads {
    directory: PathBuf,
    current: String,
    inner: Mutex<Inner>,
    emit: Box<dyn Fn(&DownloadStatus) + Send + Sync>,
}

impl UpdateDownloads {
    pub fn new(
        directory: PathBuf,
        current: String,
        emit: impl Fn(&DownloadStatus) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            directory,
            current,
            inner: Mutex::new(Inner::default()),
            emit: Box::new(emit),
        })
    }

    pub async fn check(&self) -> Result<UpdateCheck, String> {
        if self.inner.lock().unwrap().closing {
            return Err("UPDATE_DOWNLOAD_SHUTTING_DOWN".into());
        }
        let checked = updates::check(&self.current).await?;
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            return Err("UPDATE_DOWNLOAD_SHUTTING_DOWN".into());
        }
        inner.checked = Some(checked.clone());
        Ok(checked)
    }

    pub fn status(&self) -> Option<DownloadStatus> {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn start(self: &Arc<Self>, tag: &str) -> Result<DownloadStatus, String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            return Err("UPDATE_DOWNLOAD_SHUTTING_DOWN".into());
        }
        if inner.cancel.is_some() {
            return Err("UPDATE_DOWNLOAD_BUSY".into());
        }
        let checked = inner
            .checked
            .as_ref()
            .filter(|result| result.has_update && result.release_tag == tag)
            .ok_or("UPDATE_CHECK_REQUIRED")?
            .clone();
        if checked.installer.is_none() {
            return Err("UPDATE_NO_INSTALLER".into());
        }
        let (cancel, mut canceled) = watch::channel(false);
        inner.revision += 1;
        let status = DownloadStatus {
            id: uuid::Uuid::new_v4().to_string(),
            revision: inner.revision,
            filename: checked.installer.as_ref().map(|info| info.name.clone()),
            total_bytes: checked.installer.as_ref().map(|info| info.size),
            release: checked,
            phase: Phase::Preparing,
            downloaded_bytes: 0,
            error: None,
        };
        inner.status = Some(status.clone());
        inner.installer = None;
        inner.cancel = Some(cancel);
        let manager = self.clone();
        let operation = status.clone();
        inner.task = Some(tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = canceled.changed() => Err("UPDATE_DOWNLOAD_CANCELED".into()),
                result = manager.transfer(&operation) => result,
            };
            manager.finish(&operation.id, result);
        }));
        drop(inner);
        (self.emit)(&status);
        Ok(status)
    }

    pub fn cancel(&self, id: &str) -> Result<DownloadStatus, String> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.status.as_ref().is_some_and(|status| status.id == id) {
            return Err("UPDATE_DOWNLOAD_NOT_FOUND".into());
        }
        if let Some(sender) = &inner.cancel {
            sender.send_replace(true);
            inner.revision += 1;
            let revision = inner.revision;
            let status = inner.status.as_mut().unwrap();
            status.revision = revision;
            status.phase = Phase::Canceling;
        }
        let status = inner.status.clone().unwrap();
        drop(inner);
        (self.emit)(&status);
        Ok(status)
    }

    fn progress(&self, id: &str, phase: Phase, downloaded: u64, asset: &InstallerAsset) {
        let mut inner = self.inner.lock().unwrap();
        if !inner
            .status
            .as_ref()
            .is_some_and(|status| status.id == id && status.phase != Phase::Canceling)
        {
            return;
        }
        inner.revision += 1;
        let revision = inner.revision;
        let status = inner.status.as_mut().unwrap();
        status.revision = revision;
        status.phase = phase;
        status.downloaded_bytes = downloaded;
        status.total_bytes = Some(asset.asset.size);
        status.filename = Some(asset.asset.name.clone());
        let status = status.clone();
        drop(inner);
        (self.emit)(&status);
    }

    async fn transfer(&self, status: &DownloadStatus) -> Result<Downloaded, String> {
        let asset = updates::installer_for_tag(&status.release.release_tag, &self.current).await?;
        self.download_asset(&status.id, &asset, &download_client(&self.current)?)
            .await
    }

    async fn download_asset(
        &self,
        id: &str,
        selected: &InstallerAsset,
        client: &Client,
    ) -> Result<Downloaded, String> {
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|_| "IO_ERROR")?;
        let file = tempfile::Builder::new()
            .prefix(".update-")
            .suffix(".part")
            .tempfile_in(&self.directory)
            .map_err(|_| "IO_ERROR")?;
        let mut writer = tokio::fs::File::from_std(file.reopen().map_err(|_| "IO_ERROR")?);
        let mut response = client
            .get(&selected.asset.browser_download_url)
            .send()
            .await
            .map_err(download_error)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err("UPDATE_DOWNLOAD_FAILED".into());
        }
        if response
            .content_length()
            .is_some_and(|length| length != selected.asset.size)
        {
            return Err("UPDATE_SIZE_MISMATCH".into());
        }
        self.progress(id, Phase::Downloading, 0, selected);
        let mut downloaded = 0u64;
        let mut hash = Sha256::new();
        let mut last = Instant::now();
        while let Some(chunk) = response.chunk().await.map_err(download_error)? {
            downloaded += chunk.len() as u64;
            if downloaded > selected.asset.size {
                return Err("UPDATE_SIZE_MISMATCH".into());
            }
            writer.write_all(&chunk).await.map_err(|_| "IO_ERROR")?;
            hash.update(&chunk);
            if last.elapsed() >= Duration::from_millis(100) || downloaded == selected.asset.size {
                self.progress(id, Phase::Downloading, downloaded, selected);
                last = Instant::now();
            }
        }
        self.progress(id, Phase::Verifying, downloaded, selected);
        if downloaded != selected.asset.size {
            return Err("UPDATE_SIZE_MISMATCH".into());
        }
        if format!("{:x}", hash.finalize()) != selected.sha256 {
            return Err("UPDATE_CHECKSUM_MISMATCH".into());
        }
        writer.flush().await.map_err(|_| "IO_ERROR")?;
        writer.sync_all().await.map_err(|_| "IO_ERROR")?;
        drop(writer);
        Ok(Downloaded {
            file,
            filename: selected.asset.name.clone(),
            size: downloaded,
            sha256: selected.sha256.clone(),
        })
    }

    fn finish(&self, id: &str, mut result: Result<Downloaded, String>) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.status.as_ref().is_some_and(|status| status.id == id) {
            return;
        }
        if inner.cancel.as_ref().is_some_and(|sender| *sender.borrow()) {
            result = Err("UPDATE_DOWNLOAD_CANCELED".into());
        }
        let result = result.and_then(|download| {
            let path = self.directory.join(format!("{id}-{}", download.filename));
            download
                .file
                .persist_noclobber(&path)
                .map_err(|_| "IO_ERROR".to_string())?;
            Ok(Installer {
                id: id.to_owned(),
                path,
                size: download.size,
                sha256: download.sha256,
            })
        });
        inner.cancel = None;
        inner.revision += 1;
        let revision = inner.revision;
        let status = inner.status.as_mut().unwrap();
        status.revision = revision;
        match &result {
            Ok(installer) => {
                status.phase = Phase::Completed;
                status.downloaded_bytes = installer.size;
            }
            Err(error) if error == "UPDATE_DOWNLOAD_CANCELED" => {
                status.phase = Phase::Canceled;
            }
            Err(error) => {
                status.phase = Phase::Failed;
                status.error = Some(error.clone());
            }
        }
        inner.installer = result.ok();
        let status = inner.status.clone().unwrap();
        drop(inner);
        (self.emit)(&status);
    }

    pub async fn installer_path(&self, id: &str) -> Result<PathBuf, String> {
        let (path, size, sha256) = {
            let inner = self.inner.lock().unwrap();
            let installer = inner
                .installer
                .as_ref()
                .filter(|item| item.id == id)
                .ok_or("UPDATE_INSTALLER_NOT_READY")?;
            (
                installer.path.clone(),
                installer.size,
                installer.sha256.clone(),
            )
        };
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| "UPDATE_INSTALLER_MISSING")?;
        if !metadata.is_file() || metadata.len() != size {
            return Err("UPDATE_CHECKSUM_MISMATCH".into());
        }
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|_| "UPDATE_INSTALLER_MISSING")?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 65536];
        loop {
            let count = file.read(&mut buffer).await.map_err(|_| "IO_ERROR")?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        if format!("{:x}", hash.finalize()) != sha256 {
            return Err("UPDATE_CHECKSUM_MISMATCH".into());
        }
        Ok(path)
    }

    pub async fn shutdown(&self) {
        let task = {
            let mut inner = self.inner.lock().unwrap();
            inner.closing = true;
            if let Some(cancel) = &inner.cancel {
                cancel.send_replace(true);
            }
            inner.task.take()
        };
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

fn download_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "UPDATE_DOWNLOAD_TIMEOUT".into()
    } else {
        "UPDATE_DOWNLOAD_FAILED".into()
    }
}

fn allowed_redirect(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn download_client(current: &str) -> Result<Client, String> {
    Client::builder()
        .user_agent(format!("Yingqu/{current}"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !allowed_redirect(attempt.url()) {
                attempt.error("untrusted update redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "UPDATE_DOWNLOAD_FAILED".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::net::TcpListener;

    fn checked() -> UpdateCheck {
        UpdateCheck {
            current_version: "1.0.0".into(),
            latest_version: "1.2.0".into(),
            release_tag: "v1.2.0".into(),
            has_update: true,
            release_notes: "更新说明".into(),
            published_at: None,
            installer: Some(updates::InstallerInfo {
                name: "Yingqu_1.2.0_x64-setup.exe".into(),
                size: 4096,
            }),
        }
    }

    fn manager() -> (tempfile::TempDir, Arc<UpdateDownloads>) {
        let directory = tempfile::tempdir().unwrap();
        let manager = UpdateDownloads::new(directory.path().to_owned(), "1.0.0".into(), |_| {});
        manager.inner.lock().unwrap().checked = Some(checked());
        (directory, manager)
    }

    fn selected(url: &str, body: &[u8]) -> InstallerAsset {
        InstallerAsset { asset: serde_json::from_value(json!({"name":"Yingqu_1.2.0_x64-setup.exe","size":body.len(),"browser_download_url":url,"state":"uploaded","digest":null})).unwrap(), sha256: format!("{:x}", Sha256::digest(body)) }
    }

    fn preparing(manager: &UpdateDownloads) -> String {
        let mut inner = manager.inner.lock().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        inner.status = Some(DownloadStatus {
            id: id.clone(),
            revision: 1,
            release: checked(),
            phase: Phase::Preparing,
            downloaded_bytes: 0,
            total_bytes: None,
            filename: None,
            error: None,
        });
        inner.revision = 1;
        id
    }

    async fn server(
        body: Vec<u8>,
        status: u16,
        declared: usize,
        delay: Duration,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/installer", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let length = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..length]).to_lowercase();
            assert!(!request.contains("cookie:"));
            assert!(!request.contains("authorization:"));
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(&body).await;
        });
        (url, task)
    }

    #[tokio::test]
    async fn downloads_verifies_and_only_opens_known_completed_file() {
        let (dir, manager) = manager();
        let id = preparing(&manager);
        let body = vec![42; 4096];
        let (url, task) = server(body.clone(), 200, body.len(), Duration::ZERO).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let result = manager
            .download_asset(&id, &selected(&url, &body), &client)
            .await;
        assert_eq!(manager.status().unwrap().phase, Phase::Verifying);
        assert!(manager.installer_path(&id).await.is_err());
        manager.finish(&id, result);
        task.await.unwrap();
        let status = manager.status().unwrap();
        assert_eq!(status.phase, Phase::Completed);
        assert_eq!(status.downloaded_bytes, 4096);
        let path = manager.installer_path(&id).await.unwrap();
        assert_eq!(manager.cancel(&id).unwrap().phase, Phase::Completed);
        assert!(path.starts_with(dir.path()));
        assert_eq!(tokio::fs::read(&path).await.unwrap(), body);
        assert!(manager.installer_path("../../arbitrary.exe").await.is_err());
        tokio::fs::write(&path, vec![43; 4096]).await.unwrap();
        assert_eq!(
            manager.installer_path(&id).await.unwrap_err(),
            "UPDATE_CHECKSUM_MISMATCH"
        );
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(
            manager.installer_path(&id).await.unwrap_err(),
            "UPDATE_INSTALLER_MISSING"
        );
    }

    #[test]
    fn installer_path_future_fits_desktop_command_stack() {
        let (_dir, manager) = manager();
        let future = manager.installer_path("test");
        let bytes = std::mem::size_of_val(&future);
        // Tauri moves this future through command wrappers on the Windows main
        // thread's 1 MiB stack before spawning it; keep the read buffer off-stack.
        assert!(bytes < 4096, "installer verification future is {bytes} bytes");
    }

    #[tokio::test]
    async fn errors_remove_part_files_and_never_enable_open() {
        for case in ["checksum", "size", "http", "timeout"] {
            let (dir, manager) = manager();
            let id = preparing(&manager);
            let body = vec![42; 4096];
            let (url, task) = server(
                body.clone(),
                if case == "http" { 500 } else { 200 },
                if case == "size" { 4097 } else { 4096 },
                if case == "timeout" {
                    Duration::from_secs(1)
                } else {
                    Duration::ZERO
                },
            )
            .await;
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(300))
                .build()
                .unwrap();
            let mut asset = selected(&url, &body);
            if case == "checksum" {
                asset.sha256 = "0".repeat(64);
            }
            let result = manager.download_asset(&id, &asset, &client).await;
            assert!(result.is_err(), "{case}");
            manager.finish(&id, result);
            assert_eq!(manager.status().unwrap().phase, Phase::Failed);
            assert!(manager.installer_path(&id).await.is_err());
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                0,
                "part left after {case}"
            );
            task.abort();
        }
    }

    #[tokio::test]
    async fn cancel_reserved_download_prevents_network_and_allows_retry() {
        let (dir, manager) = manager();
        assert!(matches!(manager.start("v9.0.0"), Err(error) if error=="UPDATE_CHECK_REQUIRED"));
        let first = manager.start("v1.2.0").unwrap();
        assert!(matches!(manager.start("v1.2.0"),Err(error) if error=="UPDATE_DOWNLOAD_BUSY"));
        assert!(manager.cancel("wrong-id").is_err());
        assert_eq!(manager.cancel(&first.id).unwrap().phase, Phase::Canceling);
        let canceled_task = manager.inner.lock().unwrap().task.take().unwrap();
        canceled_task.await.unwrap();
        assert_eq!(manager.status().unwrap().phase, Phase::Canceled);
        assert!(manager.installer_path(&first.id).await.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let next = manager.start("v1.2.0").unwrap();
        assert_ne!(next.id, first.id);
        assert!(next.revision > first.revision);
        manager.cancel(&next.id).unwrap();
        manager.shutdown().await;
        assert!(
            matches!(manager.start("v1.2.0"),Err(error) if error=="UPDATE_DOWNLOAD_SHUTTING_DOWN")
        );
        assert!(
            matches!(manager.check().await,Err(error) if error=="UPDATE_DOWNLOAD_SHUTTING_DOWN")
        );
    }

    #[tokio::test]
    async fn cancellation_before_commit_removes_already_verified_temporary_package() {
        let (dir, manager) = manager();
        let id = preparing(&manager);
        let body = vec![42; 4096];
        let (sender, _receiver) = watch::channel(false);
        manager.inner.lock().unwrap().cancel = Some(sender);
        let (url, task) = server(body.clone(), 200, body.len(), Duration::ZERO).await;
        let result = manager
            .download_asset(
                &id,
                &selected(&url, &body),
                &Client::builder().no_proxy().build().unwrap(),
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(manager.status().unwrap().phase, Phase::Verifying);
        manager.cancel(&id).unwrap();
        manager.finish(&id, result);
        task.await.unwrap();
        assert_eq!(manager.status().unwrap().phase, Phase::Canceled);
        assert!(manager.installer_path(&id).await.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn dropping_inflight_transfer_disconnects_and_removes_partial_file() {
        let (dir, manager) = manager();
        let id = preparing(&manager);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/slow", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            socket.write_all(&vec![42; 1024]).await.unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), socket.read(&mut request))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        let owner = manager.clone();
        let asset = selected(&url, &vec![42; 4096]);
        let task = tokio::spawn(async move {
            owner
                .download_asset(&id, &asset, &Client::builder().no_proxy().build().unwrap())
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.status().unwrap().downloaded_bytes == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        server.await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn redirects_remain_https_on_github_release_hosts() {
        for value in [
            "https://github.com/release",
            "https://release-assets.githubusercontent.com/signed?token=public",
            "https://objects.githubusercontent.com/file",
        ] {
            assert!(allowed_redirect(&url::Url::parse(value).unwrap()));
        }
        for value in [
            "http://github.com/file",
            "https://example.com/file",
            "https://github.com.evil.test/file",
            "https://user@github.com/file",
            "https://github.com:8443/file",
            "file:///C:/arbitrary.exe",
        ] {
            assert!(!allowed_redirect(&url::Url::parse(value).unwrap()));
        }
    }

    #[tokio::test]
    #[ignore = "requires live GitHub connectivity and downloads an installer without executing it"]
    async fn github_live_installer_download() {
        let (_directory, manager) = manager();
        let checked = manager.check().await.unwrap();
        assert!(checked.installer.is_some());
        let status = manager.start(&checked.release_tag).unwrap();
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let status = manager.status().unwrap();
                if matches!(
                    status.phase,
                    Phase::Completed | Phase::Canceled | Phase::Failed
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        let final_status = manager.status().unwrap();
        assert_eq!(final_status.phase, Phase::Completed, "{final_status:?}");
        assert!(manager.installer_path(&status.id).await.unwrap().is_file());
        manager.shutdown().await;
    }
}
