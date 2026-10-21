use super::*;
use crate::{
    desktop_core::{Core, Status},
    files::{finalize_to, Video},
};
use std::sync::{Arc, Mutex as StdMutex};

pub(super) fn fixture(options: Value) -> (tempfile::TempDir, TdlSettings) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tdl_mock.py");
    std::fs::write(&path, include_str!("../../tests/fixtures/tdl_mock.py")).unwrap();
    std::fs::write(root.path().join("options.json"), options.to_string()).unwrap();
    (
        root,
        TdlSettings {
            enabled: true,
            path: Some(path),
            session_directory: None,
        },
    )
}
fn events(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
async fn operation(
    config: &TdlSettings,
    input: &str,
    expected: Option<&Value>,
    directory: Option<&Path>,
    progress: impl FnMut(Value),
    cancel: watch::Receiver<bool>,
) -> Result<Value> {
    Session::default()
        .operation(config, input, expected, directory, progress, cancel, None)
        .await
}
async fn resolve(config: &TdlSettings, input: &str) -> Value {
    let (_send, cancel) = watch::channel(false);
    operation(config, input, None, None, |_| {}, cancel)
        .await
        .unwrap()
}
async fn download(config: &TdlSettings, video: &Value, path: &Path) -> Result<Value> {
    let (_send, cancel) = watch::channel(false);
    operation(
        config,
        video["canonical_url"].as_str().unwrap(),
        Some(video),
        Some(path),
        |_| {},
        cancel,
    )
    .await
}
fn port_closed(port: u64) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(100),
    )
    .is_err()
}

