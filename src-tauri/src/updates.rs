use reqwest::{Client, StatusCode};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const RELEASE_API: &str = "https://api.github.com/repos/lifei6671/video-downloader/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/lifei6671/video-downloader/releases/tag/";

#[derive(Clone, Debug, Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: String,
    pub release_tag: String,
    pub has_update: bool,
    pub release_notes: String,
    pub published_at: Option<String>,
    pub installer: Option<InstallerInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallerInfo {
    pub name: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub(crate) struct Release {
    pub tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    state: String,
    digest: Option<String>,
}

pub(crate) struct InstallerAsset {
    pub asset: Asset,
    pub sha256: String,
}

pub(crate) fn stable_version(tag: &str) -> Result<Version, String> {
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "UPDATE_INVALID_RESPONSE")?;
    if !version.pre.is_empty() {
        return Err("UPDATE_INVALID_RESPONSE".into());
    }
    Ok(version)
}

pub fn release_url(tag: &str) -> Result<String, String> {
    stable_version(tag).map_err(|_| "INVALID_INPUT")?;
    Ok(format!("{RELEASES_PAGE}{tag}"))
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "UPDATE_TIMEOUT".into()
    } else if error.is_decode() {
        "UPDATE_INVALID_RESPONSE".into()
    } else {
        "UPDATE_NETWORK_ERROR".into()
    }
}

pub(crate) async fn fetch_release_data(client: &Client, endpoint: &str) -> Result<Release, String> {
    let response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(network_error)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => return Err("UPDATE_NO_RELEASE".into()),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            return Err("UPDATE_RATE_LIMITED".into());
        }
        _ => return Err("UPDATE_SERVICE_ERROR".into()),
    }
    let release: Release = response.json().await.map_err(network_error)?;
    if release.draft || release.prerelease {
        return Err("UPDATE_INVALID_RESPONSE".into());
    }
    stable_version(&release.tag_name)?;
    Ok(release)
}

pub(crate) fn select_installer(
    release: &Release,
    os: &str,
    arch: &str,
) -> Result<InstallerAsset, String> {
    let version = stable_version(&release.tag_name)?;
    let suffix = match (os, arch) {
        ("windows", "x86_64") => "x64-setup.exe",
        ("macos", "x86_64") => "x64.dmg",
        ("macos", "aarch64") => "aarch64.dmg",
        _ => return Err("UPDATE_UNSUPPORTED_SYSTEM".into()),
    };
    let name = format!("Yingqu_{version}_{suffix}");
    let matches: Vec<_> = release
        .assets
        .iter()
        .filter(|asset| asset.name == name)
        .collect();
    if matches.len() != 1 {
        return Err("UPDATE_NO_INSTALLER".into());
    }
    let asset = matches[0];
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("UPDATE_NO_INSTALLER")?;
    let expected = format!(
        "https://github.com/lifei6671/video-downloader/releases/download/{}/{name}",
        release.tag_name
    );
    if asset.state != "uploaded"
        || asset.size == 0
        || asset.size > 1024 * 1024 * 1024
        || asset.browser_download_url != expected
    {
        return Err("UPDATE_NO_INSTALLER".into());
    }
    Ok(InstallerAsset {
        asset: asset.clone(),
        sha256: digest.to_ascii_lowercase(),
    })
}

async fn fetch_release(
    client: &Client,
    endpoint: &str,
    current: &str,
) -> Result<UpdateCheck, String> {
    let release = fetch_release_data(client, endpoint).await?;
    let installer = select_installer(&release, std::env::consts::OS, std::env::consts::ARCH)
        .ok()
        .map(|selected| InstallerInfo {
            name: selected.asset.name,
            size: selected.asset.size,
        });
    let latest = stable_version(&release.tag_name)?;
    let current = Version::parse(current).map_err(|_| "UPDATE_INVALID_VERSION")?;
    Ok(UpdateCheck {
        has_update: latest.cmp_precedence(&current).is_gt(),
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        release_tag: release.tag_name,
        release_notes: release.body.unwrap_or_default(),
        published_at: release.published_at,
        installer,
    })
}

