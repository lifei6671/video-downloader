#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use yingqu_core::{
    desktop_core::{Core, Status, Task, TaskEvent},
    runtime_dependencies::RuntimeStatus,
    settings::{ProfileUpdate, Settings},
    update_download::{DownloadStatus, UpdateDownloads},
    updates::{self, UpdateCheck},
};

type AppCore = Arc<Core>;
fn error(error: anyhow::Error) -> String {
    error
        .to_string()
        .split(':')
        .next()
        .unwrap_or("IO_ERROR")
        .into()
}
#[tauri::command]
fn list_tasks(core: State<AppCore>) -> Vec<Task> {
    core.list()
}
#[tauri::command]
fn get_settings(core: State<AppCore>) -> Settings {
    core.settings()
}
#[tauri::command]
fn save_tdl_settings(
    core: State<AppCore>,
    config: yingqu_core::tdl_bridge::TdlSettings,
) -> Result<Settings, String> {
    core.save_tdl(config).map_err(error)
}
#[tauri::command]
async fn select_tdl_executable(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file().set_title("选择 tdl 可执行文件");
    #[cfg(windows)]
    let dialog = dialog.add_filter("可执行文件", &["exe"]);
    let dialog = if let Some(parent) = core.settings().tdl.path.as_ref().and_then(|p| p.parent()) {
        dialog.set_directory(parent)
    } else {
        dialog
    };
    dialog.pick_file(move |file| {
        let _ = tx.send(file);
    });
    rx.await
        .map_err(|_| "IO_ERROR")?
        .map(|file| {
            file.into_path()
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|_| "IO_ERROR".into())
        })
        .transpose()
}
#[tauri::command]
async fn check_tdl(
    path: Option<std::path::PathBuf>,
) -> Result<yingqu_core::tdl_bridge::TdlStatus, String> {
    yingqu_core::tdl_bridge::check(path.as_deref())
        .await
        .map_err(error)
}
#[tauri::command]
async fn login_tdl(
    core: State<'_, AppCore>,
    path: Option<std::path::PathBuf>,
) -> Result<Settings, String> {
    core.login_tdl(path).await.map_err(error)
}
#[tauri::command]
fn cancel_tdl_login(core: State<AppCore>) -> Result<(), String> {
    core.cancel_tdl_login().map_err(error)
}
#[tauri::command]
fn tdl_login_status(core: State<AppCore>) -> Option<yingqu_core::tdl_bridge::LoginStatus> {
    core.tdl_login_status()
}
#[tauri::command]
fn submit_tdl_password(
    core: State<AppCore>,
    login_id: String,
    password: String,
) -> Result<(), String> {
    core.submit_tdl_password(&login_id, zeroize::Zeroizing::new(password))
        .map_err(error)
}
#[tauri::command]
fn read_clipboard_text(app: tauri::AppHandle) -> Result<String, String> {
    app.clipboard()
        .read_text()
        .map_err(|_| "CLIPBOARD_READ_FAILED".into())
}
#[tauri::command]
fn get_profile_cookie(core: State<AppCore>, platform: String) -> Result<String, String> {
    core.cookie(&platform).map_err(error)
}
#[tauri::command]
fn get_task_cover(core: State<AppCore>, task_id: String) -> Result<Vec<u8>, String> {
    core.cover(&task_id).map_err(error)
}
#[tauri::command]
fn get_task_image(core: State<AppCore>, task_id: String, index: usize) -> Result<Vec<u8>, String> {
    core.image(&task_id, index).map_err(error)
}
#[tauri::command]
async fn save_profile(core: State<'_, AppCore>, update: ProfileUpdate) -> Result<Settings, String> {
    core.save_profile(update).map_err(error)
}
#[tauri::command]
async fn set_max_downloads(core: State<'_, AppCore>, value: u8) -> Result<Settings, String> {
    core.set_max_downloads(value).map_err(error)
}
#[tauri::command]
async fn resolve_video(
    core: State<'_, AppCore>,
    input: String,
    resolver: Option<String>,
) -> Result<Task, String> {
    core.resolve_with_resolver(input, resolver.unwrap_or_else(|| "auto".into()))
        .map_err(error)
}
#[tauri::command]
fn choose_video_format(
    core: State<AppCore>,
    task_id: String,
    format_id: String,
) -> Result<Task, String> {
    core.choose_format(&task_id, &format_id).map_err(error)
}
#[tauri::command]
async fn start_download(
    core: State<'_, AppCore>,
    task_id: String,
    confirm_duplicate: Option<bool>,
) -> Result<(), String> {
    core.download(&task_id, confirm_duplicate.unwrap_or(false))
        .map_err(error)
}
#[tauri::command]
async fn retry_task(core: State<'_, AppCore>, task_id: String) -> Result<Task, String> {
    core.retry(&task_id).map_err(error)
}
#[tauri::command]
fn remove_history(core: State<AppCore>, task_id: String) -> Result<(), String> {
    core.remove_history(&task_id).map_err(error)
}
#[tauri::command]
fn clear_download_history(core: State<AppCore>) -> Result<Vec<String>, String> {
    core.clear_history().map_err(error)
}
#[tauri::command]
fn delete_local_file(core: State<AppCore>, task_id: String) -> Result<(), String> {
    core.delete_local_file(&task_id).map_err(error)
}
#[tauri::command]
async fn cancel_task(core: State<'_, AppCore>, task_id: String) -> Result<(), String> {
    core.cancel(&task_id).await.map_err(error)
}
#[tauri::command]
async fn select_download_directory(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
) -> Result<Settings, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    if let Some(folder) = rx.await.map_err(|_| "IO_ERROR")? {
        core.set_directory(folder.into_path().map_err(|_| "IO_ERROR")?)
            .map_err(error)
    } else {
        Ok(core.settings())
    }
}
#[tauri::command]
fn reveal_download(
    app: tauri::AppHandle,
    core: State<AppCore>,
    task_id: String,
) -> Result<(), String> {
    let task = core
        .list()
        .into_iter()
        .find(|t| t.id == task_id && t.status == Status::Completed)
        .ok_or("TASK_NOT_FOUND")?;
    let path = task.path.filter(|p| p.exists()).ok_or("FILE_MISSING")?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|_| "IO_ERROR".into())
}
#[tauri::command]
fn play_download(
    app: tauri::AppHandle,
    core: State<AppCore>,
    task_id: String,
) -> Result<(), String> {
    let path = core.playback_path(&task_id).map_err(error)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| "PLAYBACK_OPEN_FAILED".into())
}
#[tauri::command]
fn open_project_page(
    app: tauri::AppHandle,
    page: String,
    release_tag: Option<String>,
) -> Result<(), String> {
    let url = match page.as_str() {
        "source" => "https://github.com/lifei6671/video-downloader",
        "issues" => "https://github.com/lifei6671/video-downloader/issues",
        "releases" => "https://github.com/lifei6671/video-downloader/releases",
        "license" => "https://github.com/lifei6671/video-downloader/blob/master/LICENSE",
        "credits" => "https://github.com/lifei6671/video-downloader/blob/master/vendor/UPSTREAM.md",
        "credit-ytdlp" => "https://github.com/yt-dlp/yt-dlp",
        "credit-dtk" => "https://github.com/Evil0ctal/Douyin_TikTok_Download_API",
        "credit-tauri" => "https://github.com/tauri-apps/tauri",
        "credit-react" => "https://github.com/react/react",
        "credit-shadcn" => "https://github.com/shadcn-ui/ui",
        "credit-lucide" => "https://github.com/lucide-icons/lucide",
        _ => return Err("INVALID_INPUT".into()),
    };
    let url = match release_tag {
        Some(tag) if page == "releases" => updates::release_url(&tag)?,
        Some(_) => return Err("INVALID_INPUT".into()),
        None => url.to_owned(),
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "OPEN_LINK_FAILED".into())
}
#[tauri::command]
async fn check_for_updates(
    downloads: State<'_, Arc<UpdateDownloads>>,
) -> Result<UpdateCheck, String> {
    downloads.check().await
}
#[tauri::command]
fn get_update_download(downloads: State<Arc<UpdateDownloads>>) -> Option<DownloadStatus> {
    downloads.status()
}
#[tauri::command]
async fn start_update_download(
    downloads: State<'_, Arc<UpdateDownloads>>,
    release_tag: String,
) -> Result<DownloadStatus, String> {
    downloads.inner().start(&release_tag)
}
#[tauri::command]
fn cancel_update_download(
    downloads: State<Arc<UpdateDownloads>>,
    download_id: String,
) -> Result<DownloadStatus, String> {
    downloads.cancel(&download_id)
}
#[tauri::command]
async fn open_update_installer(
    app: tauri::AppHandle,
    downloads: State<'_, Arc<UpdateDownloads>>,
    download_id: String,
) -> Result<(), String> {
    let path = downloads.installer_path(&download_id).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| "UPDATE_INSTALLER_OPEN_FAILED".into())
}
#[tauri::command]
async fn check_runtime_dependencies(core: State<'_, AppCore>) -> Result<RuntimeStatus, String> {
    let result = core.dependencies(None, |_| {}).await.map_err(error)?;
    serde_json::from_value(result).map_err(|_| "ENGINE_PROTOCOL_ERROR".into())
}
#[tauri::command]
async fn install_runtime_dependency(
    core: State<'_, AppCore>,
    tool: String,
    on_progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<RuntimeStatus, String> {
    let result = core
        .dependencies(Some(tool), move |value| {
            let _ = on_progress.send(value);
        })
        .await
        .map_err(error)?;
    serde_json::from_value(result).map_err(|_| "ENGINE_PROTOCOL_ERROR".into())
}
#[tauri::command]
fn cancel_runtime_dependency(core: State<AppCore>) -> Result<(), String> {
    core.cancel_dependency().map_err(error)
}
fn show_main_window(app: &tauri::AppHandle) {
    if app.state::<AtomicBool>().load(Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
        {
            eprintln!("Unable to show main window: {error}");
        }
    }
}

fn request_exit(app: &tauri::AppHandle) {
    if app.state::<AtomicBool>().swap(true, Ordering::SeqCst) {
        return;
    }
    let core = app.state::<AppCore>().inner().clone();
    let updates = app.state::<Arc<UpdateDownloads>>().inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        updates.shutdown().await;
        core.shutdown().await;
        handle.exit(0);
    });
}

