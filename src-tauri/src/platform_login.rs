use std::time::Duration;

use tauri::{
    webview::{Cookie, NewWindowResponse},
    Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::watch;

use super::{error, AppCore};
use yingqu_core::browser_identity::BrowserIdentity;
use yingqu_core::settings::{AccountAuth, Settings};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const WINDOW_LABEL: &str = "platform-login";

#[derive(Default)]
pub(super) struct Login(tokio::sync::Mutex<()>);

struct Site {
    title: &'static str,
    login_url: &'static str,
    cookie_url: &'static str,
    cookie_domains: &'static [&'static str],
    auth_cookies: &'static [&'static str],
}

fn site(platform: &str) -> Option<Site> {
    Some(match platform {
        "bilibili" => Site {
            title: "登录 B 站",
            login_url: "https://passport.bilibili.com/login",
            cookie_url: "https://www.bilibili.com/",
            cookie_domains: &["www.bilibili.com", "bilibili.com"],
            auth_cookies: &["SESSDATA"],
        },
        "weixin" => Site {
            title: "登录腾讯元宝",
            login_url: "https://yuanbao.tencent.com/open/login",
            cookie_url: "https://yuanbao.tencent.com/api/weixin/get_parse_result",
            cookie_domains: &["yuanbao.tencent.com", "tencent.com"],
            auth_cookies: &["hy_user", "hy_token", "hy_source"],
        },
        "douyin" => Site {
            title: "登录抖音",
            login_url: "https://www.douyin.com/?showLogin=1",
            cookie_url: "https://www.douyin.com/",
            cookie_domains: &["www.douyin.com", "douyin.com"],
            auth_cookies: &["sessionid", "sessionid_ss"],
        },
        "xiaohongshu" => Site {
            title: "登录小红书",
            login_url: "https://www.xiaohongshu.com/explore",
            cookie_url: "https://www.xiaohongshu.com/",
            cookie_domains: &["www.xiaohongshu.com", "xiaohongshu.com"],
            // Xiaohongshu also issues web_session to anonymous visitors.
            auth_cookies: &["web_session", "id_token"],
        },
        "tiktok" => Site {
            title: "登录 TikTok",
            login_url: "https://www.tiktok.com/login/qrcode",
            cookie_url: "https://www.tiktok.com/",
            cookie_domains: &["www.tiktok.com", "tiktok.com"],
            auth_cookies: &["sessionid", "sessionid_ss"],
        },
        _ => return None,
    })
}

fn https_host(url: &tauri::Url) -> Option<&str> {
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none())
    .then(|| url.host_str())
    .flatten()
}

fn login_navigation(site: &Site, url: &tauri::Url) -> bool {
    https_host(url).is_some_and(|host| {
        site.cookie_domains.contains(&host)
            || match site.cookie_domains[0] {
                "yuanbao.tencent.com" => host == "open.weixin.qq.com",
                "www.douyin.com" => host == "sso.douyin.com",
                "www.xiaohongshu.com" => host == "passport.xiaohongshu.com",
                "www.bilibili.com" => host == "passport.bilibili.com",
                _ => false,
            }
    })
}

fn cookie_path_matches(request: &str, path: &str) -> bool {
    path.starts_with('/')
        && (request == path
            || request
                .strip_prefix(path)
                .is_some_and(|rest| path.ends_with('/') || rest.starts_with('/')))
}