pub(crate) async fn installer_for_tag(tag: &str, current: &str) -> Result<InstallerAsset, String> {
    let client = check_client(current)?;
    let endpoint =
        format!("https://api.github.com/repos/lifei6671/video-downloader/releases/tags/{tag}");
    let release = fetch_release_data(&client, &endpoint).await?;
    if release.tag_name != tag {
        return Err("UPDATE_INVALID_RESPONSE".into());
    }
    select_installer(&release, std::env::consts::OS, std::env::consts::ARCH)
}

fn check_client(current: &str) -> Result<Client, String> {
    Client::builder()
        .user_agent(format!("Yingqu/{current}"))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "UPDATE_NETWORK_ERROR".into())
}

pub async fn check(current: &str) -> Result<UpdateCheck, String> {
    let client = check_client(current)?;
    fetch_release(&client, RELEASE_API, current).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn server(
        status: u16,
        body: String,
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]).to_lowercase();
            assert!(request.contains("accept: application/vnd.github+json"));
            assert!(!request.contains("cookie:"));
            assert!(!request.contains("authorization:"));
            tokio::time::sleep(delay).await;
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
        });
        (format!("http://{address}/latest"), task)
    }

    fn release(tag: &str) -> serde_json::Value {
        json!({"tag_name": tag, "draft": false, "prerelease": false,
            "body": "## 更新说明\n- 修复下载问题", "published_at": "2026-10-18T00:00:00Z"})
    }

    #[test]
    fn installer_selection_matches_os_arch_version_and_verified_asset() {
        let mut value = release("v1.2.0");
        value["assets"] = json!(["x64-setup.exe", "x64.dmg", "aarch64.dmg"].map(|suffix| {
            let name = format!("Yingqu_1.2.0_{suffix}");
            json!({"name":name,"size":4096,"state":"uploaded","digest":format!("sha256:{}", "a".repeat(64)),
                "browser_download_url":format!("https://github.com/lifei6671/video-downloader/releases/download/v1.2.0/{name}")})
        }));
        for (os, arch, suffix) in [
            ("windows", "x86_64", "x64-setup.exe"),
            ("macos", "x86_64", "x64.dmg"),
            ("macos", "aarch64", "aarch64.dmg"),
        ] {
            let data: Release = serde_json::from_value(value.clone()).unwrap();
            let selected = select_installer(&data, os, arch).unwrap();
            assert_eq!(selected.asset.name, format!("Yingqu_1.2.0_{suffix}"));
            assert_eq!(selected.sha256, "a".repeat(64));
        }
        let data: Release = serde_json::from_value(value.clone()).unwrap();
        assert!(
            matches!(select_installer(&data, "windows", "aarch64"), Err(error) if error == "UPDATE_UNSUPPORTED_SYSTEM")
        );
        assert!(select_installer(&data, "linux", "x86_64").is_err());
        for (field, invalid) in [
            ("name", json!("Yingqu_0.9.0_x64-setup.exe")), ("size", json!(0)),
            ("size", json!(1024u64 * 1024 * 1024 + 1)), ("state", json!("new")),
            ("digest", json!(null)), ("digest", json!("sha256:bad")),
            ("browser_download_url", json!("https://example.com/Yingqu_1.2.0_x64-setup.exe")),
            ("browser_download_url", json!("https://github.com/other/repo/releases/download/v1.2.0/Yingqu_1.2.0_x64-setup.exe")),
        ] {
            let mut broken = value.clone(); broken["assets"][0][field] = invalid;
            let data: Release = serde_json::from_value(broken).unwrap();
            assert!(select_installer(&data, "windows", "x86_64").is_err(), "accepted invalid {field}");
        }
        let duplicate = value["assets"][0].clone();
        value["assets"].as_array_mut().unwrap().push(duplicate);
        let data: Release = serde_json::from_value(value).unwrap();
        assert!(select_installer(&data, "windows", "x86_64").is_err());
    }

    async fn lookup(status: u16, body: String, current: &str) -> Result<UpdateCheck, String> {
        let (url, task) = server(status, body, Duration::ZERO).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let result = fetch_release(&client, &url, current).await;
        task.await.unwrap();
        result
    }

    #[tokio::test]
    async fn compares_semver_and_preserves_notes() {
        for (tag, current, expected) in [
            ("v1.10.0", "1.9.0", true),
            ("v1.2.0", "1.2.0", false),
            ("v1.2.0", "2.0.0", false),
            ("1.2.0", "1.2.0-rc.1", true),
            ("v1.2.0+release", "1.2.0+local", false),
        ] {
            let result = lookup(200, release(tag).to_string(), current)
                .await
                .unwrap();
            assert_eq!(result.has_update, expected);
            assert_eq!(result.release_tag, tag);
            assert_eq!(result.release_notes, "## 更新说明\n- 修复下载问题");
            assert_eq!(result.current_version, current);
        }
    }

    #[tokio::test]
    async fn errors_never_become_latest() {
        for (status, expected) in [
            (404, "UPDATE_NO_RELEASE"),
            (403, "UPDATE_RATE_LIMITED"),
            (429, "UPDATE_RATE_LIMITED"),
            (500, "UPDATE_SERVICE_ERROR"),
            (302, "UPDATE_SERVICE_ERROR"),
        ] {
            assert_eq!(
                lookup(status, "{}".into(), "1.0.0").await.unwrap_err(),
                expected
            );
        }
        for body in [
            "broken json".into(),
            "{}".into(),
            release("not-a-version").to_string(),
            release("v2.0.0-beta.1").to_string(),
        ] {
            assert_eq!(
                lookup(200, body, "1.0.0").await.unwrap_err(),
                "UPDATE_INVALID_RESPONSE"
            );
        }
        for field in ["draft", "prerelease"] {
            let mut value = release("v2.0.0");
            value[field] = json!(true);
            assert_eq!(
                lookup(200, value.to_string(), "1.0.0").await.unwrap_err(),
                "UPDATE_INVALID_RESPONSE"
            );
        }
    }

    #[tokio::test]
    async fn missing_notes_are_reported_as_empty() {
        let mut value = release("v2.0.0");
        value["body"] = serde_json::Value::Null;
        value["published_at"] = serde_json::Value::Null;
        let result = lookup(200, value.to_string(), "1.0.0").await.unwrap();
        assert!(result.release_notes.is_empty());
        assert!(result.published_at.is_none());
    }

    #[tokio::test]
    async fn timeout_is_distinct_from_a_successful_check() {
        let (url, task) = server(200, release("v2.0.0").to_string(), Duration::from_secs(1)).await;
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        assert_eq!(
            fetch_release(&client, &url, "1.0.0").await.unwrap_err(),
            "UPDATE_TIMEOUT"
        );
        task.abort();
    }

    #[test]
    fn download_link_only_accepts_stable_release_tags() {
        assert_eq!(
            release_url("v1.2.3").unwrap(),
            format!("{RELEASES_PAGE}v1.2.3")
        );
        for tag in [
            "https://example.com",
            "v1.2.3/../../",
            "v1.2.3?next=evil",
            "v1.2.3#x",
            "v1.2.3-beta.1",
            "v01.2.3",
        ] {
            assert_eq!(release_url(tag).unwrap_err(), "INVALID_INPUT");
        }
    }

    #[tokio::test]
    async fn disconnected_server_is_a_network_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/latest", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        assert_eq!(
            fetch_release(&client, &endpoint, "1.0.0")
                .await
                .unwrap_err(),
            "UPDATE_NETWORK_ERROR"
        );
        task.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires live GitHub connectivity"]
    async fn github_live_check() {
        let result = check(env!("CARGO_PKG_VERSION")).await.unwrap();
        assert!(!result.release_tag.is_empty());
        assert!(release_url(&result.release_tag).is_ok());
        println!("{}", serde_json::to_string(&result).unwrap());
    }
}