fn login_core(root: &Path) -> Arc<Core> {
    Core::open(
        &root.join("login.sqlite"),
        root.join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}

pub(super) async fn wait_login_event(root: &Path) -> Value {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(event) = events(root)
                .into_iter()
                .find(|event| event["kind"] == "login")
            {
                return event;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

async fn login_phase(app: &Core, phase: &str) -> LoginStatus {
    timeout(Duration::from_secs(12), async {
        loop {
            if let Some(state) = app.tdl_login_status() {
                if state.phase == phase {
                    return state;
                }
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn hidden_pty_renders_qr_accepts_only_requested_2fa_and_keeps_password_out_of_events() {
    for accepted in [true, false] {
        let (root, config) = fixture(json!({"login_ui":true}));
        let app = login_core(root.path());
        app.save_tdl(config.clone()).unwrap();
        let core = app.clone();
        let worker = tokio::spawn(async move { core.login_tdl(config.path).await });
        let qr = login_phase(&app, "qr").await;
        assert_eq!(qr.qr.as_ref().unwrap().len(), 29);
        let prompt = login_phase(&app, "password").await;
        assert!(prompt.qr.is_none());
        assert!(app
            .submit_tdl_password(
                "stale",
                zeroize::Zeroizing::new("test-only-password".into())
            )
            .is_err());
        app.submit_tdl_password(
            &prompt.id,
            zeroize::Zeroizing::new(
                if accepted {
                    "test-only-password"
                } else {
                    "wrong-password"
                }
                .into(),
            ),
        )
        .unwrap();
        let result = timeout(Duration::from_secs(10), worker)
            .await
            .unwrap()
            .unwrap();
        if accepted {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.err().unwrap().to_string(), "TDL_2FA_INVALID");
        }
        assert!(app.tdl_login_status().is_none());
        let log = std::fs::read_to_string(root.path().join("events.jsonl")).unwrap();
        assert!(!log.contains("test-only-password") && !log.contains("wrong-password"));
        assert!(events(root.path())
            .iter()
            .any(|e| e["kind"] == "password" && e["accepted"] == accepted));
        app.shutdown().await;
    }
}

#[tokio::test]
async fn login_uses_new_storage_preserves_previous_sessions_and_survives_restart() {
    let (root, config) = fixture(json!({"version":"9.1.0"}));
    let app = login_core(root.path());
    app.save_tdl(config.clone()).unwrap();
    let first = app.login_tdl(config.path.clone()).await.unwrap();
    let original = first.tdl.session_directory.clone().unwrap();
    assert_eq!(
        std::fs::read(original.join("mock-session")).unwrap(),
        b"mock-authentication"
    );
    assert_eq!(
        events(root.path())
            .iter()
            .find(|e| e["kind"] == "login")
            .unwrap()["mode"],
        "qr"
    );
    // The frontend cannot replace the selected storage through save settings.
    let mut changed = config.clone();
    changed.session_directory = Some(root.path().join("untrusted"));
    assert_eq!(
        app.save_tdl(changed).unwrap().tdl.session_directory,
        Some(original.clone())
    );
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    assert!(events(root.path()).iter().any(|e| e["kind"] == "session"
        && e["storage"]
            .as_str()
            .is_some_and(|s| s.contains(&*original.to_string_lossy()))
        && e["namespace"] == "yingqu"));
    assert_eq!(
        app.login_tdl(config.path.clone())
            .await
            .err()
            .unwrap()
            .to_string(),
        "TDL_BUSY"
    );
    app.cancel(&task.id).await.unwrap();
    let second = app
        .login_tdl(config.path.clone())
        .await
        .unwrap()
        .tdl
        .session_directory
        .unwrap();
    assert_ne!(original, second);
    assert!(original.join("mock-session").exists());
    std::fs::write(root.path().join("options.json"), r#"{"login_exit":1}"#).unwrap();
    assert_eq!(
        app.login_tdl(config.path.clone())
            .await
            .err()
            .unwrap()
            .to_string(),
        "TDL_LOGIN_FAILED"
    );
    assert_eq!(app.settings().tdl.session_directory, Some(second.clone()));
    assert_eq!(
        std::fs::read_dir(root.path().join("runtime/tdl-sessions"))
            .unwrap()
            .count(),
        2
    );
    app.shutdown().await;
    drop(app);
    let app = login_core(root.path());
    assert_eq!(app.settings().tdl.session_directory, Some(second));
    assert!(app.settings().tdl.enabled);
    app.shutdown().await;
}

#[tokio::test]
async fn login_cancel_and_shutdown_reap_process_and_remove_only_attempt_storage() {
    for shutdown in [false, true] {
        let (root, config) = fixture(json!({"login_delay":120,"login_child":true}));
        let app = login_core(root.path());
        app.save_tdl(config.clone()).unwrap();
        let core = app.clone();
        let path = config.path.clone();
        let worker = tokio::spawn(async move { core.login_tdl(path).await });
        let _event = wait_login_event(root.path()).await;
        assert!(app.tdl_login_running());
        assert_eq!(
            app.resolve("https://t.me/c/100/9".into())
                .err()
                .unwrap()
                .to_string(),
            "TDL_BUSY"
        );
        assert_eq!(
            app.save_tdl(config.clone()).err().unwrap().to_string(),
            "TDL_BUSY"
        );
        assert_eq!(
            app.login_tdl(config.path.clone())
                .await
                .err()
                .unwrap()
                .to_string(),
            "TDL_BUSY"
        );
        if shutdown {
            app.shutdown().await;
        } else {
            app.cancel_tdl_login().unwrap();
        }
        assert_eq!(
            timeout(Duration::from_secs(8), worker)
                .await
                .unwrap()
                .unwrap()
                .err()
                .unwrap()
                .to_string(),
            "CANCELED"
        );
        assert!(!app.tdl_login_running());
        assert!(app.settings().tdl.session_directory.is_none());
        assert_eq!(
            std::fs::read_dir(root.path().join("runtime/tdl-sessions"))
                .unwrap()
                .count(),
            0
        );
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            for pid in [
                _event["pid"].as_u64().unwrap(),
                _event["child_pid"].as_u64().unwrap(),
            ] {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
                if !handle.is_null() {
                    let mut exit = 0;
                    assert_ne!(GetExitCodeProcess(handle, &mut exit), 0);
                    windows_sys::Win32::Foundation::CloseHandle(handle);
                    assert_ne!(exit, 259);
                }
            }
        }
        app.shutdown().await;
    }
}

#[test]
fn identities_paths_and_names_are_strict_and_cross_channel_unique() {
    for input in [
        "https://t.me/c/100/9",
        "share https://t.me/channel/9",
        "https://telegram.me/channel/9",
    ] {
        assert_eq!(message_link(input).unwrap().message, 9);
    }
    for input in [
        "https://t.me/c/100/0",
        "https://t.me/c/100/9?comment=2",
        "https://t.me/c/100/9#x",
        "https://t.me:123/c/100/9",
        "https://t.me/c/100/../9",
        "https://t.me/c/100/9 https://t.me/c/100/10",
        "https://t.me/c/100/%39",
        "http://t.me/c/100/9",
        "https://evil.invalid/channel/9",
        "https://t.me/c/0/1",
    ] {
        assert!(message_link(input).is_err(), "{input}");
    }
    assert!(validate_identity("100_9", "https://t.me/c/100/9"));
    assert!(!validate_identity("101_9", "https://t.me/c/100/9"));
    let link = message_link("https://t.me/c/100/9").unwrap();
    assert_eq!(
        media_path("<a href=\"100/9\">file</a>", &link).unwrap().0,
        "/100/9"
    );
    for html in [
        "<a href=\"101/9\">x</a>",
        "<a href=\"100/8\">x</a>",
        "<a href=\"http://evil/100/9\">x</a>",
        "<a href=\"100/9\">x</a><a href='100/10'>y</a>",
        "<a href=\"100/9?x\">x</a>",
        "<a href=\"100/09\">x</a>",
    ] {
        assert!(media_path(html, &link).is_err(), "{html}");
    }
    assert_eq!(
        filename("attachment; filename=\"../unsafe\\原始:视频.mp4\"").unwrap(),
        "_unsafe_原始_视频.mp4"
    );
}

#[tokio::test]
async fn dependency_probe_only_runs_isolated_version_without_pinning_tool_version() {
    let (root, config) = fixture(json!({"version":"9.1.0", "help":"--url --port"}));
    let status = check(config.path.as_deref()).await.unwrap();
    assert_eq!(status.version, "9.1.0");
    let calls = events(root.path());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["kind"], "version");
    assert_eq!(calls[0]["isolated"], true);
    assert!(check_serve(config.path.as_deref()).await.is_err());
    for options in [json!({"version_output":""}), json!({"version_exit":1})] {
        std::fs::write(root.path().join("options.json"), options.to_string()).unwrap();
        assert_eq!(
            check(config.path.as_deref())
                .await
                .err()
                .unwrap()
                .to_string(),
            "TDL_UNSUPPORTED"
        );
    }
    std::fs::write(root.path().join("options.json"), "{\"version\":\"9.1.0\"}").unwrap();
    assert_eq!(
        check_serve(config.path.as_deref()).await.unwrap().version,
        "9.1.0"
    );
    assert!(check(Some(Path::new("relative.exe"))).await.is_err());
}

#[tokio::test]
async fn go_install_directories_cover_gobin_gopath_and_persisted_go_env_without_path() {
    let root = tempfile::tempdir().unwrap();
    let gobin = root.path().join("custom Go bin");
    let workspace = root.path().join("workspace");
    let second = root.path().join("workspace two");
    let gopath = std::env::join_paths([&workspace, &second]).unwrap();
    let dirs = go_bin_roots(Some(gobin.as_os_str()), Some(&gopath));
    assert_eq!(
        dirs,
        [gobin.clone(), workspace.join("bin"), second.join("bin")]
    );
    let name = if cfg!(windows) { "tdl.exe" } else { "tdl" };
    std::fs::create_dir(&gobin).unwrap();
    std::fs::write(gobin.join(name), "fixture").unwrap();
    let mut repeated = dirs.clone();
    repeated.extend(dirs.clone());
    assert_eq!(installed_executables(&repeated, name), [gobin.join(name)]);
    assert!(go_bin_roots(
        Some(std::ffi::OsStr::new("relative")),
        Some(std::ffi::OsStr::new(""))
    )
    .is_empty());
    let (fixture, config) =
        fixture(json!({"go_env":{"GOBIN":gobin,"GOPATH":gopath.to_string_lossy()}}));
    assert_eq!(
        configured_go_bins(config.path.as_deref().unwrap()).await,
        dirs
    );
    assert_eq!(events(fixture.path())[0]["toolchain"], "local");
}

#[tokio::test]
async fn head_and_streamed_original_bytes_progress_and_safe_commit_for_different_sizes() {
    for size in [12, 65537, 1048576] {
        let (root, config) =
            fixture(json!({"version":"9.1.0","size":size,"chunk":4096,"chunk_delay":0.002}));
        let video = resolve(&config, "https://t.me/public_channel/9").await;
        assert_eq!(video["video_id"], "100_9");
        assert_eq!(video["filesize"], size);
        assert_eq!(video["title"], "原始视频.mp4");
        assert!(video["width"].is_null() && video["duration_ms"].is_null());
        let stage = tempfile::tempdir().unwrap();
        let observed = Arc::new(StdMutex::new(Vec::new()));
        let capture = observed.clone();
        let partial = stage.path().join("video.mp4.part");
        let (_send, cancel) = watch::channel(false);
        let result = operation(
            &config,
            video["canonical_url"].as_str().unwrap(),
            Some(&video),
            Some(stage.path()),
            move |v| {
                assert_eq!(
                    std::fs::metadata(&partial).unwrap().len(),
                    v["bytes"].as_u64().unwrap()
                );
                capture.lock().unwrap().push(v);
            },
            cancel,
        )
        .await
        .unwrap();
        let updates = observed.lock().unwrap();
        assert_eq!(updates.last().unwrap()["bytes"], size);
        assert!(updates
            .windows(2)
            .all(|v| v[0]["bytes"].as_u64() < v[1]["bytes"].as_u64()));
        assert!(updates.iter().all(
            |v| v["bytes"].as_u64().unwrap() <= size && v["speed_bps"].as_f64().unwrap() > 0.0
        ));
        if size == 1048576 {
            assert!(updates.len() >= 2);
        }
        let identity: Video = serde_json::from_value(video).unwrap();
        let target = root.path().join("finished.mp4");
        let committed =
            finalize_to(&identity, &result, stage.path(), &target, |_, _| Ok(())).unwrap();
        assert_eq!(committed.bytes, size);
        assert_eq!(std::fs::metadata(target).unwrap().len(), size);
        assert!(!stage.path().join("video.mp4.part").exists());
        for event in events(root.path()).iter().filter(|e| e["kind"] == "serve") {
            assert!(port_closed(event["port"].as_u64().unwrap()));
        }
    }
}

#[tokio::test]
async fn rejects_errors_truncation_wrong_media_redirects_and_metadata_changes() {
    let (root, config) = fixture(json!({"size":65536}));
    let video = resolve(&config, "https://t.me/c/100/9").await;
    let cases = [
        json!({"size":65536,"actual_size":32}),
        json!({"size":65536,"get_status":500}),
        json!({"size":65536,"bad_header":true}),
        json!({"size":65536,"mime":"text/html"}),
        json!({"size":65536,"home_status":302}),
        json!({"size":65536,"html":"<a href=\"100/10\">x</a>"}),
        json!({"size":65536,"encoding":"gzip"}),
        json!({"size":65536,"name":"changed.mp4"}),
        json!({"size":65537}),
    ];
    for options in cases {
        std::fs::write(root.path().join("options.json"), options.to_string()).unwrap();
        let stage = tempfile::tempdir().unwrap();
        assert!(
            download(&config, &video, stage.path()).await.is_err(),
            "{options}"
        );
        assert!(!stage.path().join("video.mp4").exists());
    }
    assert!(events(root.path())
        .iter()
        .filter(|e| e["kind"] == "request")
        .all(|e| matches!(e["path"].as_str(), Some("/" | "/100/9"))));
}

#[tokio::test]
async fn accepts_only_complete_zero_origin_range_responses() {
    let (root, config) = fixture(json!({"size":4096}));
    let video = resolve(&config, "https://t.me/c/100/9").await;
    for (range, valid) in [
        ("bytes 0-4095/4096", true),
        ("bytes 1-4095/4096", false),
        ("bytes 0-2047/4096", false),
    ] {
        std::fs::write(
            root.path().join("options.json"),
            json!({"size":4096,"get_status":206,"range":range}).to_string(),
        )
        .unwrap();
        let stage = tempfile::tempdir().unwrap();
        assert_eq!(download(&config, &video, stage.path()).await.is_ok(), valid);
    }
}

async fn wait_request(root: &Path, method: &str, path: &str) {
    timeout(Duration::from_secs(8), async {
        loop {
            if events(root)
                .iter()
                .any(|e| e["kind"] == "request" && e["method"] == method && e["path"] == path)
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_service_parses_other_messages_and_channels_during_a_download() {
    let (root, config) = fixture(json!({"size":262144,"chunk":1024,"chunk_delay":0.005,
        "files":{"/100/10":{"size":1234,"name":"second.mp4","fill":65},
                 "/101/9":{"size":2345,"name":"other-channel.mp4","fill":66}}}));
    let session = Arc::new(Session::default());
    let video = resolve(&config, "https://t.me/public_channel/9").await;
    let stage = tempfile::tempdir().unwrap();
    let (send, cancel) = watch::channel(false);
    let c = config.clone();
    let path = stage.path().to_owned();
    let shared = session.clone();
    let running = tokio::spawn(async move {
        shared
            .operation(
                &c,
                "https://t.me/public_channel/9",
                Some(&video),
                Some(&path),
                |_| {},
                cancel,
                None,
            )
            .await
    });
    wait_request(root.path(), "GET", "/100/9").await;
    for (link, identity, size) in [
        ("https://t.me/public_channel/10", "100_10", 1234),
        ("https://t.me/c/101/9", "101_9", 2345),
    ] {
        let (_keep, cancel) = watch::channel(false);
        let parsed = timeout(
            Duration::from_millis(800),
            session.operation(&config, link, None, None, |_| {}, cancel, None),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(parsed["video_id"], identity);
        assert_eq!(parsed["filesize"], size);
        assert!(
            !running.is_finished(),
            "parse must finish before the slow download"
        );
    }
    let (_keep, cancel) = watch::channel(false);
    let error = session
        .operation(
            &config,
            "https://t.me/unknown_channel/11",
            None,
            None,
            |_| {},
            cancel,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "TDL_CHANNEL_ID_REQUIRED");
    // A configuration change cannot replace a service still serving another task.
    let mut changed = config.clone();
    changed.session_directory = Some(root.path().join("other-session"));
    let (_keep, cancel) = watch::channel(false);
    assert_eq!(
        session
            .operation(
                &changed,
                "https://t.me/c/100/10",
                None,
                None,
                |_| {},
                cancel,
                None
            )
            .await
            .unwrap_err()
            .to_string(),
        "TDL_BUSY"
    );
    assert_eq!(
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count(),
        2
    );
    assert_eq!(running.await.unwrap().unwrap()["bytes"], 262144);
    drop(send);
    assert_eq!(
        std::fs::metadata(stage.path().join("video.mp4"))
            .unwrap()
            .len(),
        262144
    );
    assert!(events(root.path())
        .iter()
        .filter(|e| e["kind"] == "request")
        .all(|e| matches!(
            e["path"].as_str(),
            Some("/" | "/100/9" | "/100/10" | "/101/9")
        )));
    for e in events(root.path()).iter().filter(|e| e["kind"] == "serve") {
        assert!(port_closed(e["port"].as_u64().unwrap()));
    }
}

#[tokio::test]
async fn shared_request_cancellation_and_errors_do_not_stop_other_downloads() {
    let (root, config) = fixture(json!({"size":131072,"chunk":1024,"chunk_delay":0.01,
        "files":{"/100/10":{"size":32768,"head_delay":1},"/100/11":{"head_status":500}}}));
    let video = resolve(&config, "https://t.me/c/100/9").await;
    let session = Arc::new(Session::default());
    let stage = tempfile::tempdir().unwrap();
    let (send, cancel) = watch::channel(false);
    let c = config.clone();
    let shared = session.clone();
    let path = stage.path().to_owned();
    let running = tokio::spawn(async move {
        shared
            .operation(
                &c,
                "https://t.me/c/100/9",
                Some(&video),
                Some(&path),
                |_| {},
                cancel,
                None,
            )
            .await
    });
    wait_request(root.path(), "GET", "/100/9").await;
    let (parse_send, parse_cancel) = watch::channel(false);
    let c = config.clone();
    let shared = session.clone();
    let parsing = tokio::spawn(async move {
        shared
            .operation(
                &c,
                "https://t.me/c/100/10",
                None,
                None,
                |_| {},
                parse_cancel,
                None,
            )
            .await
    });
    wait_request(root.path(), "HEAD", "/100/10").await;
    parse_send.send(true).unwrap();
    assert_eq!(parsing.await.unwrap().unwrap_err().to_string(), "CANCELED");
    let (_keep, cancel) = watch::channel(false);
    assert_eq!(
        session
            .operation(
                &config,
                "https://t.me/c/100/11",
                None,
                None,
                |_| {},
                cancel,
                None
            )
            .await
            .unwrap_err()
            .to_string(),
        "TDL_HTTP_ERROR"
    );
    let port = events(root.path())
        .into_iter()
        .rfind(|e| e["kind"] == "serve")
        .unwrap()["port"]
        .as_u64()
        .unwrap();
    assert!(!port_closed(port));
    assert!(!running.is_finished());
    send.send(true).unwrap();
    assert_eq!(running.await.unwrap().unwrap_err().to_string(), "CANCELED");
    assert!(port_closed(port));
    assert!(!stage.path().join("video.mp4").exists());
}

#[tokio::test]
async fn shared_service_rejects_upstream_concatenated_cache_key_identity_collision() {
    let (root, config) = fixture(json!({"size":131072,"chunk":1024,"chunk_delay":0.01}));
    let video = resolve(&config, "https://t.me/c/100/12").await;
    let session = Arc::new(Session::default());
    let stage = tempfile::tempdir().unwrap();
    let (send, cancel) = watch::channel(false);
    let c = config.clone();
    let shared = session.clone();
    let path = stage.path().to_owned();
    let running = tokio::spawn(async move {
        shared
            .operation(
                &c,
                "https://t.me/c/100/12",
                Some(&video),
                Some(&path),
                |_| {},
                cancel,
                None,
            )
            .await
    });
    wait_request(root.path(), "GET", "/100/12").await;
    let (_keep, cancel) = watch::channel(false);
    assert_eq!(
        session
            .operation(
                &config,
                "https://t.me/c/1001/2",
                None,
                None,
                |_| {},
                cancel,
                None
            )
            .await
            .unwrap_err()
            .to_string(),
        "TDL_IDENTITY_MISMATCH"
    );
    assert!(!events(root.path()).iter().any(|e| e["path"] == "/1001/2"));
    send.send(true).unwrap();
    assert_eq!(running.await.unwrap().unwrap_err().to_string(), "CANCELED");
}

#[tokio::test]
async fn startup_timeout_port_conflict_and_illegal_request_boundary() {
    let (_root, config) = fixture(json!({}));
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cmd = command(config.path.as_ref().unwrap());
    cmd.arg("exit");
    let process = Process::spawn(cmd).unwrap();
    let mut service = Service {
        process,
        client: http_client().unwrap(),
        origin: url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        path: None,
    };
    sleep(Duration::from_millis(150)).await;
    assert_eq!(
        service
            .inspect(
                &message_link("https://t.me/c/100/9").unwrap(),
                Duration::from_millis(200)
            )
            .await
            .unwrap_err()
            .to_string(),
        "TDL_PORT_CONFLICT"
    );
    assert!(service
        .request(Method::GET, "http://evil.invalid")
        .await
        .is_err());
    assert!(service.request(Method::GET, "/100/9").await.is_err());
    service.process.stop().await.unwrap();
    drop(listener);
    let mut cmd = command(config.path.as_ref().unwrap());
    cmd.arg("sleep");
    let process = Process::spawn(cmd).unwrap();
    service.process = process;
    assert_eq!(
        service
            .inspect(
                &message_link("https://t.me/c/100/9").unwrap(),
                Duration::from_millis(200)
            )
            .await
            .unwrap_err()
            .to_string(),
        "TDL_START_TIMEOUT"
    );
    service.process.stop().await.unwrap();
}

async fn wait(core: &Core, id: &str, expected: Status) -> crate::desktop_core::Task {
    timeout(Duration::from_secs(15), async {
        loop {
            let task = core.list().into_iter().find(|t| t.id == id).unwrap();
            if task.status == expected {
                return task;
            }
            assert_ne!(task.status, Status::Failed, "{:?}", task.error);
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn extracted_telegram_cover_reuses_task_display_cache_and_restart() {
    let (root, config) = fixture(json!({"size":4096}));
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let open = || {
        Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            project
                .join(if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                })
                .to_string_lossy()
                .into_owned(),
            vec![project
                .join("tests/fixtures/desktop_engine.py")
                .to_string_lossy()
                .into_owned()],
            |_| {},
        )
        .unwrap()
    };
    let app = open();
    app.save_tdl(config).unwrap();
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.download(&task.id, false).unwrap();
    let done = wait(&app, &task.id, Status::Completed).await;
    assert!(done.cover_path.is_none());
    assert!(done.cover_error.is_none());
    assert!(done.cover_cached);
    assert_eq!(app.cover(&task.id).unwrap(), b"\x89PNG\r\n\x1a\nfixture");
    let video_path = done.path.unwrap();
    assert_eq!(std::fs::read(&video_path).unwrap().len(), 4096);
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        1
    );
    app.shutdown().await;
    drop(app);
    let app = open();
    assert!(app.cover(&task.id).is_ok());
    let services = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    let cached = app.resolve("https://t.me/c/100/9".into()).unwrap();
    assert!(cached.cache_hit && cached.cover_cached);
    assert_eq!(app.cover(&cached.id).unwrap(), b"\x89PNG\r\n\x1a\nfixture");
    assert_eq!(
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count(),
        services
    );
    app.delete_local_file(&task.id).unwrap();
    assert!(!video_path.exists());
    assert!(app.list().iter().all(|t| t.id != task.id));
    assert!(app.cover(&task.id).is_err());
    // Deleting the video also removes its task. An active preview keeps the
    // shared cover until the last consumer is removed.
    assert!(app.cover(&cached.id).is_ok());
    app.cancel(&cached.id).await.unwrap();
    app.remove_history(&cached.id).unwrap();
    let fresh = app.resolve("https://t.me/c/100/9".into()).unwrap();
    assert!(fresh.cache_hit && !fresh.cover_cached);
    assert!(app.cover(&fresh.id).is_err());
    assert_eq!(
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count(),
        services
    );
    assert!(!video_path.exists());
    app.shutdown().await;
}

#[tokio::test]
async fn telegram_sqlite_cover_cleanup_is_atomic_shared_and_channel_specific() {
    let (root, config) = fixture(json!({"size":4096}));
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let db_path = root.path().join("tasks.sqlite");
    let app = Core::open(
        &db_path,
        root.path().join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into_owned(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into_owned()],
        |_| {},
    )
    .unwrap();
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    let preview = app.resolve("https://t.me/c/100/9".into()).unwrap();
    assert!(!preview.cover_cached);
    app.download(&first.id, false).unwrap();
    let first = wait(&app, &first.id, Status::Completed).await;
    let second = app.resolve("https://t.me/c/100/9".into()).unwrap();
    app.download(&second.id, true).unwrap();
    let second = wait(&app, &second.id, Status::Completed).await;
    let other = app.resolve("https://t.me/c/101/9".into()).unwrap();
    wait(&app, &other.id, Status::Ready).await;
    app.download(&other.id, false).unwrap();
    let other = wait(&app, &other.id, Status::Completed).await;
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let covers = || {
        db.query_row(
            "SELECT count(*) FROM parse_cache WHERE cover IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(covers(), 2);
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        3
    );
    app.remove_history(&first.id).unwrap();
    assert_eq!(covers(), 2);
    assert!(app.cover(&second.id).is_ok());
    assert!(first.path.as_ref().unwrap().is_file());
    app.remove_history(&second.id).unwrap();
    // This preview was created before the cover existed. Its work identity
    // still references the shared cache, even though its old flag is false.
    assert_eq!(covers(), 2);
    app.cancel(&preview.id).await.unwrap();
    db.execute_batch(
        "CREATE TRIGGER block_cover_cleanup BEFORE UPDATE OF cover ON parse_cache
        WHEN NEW.cover IS NULL BEGIN SELECT RAISE(ABORT, 'test cleanup error'); END;",
    )
    .unwrap();
    assert_eq!(
        app.remove_history(&preview.id).unwrap_err().to_string(),
        "DATABASE_ERROR"
    );
    assert!(app.list().iter().any(|t| t.id == preview.id));
    assert_eq!(covers(), 2);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM download_tasks WHERE id=?",
            [&preview.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER block_cover_cleanup")
        .unwrap();
    app.remove_history(&preview.id).unwrap();
    assert_eq!(covers(), 1);
    assert!(second.path.as_ref().unwrap().is_file());
    assert!(app.cover(&other.id).is_ok());
    app.shutdown().await;
    drop(app);
    // Simulate the pre-completion snapshot of a video whose commit intent and
    // cached cover are durable, as if the app exited before publishing success.
    let mut snapshot = other.clone();
    snapshot.status = Status::Downloading;
    snapshot.cover_cached = false;
    db.execute(
        "UPDATE download_tasks SET snapshot=? WHERE id=?",
        rusqlite::params![serde_json::to_string(&snapshot).unwrap(), other.id],
    )
    .unwrap();
    let app = Core::open(
        &db_path,
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let recovered = app.list().into_iter().find(|t| t.id == other.id).unwrap();
    assert_eq!(recovered.status, Status::Completed);
    assert!(recovered.cover_cached && recovered.cover_path.is_none());
    assert!(app.cover(&other.id).is_ok());
    app.remove_history(&other.id).unwrap();
    assert_eq!(covers(), 0);
    assert!(other.path.as_ref().unwrap().is_file());
    app.shutdown().await;
}

#[tokio::test]
async fn telegram_cover_cache_write_failure_keeps_completed_video_without_sidecar_file() {
    let (root, config) = fixture(json!({"size":4096}));
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let db_path = root.path().join("tasks.sqlite");
    let app = Core::open(
        &db_path,
        root.path().join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into_owned(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into_owned()],
        |_| {},
    )
    .unwrap();
    app.save_tdl(config).unwrap();
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    let db = rusqlite::Connection::open(db_path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER block_cover_save BEFORE UPDATE OF cover ON parse_cache
        BEGIN SELECT RAISE(ABORT, 'test cache error'); END;",
    )
    .unwrap();
    app.download(&task.id, false).unwrap();
    let done = wait(&app, &task.id, Status::Completed).await;
    assert_eq!(done.bytes, 4096);
    assert!(done.path.unwrap().is_file());
    assert!(done.cover_path.is_none() && !done.cover_cached);
    assert_eq!(done.cover_error.as_deref(), Some("COVER_UNAVAILABLE"));
    app.shutdown().await;
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn unavailable_or_failed_cover_does_not_fail_telegram_download() {
    for missing_engine in [false, true] {
        let (root, config) = fixture(json!({"size":4096}));
        std::fs::write(root.path().join("cover-mode"), "failure").unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let program = if missing_engine {
            "unused-python".into()
        } else {
            project
                .join(if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                })
                .to_string_lossy()
                .into_owned()
        };
        let app = Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            program,
            vec![project
                .join("tests/fixtures/desktop_engine.py")
                .to_string_lossy()
                .into_owned()],
            |_| {},
        )
        .unwrap();
        app.save_tdl(config).unwrap();
        let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        let done = wait(&app, &task.id, Status::Completed).await;
        assert_eq!(done.cover_error.as_deref(), Some("COVER_UNAVAILABLE"));
        assert!(done.cover_path.is_none());
        assert_eq!(std::fs::metadata(done.path.unwrap()).unwrap().len(), 4096);
        app.shutdown().await;
    }
}

#[tokio::test]
async fn cancel_and_shutdown_during_cover_extraction_reap_operation_before_staging_cleanup() {
    for shutdown in [false, true] {
        let (root, config) = fixture(json!({"size":4096}));
        std::fs::write(root.path().join("cover-mode"), "block").unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let app = Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            project
                .join(if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                })
                .to_string_lossy()
                .into_owned(),
            vec![project
                .join("tests/fixtures/desktop_engine.py")
                .to_string_lossy()
                .into_owned()],
            |_| {},
        )
        .unwrap();
        app.save_tdl(config).unwrap();
        let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        timeout(Duration::from_secs(5), async {
            while !root.path().join("cover-started").exists() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if shutdown {
            app.shutdown().await;
        } else {
            app.cancel(&task.id).await.unwrap();
        }
        let done = app.list().into_iter().find(|t| t.id == task.id).unwrap();
        assert_eq!(done.status, Status::Canceled);
        assert!(done.path.is_none());
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
        for event in events(root.path()).iter().filter(|e| e["kind"] == "serve") {
            assert!(port_closed(event["port"].as_u64().unwrap()));
        }
        app.shutdown().await;
    }
}

#[tokio::test]
async fn task_queue_cache_restart_dedup_commit_and_shutdown_use_existing_lifecycle() {
    let (root, config) = fixture(json!({"size":65536,"chunk":1024,"chunk_delay":0.02}));
    let db = root.path().join("tasks.sqlite");
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert!(app.resolve("https://t.me/c/100/9".into()).is_err());
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    let other = app.resolve("https://t.me/c/101/9".into()).unwrap();
    wait(&app, &other.id, Status::Ready).await;
    app.shutdown().await;
    drop(app);
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert!(
        app.list()
            .iter()
            .filter(|t| t.status == Status::Ready)
            .count()
            >= 2
    );
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    let cached = app.resolve("https://t.me/c/100/9".into()).unwrap();
    assert!(cached.cache_hit);
    assert_eq!(
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count(),
        before
    );
    app.download(&first.id, false).unwrap();
    app.download(&other.id, false).unwrap();
    assert_eq!(
        app.list().iter().find(|t| t.id == other.id).unwrap().status,
        Status::Downloading
    );
    let done = wait(&app, &first.id, Status::Completed).await;
    let second = wait(&app, &other.id, Status::Completed).await;
    assert_ne!(done.path, second.path);
    assert_eq!(done.bytes, 65536);
    assert_eq!(
        app.download(&cached.id, false).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    app.download(&cached.id, true).unwrap();
    timeout(Duration::from_secs(5), async {
        while app.list().iter().find(|t| t.id == cached.id).unwrap().bytes == 0 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.shutdown().await;
    assert_eq!(
        app.list()
            .iter()
            .find(|t| t.id == cached.id)
            .unwrap()
            .status,
        Status::Canceled
    );
    for e in events(root.path()).iter().filter(|e| e["kind"] == "serve") {
        assert!(port_closed(e["port"].as_u64().unwrap()));
    }
    assert!(std::fs::read_dir(root.path().join("downloads"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("yingqu-")));
}

#[tokio::test]
async fn core_can_parse_during_download_after_restart_and_cancel_one_shared_transfer() {
    let (root, config) = fixture(json!({"size":1048576,"chunk":1024,"chunk_delay":0.01,
        "files":{"/100/10":{"size":131072,"name":"second.mp4","fill":65}}}));
    let db = root.path().join("tasks.sqlite");
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    app.save_tdl(config.clone()).unwrap();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.shutdown().await;
    drop(app);
    // Restored previews retain the validated alias, but the new process must
    // confirm it against the numeric identity before serving a download.
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    let second = app
        .resolve("https://t.me/public_channel/10".into())
        .unwrap();
    let parsed = timeout(
        Duration::from_secs(1),
        wait(&app, &second.id, Status::Ready),
    )
    .await
    .unwrap();
    assert_eq!(parsed.video.as_ref().unwrap()["video_id"], "100_10");
    assert_eq!(parsed.video.as_ref().unwrap()["filesize"], 131072);
    assert_eq!(
        app.list().iter().find(|t| t.id == first.id).unwrap().status,
        Status::Downloading
    );
    let unknown = app
        .resolve("https://t.me/unknown_channel/9".into())
        .unwrap();
    let failed = timeout(
        Duration::from_secs(1),
        wait(&app, &unknown.id, Status::Failed),
    )
    .await
    .unwrap();
    assert_eq!(failed.error.as_deref(), Some("TDL_CHANNEL_ID_REQUIRED"));
    // A failed parse releases the parsing state, so another link works immediately.
    let numeric = app.resolve("https://t.me/c/101/9".into()).unwrap();
    assert_eq!(
        wait(&app, &numeric.id, Status::Ready).await.video.unwrap()["video_id"],
        "101_9"
    );
    app.download(&second.id, false).unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if app.list().iter().any(|t| t.id == second.id && t.bytes > 0) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.cancel(&first.id).await.unwrap();
    assert_eq!(
        app.list().iter().find(|t| t.id == first.id).unwrap().status,
        Status::Canceled
    );
    let done = wait(&app, &second.id, Status::Completed).await;
    assert_eq!(done.bytes, 131072);
    let bytes = std::fs::read(done.path.unwrap()).unwrap();
    assert_eq!(&bytes[..12], b"\0\0\0\x18ftypisom");
    assert!(bytes[12..].iter().all(|b| *b == 65));
    assert_eq!(
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count(),
        before
    );
    // Unknown usernames were never sent to HTTP, and only one live service served both GETs.
    assert!(!events(root.path()).iter().any(|e| e["path"]
        .as_str()
        .is_some_and(|p| p.contains("unknown_channel"))));
    app.shutdown().await;
    for e in events(root.path()).iter().filter(|e| e["kind"] == "serve") {
        assert!(port_closed(e["port"].as_u64().unwrap()));
    }
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM task_staging", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn core_reuses_another_channels_cached_username_without_replacing_running_service() {
    let (root, config) = fixture(json!({"channels":{"known_channel":200},"size":4096,
        "files":{"/100/9":{"size":65536,"release_file":"first-download.release"}}}));
    let open = || {
        Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            |_| {},
        )
        .unwrap()
    };
    let app = open();
    app.save_tdl(config).unwrap();
    let known = app
        .resolve("https://telegram.me/Known_Channel/9".into())
        .unwrap();
    wait(&app, &known.id, Status::Ready).await;
    app.cancel(&known.id).await.unwrap();
    app.remove_history(&known.id).unwrap(); // independent parse cache survives removal
    app.shutdown().await;
    drop(app);
    let app = open();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    let other = app.resolve("https://t.me/known_channel/10".into()).unwrap();
    let parsed = wait(&app, &other.id, Status::Ready).await;
    assert_eq!(parsed.video.unwrap()["video_id"], "200_10");
    assert!(!parsed.cache_hit); // new message still gets its own real HEAD
    wait_request(root.path(), "HEAD", "/200/10").await;
    assert_eq!(
        before,
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count()
    );
    assert_eq!(
        app.list().iter().find(|t| t.id == first.id).unwrap().status,
        Status::Downloading
    );
    std::fs::write(root.path().join("first-download.release"), b"release").unwrap();
    wait(&app, &first.id, Status::Completed).await;
    app.shutdown().await;
}

#[tokio::test]
async fn deferred_channel_waits_for_telegram_but_allows_other_platform_and_keeps_fifo() {
    let (root, config) = fixture(
        json!({"channels":{"new_channel":200,"canceled_channel":300},"size":4096,
        "files":{"/100/9":{"size":65536,"release_file":"first-download.release"}}}),
    );
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let history = Arc::new(StdMutex::new(Vec::new()));
    let recorded = history.clone();
    let app = Core::open(
        &root.path().join("tasks.sqlite"),
        root.path().join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into()],
        move |event| {
            if let crate::desktop_core::TaskEvent::Updated(task) = event {
                recorded
                    .lock()
                    .unwrap()
                    .push((task.id.clone(), task.status));
            }
        },
    )
    .unwrap();
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let deferred = app
        .resolve("分享文案 https://t.me/new_channel/9".into())
        .unwrap();
    wait(&app, &deferred.id, Status::Failed).await;
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    app.download(&deferred.id, false).unwrap();
    let queued = app
        .list()
        .into_iter()
        .find(|t| t.id == deferred.id)
        .unwrap();
    assert_eq!(queued.status, Status::Queued);
    assert_eq!(
        queued.telegram_link.as_deref(),
        Some("https://t.me/new_channel/9")
    );
    assert!(queued.video.is_none() && queued.error.is_none());
    assert!(app.download(&deferred.id, false).is_err());
    let canceled = app
        .resolve("https://t.me/canceled_channel/9".into())
        .unwrap();
    wait(&app, &canceled.id, Status::Failed).await;
    app.download(&canceled.id, false).unwrap();
    app.cancel(&canceled.id).await.unwrap();
    let python = app
        .resolve("https://www.douyin.com/video/123".into())
        .unwrap();
    wait(&app, &python.id, Status::Ready).await;
    app.download(&python.id, false).unwrap();
    wait(&app, &python.id, Status::Completed).await;
    assert_eq!(
        app.list().iter().find(|t| t.id == first.id).unwrap().status,
        Status::Downloading
    );
    let later = app
        .resolve("https://t.me/public_channel/10".into())
        .unwrap();
    wait(&app, &later.id, Status::Ready).await;
    app.download(&later.id, false).unwrap();
    assert_eq!(
        app.list().iter().find(|t| t.id == later.id).unwrap().status,
        Status::Queued
    );
    assert_eq!(
        before,
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count()
    );
    std::fs::write(root.path().join("first-download.release"), b"release").unwrap();
    wait(&app, &first.id, Status::Completed).await;
    let completed = wait(&app, &deferred.id, Status::Completed).await;
    wait(&app, &later.id, Status::Completed).await;
    assert_eq!(completed.created_at, deferred.created_at);
    assert_eq!(completed.queue_order, queued.queue_order);
    assert_eq!(completed.video.unwrap()["video_id"], "200_9");
    {
        let observed = history.lock().unwrap();
        let ended = observed
            .iter()
            .position(|(id, status)| id == &first.id && *status == Status::Completed)
            .unwrap();
        let started = observed
            .iter()
            .rposition(|(id, status)| id == &deferred.id && *status == Status::Parsing)
            .unwrap();
        assert!(ended < started);
    }
    assert!(!events(root.path())
        .iter()
        .any(|e| e["kind"] == "serve" && e["channel"] == "300"));
    app.shutdown().await;
}

#[tokio::test]
async fn deferred_parse_failure_retains_link_and_retry_keeps_identity() {
    let (root, config) = fixture(json!({"channels":{"new_channel":200},"size":4096,
        "files":{"/100/9":{"size":65536,"chunk":1024,"chunk_delay":0.02},"/200/9":{"head_status":403}}}));
    let app = login_core(root.path());
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let pending = app.resolve("https://t.me/new_channel/9".into()).unwrap();
    wait(&app, &pending.id, Status::Failed).await;
    app.download(&pending.id, false).unwrap();
    wait(&app, &first.id, Status::Completed).await;
    let failed = wait(&app, &pending.id, Status::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("TDL_HTTP_ERROR"));
    assert!(failed.video.is_none() && failed.download_requested);
    std::fs::write(
        root.path().join("options.json"),
        json!({"channels":{"new_channel":200},"size":4096}).to_string(),
    )
    .unwrap();
    let retry = app.retry(&pending.id).unwrap();
    assert_eq!(retry.id, pending.id);
    assert_eq!(retry.created_at, pending.created_at);
    wait(&app, &pending.id, Status::Completed).await;
    app.shutdown().await;
}

#[tokio::test]
async fn deferred_queue_shutdown_and_restart_keep_link_without_automatic_resume() {
    let (root, config) = fixture(json!({"channels":{"new_channel":200},"size":4096,
        "files":{"/100/9":{"size":262144,"chunk":1024,"chunk_delay":0.02}}}));
    let app = login_core(root.path());
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let pending = app.resolve("https://t.me/new_channel/9".into()).unwrap();
    wait(&app, &pending.id, Status::Failed).await;
    app.download(&pending.id, false).unwrap();
    let queued = app.list().into_iter().find(|t| t.id == pending.id).unwrap();
    app.shutdown().await;
    assert_eq!(
        app.list()
            .iter()
            .find(|t| t.id == pending.id)
            .unwrap()
            .status,
        Status::Canceled
    );
    drop(app);
    // Simulate a crash snapshot from before shutdown, using only fixture data.
    let db = rusqlite::Connection::open(root.path().join("login.sqlite")).unwrap();
    db.execute(
        "UPDATE download_tasks SET snapshot=? WHERE id=?",
        rusqlite::params![serde_json::to_string(&queued).unwrap(), pending.id],
    )
    .unwrap();
    drop(db);
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "serve")
        .count();
    let app = login_core(root.path());
    let restored = app.list().into_iter().find(|t| t.id == pending.id).unwrap();
    assert_eq!(restored.status, Status::Failed);
    assert_eq!(restored.error.as_deref(), Some("INTERRUPTED"));
    assert_eq!(restored.telegram_link, queued.telegram_link);
    assert_eq!(
        before,
        events(root.path())
            .iter()
            .filter(|e| e["kind"] == "serve")
            .count()
    );
    let retry = app.retry(&pending.id).unwrap();
    assert_eq!(retry.id, pending.id);
    assert_eq!(retry.created_at, pending.created_at);
    wait(&app, &pending.id, Status::Completed).await;
    app.shutdown().await;
}

#[tokio::test]
async fn deferred_duplicate_download_requires_confirmation() {
    let (root, config) = fixture(json!({"channels":{"other":200},"size":4096,
        "files":{"/100/9":{"size":65536,"chunk":1024,"chunk_delay":0.02}}}));
    let app = login_core(root.path());
    app.save_tdl(config).unwrap();
    let original = app.resolve("https://t.me/c/200/9".into()).unwrap();
    wait(&app, &original.id, Status::Ready).await;
    app.download(&original.id, false).unwrap();
    wait(&app, &original.id, Status::Completed).await;
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let pending = app.resolve("https://t.me/other/9".into()).unwrap();
    wait(&app, &pending.id, Status::Failed).await;
    app.download(&pending.id, false).unwrap();
    wait(&app, &first.id, Status::Completed).await;
    wait(&app, &pending.id, Status::Ready).await;
    assert_eq!(
        app.download(&pending.id, false).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    app.cancel(&pending.id).await.unwrap();
    app.shutdown().await;
}

#[tokio::test]
async fn deferred_resolution_cancellation_reaps_startup_and_allows_next_parse() {
    let (root, config) = fixture(json!({"size":4096,
        "files":{"/100/9":{"size":65536,"chunk":1024,"chunk_delay":0.02}}}));
    let app = login_core(root.path());
    app.save_tdl(config).unwrap();
    let first = app.resolve("https://t.me/public_channel/9".into()).unwrap();
    wait(&app, &first.id, Status::Ready).await;
    app.download(&first.id, false).unwrap();
    wait_request(root.path(), "GET", "/100/9").await;
    let pending = app.resolve("https://t.me/new_channel/9".into()).unwrap();
    wait(&app, &pending.id, Status::Failed).await;
    std::fs::write(
        root.path().join("options.json"),
        json!({"channels":{"new_channel":200},"startup_delay":10,"size":4096}).to_string(),
    )
    .unwrap();
    let before = events(root.path())
        .iter()
        .filter(|e| e["kind"] == "session")
        .count();
    app.download(&pending.id, false).unwrap();
    wait(&app, &first.id, Status::Completed).await;
    wait(&app, &pending.id, Status::Parsing).await;
    timeout(Duration::from_secs(5), async {
        while events(root.path())
            .iter()
            .filter(|e| e["kind"] == "session")
            .count()
            == before
        {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.cancel(&pending.id).await.unwrap();
    assert_eq!(
        app.list()
            .iter()
            .find(|t| t.id == pending.id)
            .unwrap()
            .status,
        Status::Canceled
    );
    std::fs::write(
        root.path().join("options.json"),
        json!({"size":4096}).to_string(),
    )
    .unwrap();
    let next = app.resolve("https://t.me/c/100/10".into()).unwrap();
    wait(&app, &next.id, Status::Ready).await;
    app.shutdown().await;
}

#[tokio::test]
async fn a_reassigned_bootstrap_username_cannot_download_a_cached_numeric_identity() {
    for input in [
        "https://t.me/public_channel/9",
        "https://telegram.me/public_channel/9",
    ] {
        let (root, config) = fixture(json!({"size":4096,"channel":100}));
        let app = login_core(root.path());
        app.save_tdl(config).unwrap();
        let task = app.resolve(input.into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        std::fs::write(
            root.path().join("options.json"),
            json!({"size":4096,"channel":101}).to_string(),
        )
        .unwrap();
        app.download(&task.id, false).unwrap();
        let failed = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed.error.as_deref(), Some("TDL_IDENTITY_MISMATCH"));
        assert!(!events(root.path())
            .iter()
            .any(|e| e["method"] == "GET" && e["path"] == "/101/9"));
        // The exact stale public alias was removed, while the original numeric
        // identity and preview remain usable without any automatic fallback/retry.
        let canonical = app.resolve("https://t.me/c/100/9".into()).unwrap();
        let ready = wait(&app, &canonical.id, Status::Ready).await;
        assert!(ready.cache_hit);
        assert_eq!(ready.video.unwrap()["video_id"], "100_9");
        app.download(&canonical.id, false).unwrap();
        assert_eq!(
            wait(&app, &canonical.id, Status::Completed).await.bytes,
            4096
        );
        let reassigned = app.resolve(input.into()).unwrap();
        let fresh = wait(&app, &reassigned.id, Status::Ready).await;
        assert!(!fresh.cache_hit);
        assert_eq!(fresh.video.unwrap()["video_id"], "101_9");
        app.shutdown().await;
    }
}

#[tokio::test]
async fn changed_cached_metadata_is_invalidated_and_retry_resolves_fresh_original() {
    let (root, config) = fixture(json!({"size":4096}));
    let app = Core::open(
        &root.path().join("tasks.sqlite"),
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    app.save_tdl(config).unwrap();
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    std::fs::write(
        root.path().join("options.json"),
        json!({"size":8192}).to_string(),
    )
    .unwrap();
    app.download(&task.id, false).unwrap();
    let failed = wait(&app, &task.id, Status::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("TDL_MEDIA_CHANGED"));
    let retry = app.retry(&task.id).unwrap();
    let finished = wait(&app, &retry.id, Status::Completed).await;
    assert_eq!(finished.bytes, 8192);
    assert_eq!(finished.video.unwrap()["selected_format_id"], "original");
    app.shutdown().await;
}

#[tokio::test]
async fn core_cancellation_cleans_partial_files_and_restart_rejects_corrupt_commit_intent() {
    let (root, config) = fixture(json!({"size":1048576,"chunk":1024,"chunk_delay":0.01}));
    let db = root.path().join("tasks.sqlite");
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    app.save_tdl(config).unwrap();
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&app, &task.id, Status::Ready).await;
    app.download(&task.id, false).unwrap();
    timeout(Duration::from_secs(5), async {
        while app.list().iter().find(|t| t.id == task.id).unwrap().bytes == 0 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.cancel(&task.id).await.unwrap();
    assert_eq!(
        app.list().iter().find(|t| t.id == task.id).unwrap().status,
        Status::Canceled
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        0
    );
    app.shutdown().await;
    drop(app);
    let connection = rusqlite::Connection::open(&db).unwrap();
    let mut snapshot: Value = serde_json::from_str(
        &connection
            .query_row(
                "SELECT snapshot FROM download_tasks WHERE id=?",
                [&task.id],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    snapshot["status"] = "downloading".into();
    connection
        .execute(
            "UPDATE download_tasks SET snapshot=?1 WHERE id=?2",
            rusqlite::params![snapshot.to_string(), task.id],
        )
        .unwrap();
    let target = root.path().join("corrupted.mp4");
    std::fs::write(&target, b"<html>error!!").unwrap();
    connection
        .execute(
            "INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,12)",
            rusqlite::params![
                task.id,
                root.path().join("missing").to_string_lossy(),
                target.to_string_lossy()
            ],
        )
        .unwrap();
    drop(connection);
    let app = Core::open(
        &db,
        root.path().join("downloads"),
        "unused-python".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let recovered = app.list().into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(recovered.status, Status::Failed);
    assert_eq!(recovered.error.as_deref(), Some("INTERRUPTED"));
    app.shutdown().await;
}

#[test]
fn recovery_checks_file_header_and_size_and_does_not_overwrite_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("video.mp4");
    std::fs::write(&path, b"0000ftypisom").unwrap();
    assert!(crate::files::tdl_recovery_valid(&path, 12));
    assert!(!crate::files::tdl_recovery_valid(&path, 13));
    std::fs::write(&path, b"<html>error!!").unwrap();
    assert!(!crate::files::tdl_recovery_valid(&path, 12));
    std::fs::write(&path, b"0000ftypisom").unwrap();
    let identity:Video=serde_json::from_value(json!({"platform":"telegram","video_id":"100_9","canonical_url":"https://t.me/c/100/9","resolver":"tdl","kind":"video"})).unwrap();
    let result = json!({"platform":"telegram","video_id":"100_9","container":"mp4","filename":"video.mp4","bytes":12});
    let target = root.path().join("existing.mp4");
    std::fs::write(&target, b"keep").unwrap();
    assert!(finalize_to(&identity, &result, root.path(), &target, |_, _| Ok(())).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
}
