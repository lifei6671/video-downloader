//! Exercise the actual build configuration in child processes without real credentials.
#[path = "../build_support/telegram_app.rs"]
mod telegram_app;

use std::{fs, process::Command};

const TEST_HASH: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";

#[test]
fn configure_child() {
    if std::env::var_os("YINGQU_BUILD_IDENTITY_CHILD").is_some() {
        telegram_app::configure();
    }
}

struct BuildResult {
    success: bool,
    diagnostics: String,
    generated: Option<String>,
}

fn configure(
    profile: &str,
    desktop: bool,
    id: Option<&str>,
    hash: Option<&str>,
    local: Option<serde_json::Value>,
) -> BuildResult {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("src-tauri");
    let out = dir.path().join("out");
    fs::create_dir(&manifest).unwrap();
    fs::create_dir(&out).unwrap();
    if let Some(local) = local {
        fs::write(
            dir.path().join("telegram-app.local.json"),
            serde_json::to_vec(&local).unwrap(),
        )
        .unwrap();
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "configure_child", "--nocapture"])
        .env("YINGQU_BUILD_IDENTITY_CHILD", "1")
        .env("CARGO_MANIFEST_DIR", &manifest)
        .env("OUT_DIR", &out)
        .env("PROFILE", profile)
        .env_remove("CARGO_FEATURE_DESKTOP")
        .env_remove("YINGQU_TELEGRAM_API_ID")
        .env_remove("YINGQU_TELEGRAM_API_HASH");
    if desktop {
        command.env("CARGO_FEATURE_DESKTOP", "1");
    }
    if let Some(id) = id {
        command.env("YINGQU_TELEGRAM_API_ID", id);
    }
    if let Some(hash) = hash {
        command.env("YINGQU_TELEGRAM_API_HASH", hash);
    }
    let output = command.output().unwrap();
    BuildResult {
        success: output.status.success(),
        diagnostics: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        generated: fs::read_to_string(out.join("telegram_app.rs")).ok(),
    }
}

#[test]
fn release_desktop_rejects_missing_partial_and_invalid_identity_without_local_fallback() {
    let invalid = [
        (None, None),
        (Some("42"), None),
        (None, Some(TEST_HASH)),
        (Some(""), Some(TEST_HASH)),
        (Some("0"), Some(TEST_HASH)),
        (Some("-1"), Some(TEST_HASH)),
        (Some("2147483648"), Some(TEST_HASH)),
        (Some(" 42 "), Some(TEST_HASH)),
        (Some("42"), Some("")),
        (Some("42"), Some("0123456789abcdef0123456789abcdef\n")),
        (Some("42"), Some("g123456789abcdef0123456789abcdef")),
        (Some("42"), Some("１２３４５６７８")),
    ];
    for (id, hash) in invalid {
        let result = configure(
            "release",
            true,
            id,
            hash,
            Some(serde_json::json!({"api_id": 42, "api_hash": TEST_HASH})),
        );
        assert!(!result.success, "invalid identity was accepted");
        assert!(result.generated.is_none());
        assert!(result.diagnostics.contains("YINGQU_TELEGRAM_API_ID"));
        assert!(result.diagnostics.contains("YINGQU_TELEGRAM_API_HASH"));
        assert!(!result.diagnostics.contains(TEST_HASH));
    }
}

#[test]
fn release_desktop_accepts_runtime_valid_identity_and_emits_no_identity_values() {
    let hash = TEST_HASH.to_uppercase();
    let result = configure("release", true, Some("2147483647"), Some(&hash), None);
    assert!(result.success, "{}", result.diagnostics);
    let generated = result.generated.unwrap();
    assert!(generated.contains("Some(\"2147483647\")"));
    assert!(generated.contains(&format!("Some({hash:?})")));
    assert!(!result.diagnostics.contains("2147483647"));
    assert!(!result.diagnostics.contains(&hash));
}

#[test]
fn invalid_identity_values_are_not_in_failure_diagnostics() {
    let result = configure(
        "release",
        true,
        Some("api-id-must-not-leak"),
        Some("api-hash-must-not-leak"),
        None,
    );
    assert!(!result.success);
    assert!(!result.diagnostics.contains("api-id-must-not-leak"));
    assert!(!result.diagnostics.contains("api-hash-must-not-leak"));
}

#[test]
fn debug_and_core_only_release_allow_absent_identity() {
    for (profile, desktop) in [("debug", true), ("release", false)] {
        let result = configure(profile, desktop, None, None, None);
        assert!(result.success, "{}", result.diagnostics);
        let generated = result.generated.unwrap();
        assert_eq!(generated.matches("= None;").count(), 2);
    }
}

#[test]
fn debug_local_identity_and_environment_precedence_are_preserved() {
    let local = serde_json::json!({"api_id": 42, "api_hash": TEST_HASH});
    let result = configure("debug", true, None, None, Some(local.clone()));
    assert!(result.success);
    let generated = result.generated.unwrap();
    assert!(generated.contains("Some(\"42\")"));
    assert!(generated.contains(TEST_HASH));

    let result = configure("debug", true, Some("99"), None, Some(local));
    assert!(result.success);
    let generated = result.generated.unwrap();
    assert!(generated.contains("Some(\"99\")"));
    assert!(
        !generated.contains(TEST_HASH),
        "must not mix identity sources"
    );
}