fn session_header(
    site: &Site,
    cookies: &[Cookie<'_>],
    url: &tauri::Url,
    now: i64,
) -> Option<String> {
    let mut applicable = cookies
        .iter()
        .filter(|cookie| {
            cookie
                .domain()
                .is_some_and(|domain| site.cookie_domains.contains(&domain))
                && cookie_path_matches(url.path(), cookie.path().unwrap_or_default())
                && cookie
                    .expires_datetime()
                    .is_none_or(|time| time.unix_timestamp() > now)
                && !cookie.name().is_empty()
                && cookie
                    .name()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b))
                && !cookie.value().is_empty()
                && cookie.value().bytes().all(
                    |b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e),
                )
        })
        .collect::<Vec<_>>();
    applicable.sort_by_key(|cookie| {
        std::cmp::Reverse((
            cookie.path().unwrap_or_default().len(),
            cookie.domain() == url.host_str(),
        ))
    });
    let is_yuanbao = url.host_str() == Some("yuanbao.tencent.com");
    let has_cookie = |name: &&str| applicable.iter().any(|cookie| cookie.name() == *name);
    let logged_in = if is_yuanbao || url.host_str() == Some("www.xiaohongshu.com") {
        site.auth_cookies.iter().all(has_cookie)
    } else {
        site.auth_cookies.iter().any(has_cookie)
    };
    if !logged_in {
        return None;
    }
    let mut names = std::collections::HashSet::new();
    Some(
        applicable
            .into_iter()
            .filter(|cookie| !is_yuanbao || site.auth_cookies.contains(&cookie.name()))
            .filter(|cookie| names.insert(cookie.name()))
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

async fn wait_for_session(
    window: &WebviewWindow,
    site: std::sync::Arc<Site>,
    mut closed: watch::Receiver<bool>,
) -> Result<Option<(String, BrowserIdentity)>, &'static str> {
    let url: tauri::Url = site
        .cookie_url
        .parse()
        .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let deadline = tokio::time::sleep(LOGIN_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return Ok(None),
            _ = &mut deadline => return Err("PLATFORM_LOGIN_TIMEOUT"),
            _ = tick.tick() => {}
        }
        let cookie_window = window.clone();
        let url = url.clone();
        let cookie_site = site.clone();
        // WebView2 dispatches Cookie reads back to the UI thread.
        let read = tokio::task::spawn_blocking(move || {
            #[cfg(target_os = "macos")]
            // WKWebView's URL lookup omits parent-domain cookies.
            let cookies = cookie_window.cookies();
            #[cfg(not(target_os = "macos"))]
            let cookies = cookie_window.cookies_for_url(url.clone());
            let cookies = cookies.map_err(|_| "PLATFORM_COOKIE_READ_FAILED")?;
            Ok::<_, &'static str>(session_header(
                &cookie_site,
                &cookies,
                &url,
                chrono::Utc::now().timestamp(),
            ))
        });
        let header = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return Ok(None),
            _ = &mut deadline => return Err("PLATFORM_LOGIN_TIMEOUT"),
            result = read => result.map_err(|_| "PLATFORM_COOKIE_READ_FAILED")??,
        };
        if let Some(header) = header {
            let identity = tokio::select! {
                biased;
                _ = closed.wait_for(|closed| *closed) => return Ok(None),
                _ = &mut deadline => return Err("PLATFORM_LOGIN_TIMEOUT"),
                identity = browser_identity(window.clone(), &site) => identity?,
            };
            if let Some(identity) = identity {
                return Ok(Some((header, identity)));
            }
        }
    }
}

async fn browser_identity(
    window: WebviewWindow,
    site: &Site,
) -> Result<Option<BrowserIdentity>, &'static str> {
    #[derive(serde::Deserialize)]
    struct Reading {
        url: String,
        identity: Option<BrowserIdentity>,
    }
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    window
        .eval_with_callback(
            "({url:location.href,identity:window.__yingquBrowserIdentity||null})",
            move |json| {
                let _ = sender.try_send(json);
            },
        )
        .map_err(|_| "PLATFORM_LOGIN_DEVICE_FAILED")?;
    let json = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .map_err(|_| "PLATFORM_LOGIN_DEVICE_FAILED")?
        .ok_or("PLATFORM_LOGIN_DEVICE_FAILED")?;
    let reading: Reading =
        serde_json::from_str(&json).map_err(|_| "PLATFORM_LOGIN_DEVICE_FAILED")?;
    let url = reading
        .url
        .parse()
        .map_err(|_| "PLATFORM_LOGIN_DEVICE_FAILED")?;
    if !login_navigation(site, &url) {
        return Ok(None);
    }
    if let Some(identity) = &reading.identity {
        identity
            .validate()
            .map_err(|_| "PLATFORM_LOGIN_DEVICE_FAILED")?;
    }
    Ok(reading.identity)
}