fn main() {
    tauri::Builder::default()
        .manage(AtomicBool::new(false))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            // Preserve the existing database when adopting the new bundle identifier.
            let legacy = data.with_file_name("com.yingqu.desktop");
            if !data.exists() && legacy.join("yingqu.sqlite").is_file() {
                std::fs::rename(&legacy, &data)?;
            }
            std::fs::create_dir_all(&data)?;
            let update_handle = app.handle().clone();
            app.manage(UpdateDownloads::new(
                data.join("updates"),
                app.package_info().version.to_string(),
                move |status| {
                    let _ = update_handle.emit("update_download", status);
                },
            ));
            let downloads = app.path().download_dir()?.join("Yingqu");
            #[cfg(debug_assertions)]
            let engine_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("python-engine/dist/video-engine");
            #[cfg(not(debug_assertions))]
            let engine_dir = app.path().resource_dir()?.join("video-engine");
            let engine = engine_dir.join(if cfg!(windows) {
                "video-engine.exe"
            } else {
                "video-engine"
            });
            let handle = app.handle().clone();
            let core = Core::open(
                &data.join("yingqu.sqlite"),
                downloads,
                engine.to_string_lossy().into_owned(),
                vec![],
                move |event| match event {
                    TaskEvent::Updated(task) => {
                        let _ = handle.emit("task_updated", task);
                    }
                    TaskEvent::Removed(id) => {
                        let _ = handle.emit("task_removed", id);
                    }
                },
            )?;
            app.manage(core);
            let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出映取", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(
                    app.default_window_icon()
                        .expect("App icon is configured")
                        .clone(),
                )
                .tooltip("映取")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => request_exit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_tasks,
            get_settings,
            save_tdl_settings,
            select_tdl_executable,
            login_tdl,
            cancel_tdl_login,
            tdl_login_status,
            submit_tdl_password,
            check_tdl,
            read_clipboard_text,
            get_profile_cookie,
            get_task_cover,
            get_task_image,
            save_profile,
            set_max_downloads,
            resolve_video,
            start_download,
            choose_video_format,
            cancel_task,
            retry_task,
            remove_history,
            clear_download_history,
            delete_local_file,
            select_download_directory,
            reveal_download,
            play_download,
            open_project_page,
            check_for_updates,
            get_update_download,
            start_update_download,
            cancel_update_download,
            open_update_installer,
            check_runtime_dependencies,
            install_runtime_dependency,
            cancel_runtime_dependency
        ])
        .build(tauri::generate_context!())
        .expect("无法启动映取")
        .run(|app, event| match event {
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                api.prevent_close();
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(error) = window.hide() {
                        eprintln!("Unable to hide main window: {error}");
                    }
                }
            }
            tauri::RunEvent::ExitRequested { api, code, .. } if code.is_none() => {
                api.prevent_exit();
                request_exit(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(app),
            _ => {}
        });
}
