pub fn configure() {
    use std::{env, fs, path::PathBuf};
    println!("cargo:rerun-if-env-changed=YINGQU_TELEGRAM_API_ID");
    println!("cargo:rerun-if-env-changed=YINGQU_TELEGRAM_API_HASH");
    let local = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo manifest directory"))
        .join("../telegram-app.local.json");
    println!("cargo:rerun-if-changed={}", local.display());
    let mut id = env::var("YINGQU_TELEGRAM_API_ID").ok();
    let mut hash = env::var("YINGQU_TELEGRAM_API_HASH").ok();
    // Developer input is read only for debug builds; released apps require build configuration.
    if id.is_none()
        && hash.is_none()
        && env::var("PROFILE").as_deref() == Ok("debug")
        && local.is_file()
    {
        let raw = fs::read_to_string(local).expect("cannot read local Telegram app configuration");
        let value: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|_| panic!("invalid local Telegram app configuration"));
        id = value.get("api_id").and_then(|v| {
            v.as_str()
                .map(String::from)
                .or_else(|| v.as_u64().map(|v| v.to_string()))
        });
        hash = value
            .get("api_hash")
            .and_then(|v| v.as_str().map(String::from));
    }
    let desktop_release = env::var_os("CARGO_FEATURE_DESKTOP").is_some()
        && env::var("PROFILE").as_deref() != Ok("debug");
    if desktop_release {
        let valid_id = id
            .as_deref()
            .and_then(|value| value.parse::<i32>().ok())
            .is_some_and(|value| value > 0);
        let valid_hash = hash.as_deref().is_some_and(|value| {
            value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        assert!(
            valid_id && valid_hash,
            "Telegram application identity is missing or invalid. Desktop release builds require \
             YINGQU_TELEGRAM_API_ID (positive i32) and YINGQU_TELEGRAM_API_HASH (32 ASCII hex digits). \
             telegram-app.local.json is only read by debug builds."
        );
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo output directory"))
        .join("telegram_app.rs");
    // Write private constants under the ignored build directory, never cargo:rustc-env secret values.
    let source = format!(
        "const TELEGRAM_API_ID: Option<&str> = {};\nconst TELEGRAM_API_HASH: Option<&str> = {};\n",
        literal(id),
        literal(hash)
    );
    if fs::read_to_string(&out).ok().as_deref() != Some(&source) {
        fs::write(out, source).expect("cannot write Telegram app build configuration");
    }
}
fn literal(value: Option<String>) -> String {
    value
        .map(|s| format!("Some({s:?})"))
        .unwrap_or_else(|| "None".into())
}
