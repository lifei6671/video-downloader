#[path = "build_support/telegram_app.rs"]
mod telegram_app;

fn main() {
    telegram_app::configure();
    #[cfg(feature = "desktop")]
    tauri_build::build();
}