fn build_login_window(
    main: &WebviewWindow,
    platform: &str,
    site: std::sync::Arc<Site>,
    clear: bool,
) -> Result<WebviewWindow, String> {
    let app = main.app_handle();
    let url = if clear { "about:blank" } else { site.login_url }
        .parse()
        .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?;
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?
        .join("platform-browser")
        .join(platform);
    std::fs::create_dir_all(&directory).map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?;
    let opener = app.clone();
    let navigation_site = site.clone();
    let builder = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::External(url))
        .title(site.title)
        .inner_size(
            if platform == "weixin" { 440.0 } else { 980.0 },
            if platform == "weixin" { 560.0 } else { 700.0 },
        )
        .resizable(platform != "weixin")
        .visible(!clear)
        .skip_taskbar(true)
        .devtools(false)
        .incognito(false)
        .data_directory(directory)
        .initialization_script(include_str!("browser-identity.js"))
        .parent(main)
        .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?
        .center()
        .on_navigation(move |url| {
            clear && url.as_str() == "about:blank" || login_navigation(&navigation_site, url)
        })
        .on_new_window(move |url, _| {
            if matches!(
                https_host(&url),
                Some("rule.tencent.com" | "privacy.qq.com")
            ) {
                let _ = opener.opener().open_url(url.as_str(), None::<&str>);
            }
            NewWindowResponse::Deny
        });
    #[cfg(target_os = "macos")]
    let builder = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(format!("me.disign.yingqu.platform-browser.{platform}"));
        let mut identifier = [0; 16];
        identifier.copy_from_slice(&hash[..16]);
        // Wry uses this isolated store on macOS14+, and the native origin-scoped
        // persistent store on13.3. Never clear the shared store globally.
        builder.data_store_identifier(identifier)
    };
    builder
        .build()
        .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED".into())
}

fn should_clear_cookie(site: &Site, cookie: &Cookie<'_>, auth_only: bool) -> bool {
    let domain = cookie.domain().unwrap_or_default().trim_start_matches('.');
    site.cookie_domains
        .iter()
        .any(|root| domain == *root || domain.ends_with(&format!(".{root}")))
        && (!auth_only || site.auth_cookies.contains(&cookie.name()))
}

async fn remove_site_cookies(
    window: &WebviewWindow,
    site: std::sync::Arc<Site>,
    auth_only: bool,
) -> Result<(), &'static str> {
    let reader = window.clone();
    tokio::task::spawn_blocking(move || {
        let cookies = reader
            .cookies()
            .map_err(|_| "PLATFORM_COOKIE_READ_FAILED")?;
        for cookie in cookies {
            if should_clear_cookie(&site, &cookie, auth_only) {
                reader
                    .delete_cookie(cookie)
                    .map_err(|_| "PLATFORM_COOKIE_READ_FAILED")?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "PLATFORM_COOKIE_READ_FAILED")?
}

async fn clear_site_cookies(
    main: &WebviewWindow,
    platform: &str,
    site: std::sync::Arc<Site>,
) -> Result<(), String> {
    let window = build_login_window(main, platform, site.clone(), true)?;
    let result = remove_site_cookies(&window, site, false).await;
    let _ = window.destroy();
    result.map_err(str::to_owned)
}

#[tauri::command]
pub(super) async fn login_platform(
    main: WebviewWindow,
    core: tauri::State<'_, AppCore>,
    login: tauri::State<'_, Login>,
    platform: String,
) -> Result<Option<Settings>, String> {
    if main.label() != "main" {
        return Err("ACCESS_DENIED".into());
    }
    let _guard = login.0.try_lock().map_err(|_| "PLATFORM_LOGIN_BUSY")?;
    let site = std::sync::Arc::new(site(&platform).ok_or("CONFIG_INVALID")?);
    let app = main.app_handle();
    let (closed, closed_rx) = watch::channel(false);
    let window = build_login_window(&main, &platform, site.clone(), true)?;
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
        ) {
            closed.send_replace(true);
        }
    });
    let result = async {
        // Force a fresh authentication result while retaining device cookies and
        // LocalStorage. A stale persisted session must not close the login page.
        remove_site_cookies(&window, site.clone(), true).await?;
        window
            .navigate(
                site.login_url
                    .parse()
                    .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?,
            )
            .map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?;
        window.show().map_err(|_| "PLATFORM_LOGIN_WINDOW_FAILED")?;
        let _ = window.set_focus();
        wait_for_session(&window, site, closed_rx.clone()).await
    }
    .await;
    let outcome = match result {
        Ok(Some((cookie, identity))) if !*closed_rx.borrow() => core
            .save_login_session(&platform, cookie, identity)
            .map(Some)
            .map_err(error),
        Ok(_) => Ok(None),
        Err(code) => Err(code.to_owned()),
    };
    if app.get_webview_window(WINDOW_LABEL).is_some() {
        let _ = window.destroy();
    }
    let _ = main.set_focus();
    outcome
}

