// Exercise the production entry point with a compile-time isolated Tauri
// identifier, supplied by scripts/p1a-acceptance.ps1 or p1a-acceptance-macos.py.
// No mock IPC or alternate UI.
#[path = "../src/desktop.rs"]
#[expect(
    dead_code,
    reason = "the production main is replaced by this isolated entry point"
)]
mod desktop;

fn main() {
    use tauri::Manager;
    let context = desktop::context();
    let identifier = context.config().identifier.clone();
    let isolated = option_env!("TAURI_CONFIG")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value["identifier"].as_str().map(str::to_owned))
        .is_some_and(|id| id == identifier && id.starts_with("me.disign.yingqu.p1a-"));
    if !isolated {
        eprintln!("Use scripts/p1a-acceptance.ps1 or scripts/p1a-acceptance-macos.py to build an isolated acceptance profile");
        std::process::exit(2);
    }
    let guard = tauri::plugin::Builder::<tauri::Wry>::new("p1a-isolation")
        .setup(move |app, _| {
            // Plugins initialize before the production setup/legacy migration.
            let data = app.path().app_data_dir()?;
            let marker: serde_json::Value =
                serde_json::from_slice(&std::fs::read(data.join("p1a-acceptance.json"))?)?;
            if marker["identifier"].as_str() != Some(identifier.as_str()) {
                return Err("Acceptance profile marker does not match".into());
            }
            Ok(())
        })
        .build();
    desktop::run(tauri::Builder::default().plugin(guard), context);
}