#[tauri::command]
pub(super) async fn set_platform_auth(
    main: WebviewWindow,
    core: tauri::State<'_, AppCore>,
    login: tauri::State<'_, Login>,
    platform: String,
    mode: AccountAuth,
    cookie: Option<String>,
) -> Result<Settings, String> {
    if main.label() != "main" {
        return Err("ACCESS_DENIED".into());
    }
    if mode == AccountAuth::Login
        && (site(&platform).is_none() || cookie.as_deref().is_some_and(|value| !value.is_empty()))
    {
        return Err("CONFIG_INVALID".into());
    }
    let _guard = login.0.try_lock().map_err(|_| "PLATFORM_LOGIN_BUSY")?;
    if mode == AccountAuth::Login && cookie.as_deref() == Some("") {
        let site = std::sync::Arc::new(site(&platform).ok_or("CONFIG_INVALID")?);
        clear_site_cookies(&main, &platform, site).await?;
    }
    core.update_account_auth(&platform, mode, cookie)
        .map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relogin_removes_authentication_but_preserves_device_cookies_and_other_sites() {
        let bili = site("bilibili").unwrap();
        let cookie = |name, domain| {
            Cookie::build((name, "fixture"))
                .domain(domain)
                .path("/")
                .build()
        };
        assert!(should_clear_cookie(
            &bili,
            &cookie("SESSDATA", ".bilibili.com"),
            true
        ));
        assert!(!should_clear_cookie(
            &bili,
            &cookie("buvid3", ".bilibili.com"),
            true
        ));
        assert!(!should_clear_cookie(
            &bili,
            &cookie("SESSDATA", "bilibili.com.evil.test"),
            false
        ));
        assert!(!should_clear_cookie(
            &bili,
            &cookie("sessionid", ".douyin.com"),
            false
        ));
        assert!(should_clear_cookie(
            &bili,
            &cookie("buvid3", ".bilibili.com"),
            false
        ));
        assert!(should_clear_cookie(
            &bili,
            &cookie("SESSDATA", "passport.bilibili.com"),
            true
        ));
    }

    #[test]
    fn platform_sessions_require_login_markers_and_keep_only_same_site_cookies() {
        for (platform, marker) in [
            ("douyin", "sessionid"),
            ("xiaohongshu", "web_session"),
            ("tiktok", "sessionid_ss"),
            ("bilibili", "SESSDATA"),
        ] {
            let site = site(platform).unwrap();
            let url = site.cookie_url.parse().unwrap();
            let host = site.cookie_domains[0];
            let mut cookies = vec![
                Cookie::parse(format!(
                    "{marker}=foreign; Domain=accounts.google.com; Path=/"
                ))
                .unwrap(),
                Cookie::parse(format!("{marker}=other-path; Domain={host}; Path=/login")).unwrap(),
                Cookie::parse(format!("preference=fixture; Domain={host}; Path=/")).unwrap(),
            ];
            assert!(
                session_header(&site, &cookies, &url, 2).is_none(),
                "{platform}"
            );
            cookies.push(
                Cookie::parse(format!(
                    "{marker}=valid; Domain={host}; Path=/; Secure; HttpOnly"
                ))
                .unwrap(),
            );
            if platform == "xiaohongshu" {
                assert!(session_header(&site, &cookies, &url, 2).is_none());
                cookies.push(
                    Cookie::parse(format!(
                        "id_token=fixture; Domain={host}; Path=/; Secure; HttpOnly"
                    ))
                    .unwrap(),
                );
            }
            let header = session_header(&site, &cookies, &url, 2).unwrap();
            assert!(header.contains(&format!("{marker}=valid")), "{platform}");
            assert!(
                !header.contains("foreign") && !header.contains("other-path"),
                "{platform}"
            );
        }
    }

    #[test]
    fn yuanbao_requires_all_three_credentials_and_drops_other_cookies() {
        let site = site("weixin").unwrap();
        let url = site.cookie_url.parse().unwrap();
        let mut cookies = vec![
            Cookie::parse("hy_user=user; Domain=.tencent.com; Path=/").unwrap(),
            Cookie::parse("hy_source=web; Domain=.tencent.com; Path=/").unwrap(),
            Cookie::parse("hy_token=expired; Domain=yuanbao.tencent.com; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT").unwrap(),
            Cookie::parse("analytics=secret; Domain=yuanbao.tencent.com; Path=/").unwrap(),
        ];
        assert!(session_header(&site, &cookies, &url, 2).is_none());
        cookies.push(
            Cookie::parse("hy_token=token; Domain=.tencent.com; Path=/; Secure; HttpOnly").unwrap(),
        );
        let header = session_header(&site, &cookies, &url, 2).unwrap();
        assert_eq!(header.split("; ").count(), 3);
        assert!(header.contains("hy_token=token"));
    }

    #[test]
    fn login_windows_only_admit_official_https_origins() {
        for platform in ["weixin", "douyin", "xiaohongshu", "tiktok", "bilibili"] {
            let site = site(platform).unwrap();
            assert!(login_navigation(&site, &site.login_url.parse().unwrap()));
            for url in [
                "https://accounts.google.com/",
                "https://www.tiktok.com.evil.test/",
                "http://www.tiktok.com/",
                "https://user@www.tiktok.com/",
                "https://www.tiktok.com:8443/",
            ] {
                assert!(
                    !login_navigation(&site, &url.parse().unwrap()),
                    "{platform}: {url}"
                );
            }
        }
        assert!(site("youtube").is_none());
    }

    #[test]
    fn bilibili_login_collects_parent_session_and_excludes_passport_only_or_expired_cookies() {
        let site = site("bilibili").unwrap();
        let url = site.cookie_url.parse().unwrap();
        let mut cookies = vec![
            Cookie::parse("SESSDATA=passport-only; Domain=passport.bilibili.com; Path=/; Secure; HttpOnly").unwrap(),
            Cookie::parse("SESSDATA=expired; Domain=.bilibili.com; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT").unwrap(),
        ];
        assert!(session_header(&site, &cookies, &url, 2).is_none());
        cookies.push(
            Cookie::parse(
                "SESSDATA=fixture-session; Domain=.bilibili.com; Path=/; Secure; HttpOnly",
            )
            .unwrap(),
        );
        cookies.push(Cookie::parse("DedeUserID=123; Domain=.bilibili.com; Path=/").unwrap());
        let header = session_header(&site, &cookies, &url, 2).unwrap();
        assert!(header.contains("SESSDATA=fixture-session") && header.contains("DedeUserID=123"));
        assert!(!header.contains("passport-only") && !header.contains("expired"));
        assert!(login_navigation(
            &site,
            &"https://passport.bilibili.com/login".parse().unwrap()
        ));
    }
}
