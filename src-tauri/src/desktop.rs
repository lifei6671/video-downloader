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
#[tauri::command]
async fn get_mcp_connection(
    core: State<'_, AppCore>,
) -> Result<yingqu_core::mcp_connection::ConnectionInfo, String> {
    let mut info = yingqu_core::mcp_connection::connection_info(
        core.mcp_ipc_status().await,
        &std::env::current_exe().map_err(|_| "IO_ERROR")?,
    )
    .map_err(error)?;
    info.write_enabled = info.status.enabled && core.mcp_write_enabled();
    Ok(info)
}
#[tauri::command]
async fn set_mcp_write_enabled(core: State<'_, AppCore>, enabled: bool) -> Result<yingqu_core::mcp_connection::ConnectionInfo, String> {
    core.set_mcp_write_enabled(enabled).await.map_err(error)?;
    get_mcp_connection(core).await
}
#[tauri::command]
async fn set_mcp_service_enabled(
    core: State<'_, AppCore>,
    enabled: bool,
) -> Result<yingqu_core::mcp_connection::ConnectionInfo, String> {
    let executable = std::env::current_exe().map_err(|_| "IO_ERROR")?;
    if enabled {
        yingqu_core::mcp_connection::adapter_path(&executable).map_err(error)?;
    }
    core
        .inner()
        .set_mcp_ipc_enabled(enabled)
        .await
        .map_err(error)?;
    get_mcp_connection(core).await
}
#[tauri::command]
async fn copy_mcp_client_config(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    format: yingqu_core::mcp_connection::ConfigFormat,
) -> Result<(), String> {
    let info = get_mcp_connection(core).await?;
    app.clipboard()
        .write_text(info.config(format).map_err(error)?)
        .map_err(|_| "CLIPBOARD_WRITE_FAILED".into())
}
#[tauri::command]
async fn get_mcp_ipc_status(core: State<'_, AppCore>) -> Result<yingqu_core::core_ipc::IpcStatus, String> {
    Ok(core.mcp_ipc_status().await)
}
#[tauri::command]
async fn set_mcp_ipc_enabled(core: State<'_, AppCore>, enabled: bool) -> Result<yingqu_core::core_ipc::IpcStatus, String> {
    core.inner().set_mcp_ipc_enabled(enabled).await.map_err(error)
}
mod library_commands;
mod library_transfer_commands;
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
fn list_video_transcripts(
    core: State<AppCore>,
) -> Result<Vec<yingqu_core::desktop_core::transcription::Snapshot>, String> {
    core.transcripts().map_err(error)
}
#[tauri::command]
fn get_video_transcript(
    core: State<AppCore>,
    task_id: String,
) -> Result<Option<yingqu_core::desktop_core::transcription::Snapshot>, String> {
    core.transcript(&task_id).map_err(error)
}
#[tauri::command]
async fn start_video_transcription(
    core: State<'_, AppCore>,
    task_id: String,
    force: Option<bool>,
) -> Result<yingqu_core::desktop_core::transcription::Snapshot, String> {
    core.start_transcription(&task_id, force.unwrap_or(false))
        .map_err(error)
}
#[tauri::command]
async fn cancel_video_transcription(
    core: State<'_, AppCore>,
    task_id: String,
    operation_id: String,
) -> Result<(), String> {
    core.cancel_transcription(&task_id, &operation_id)
        .await
        .map_err(error)
}
#[tauri::command]
fn copy_video_transcript(
    app: tauri::AppHandle,
    core: State<AppCore>,
    task_id: String,
) -> Result<(), String> {
    let result = core.transcript_text(&task_id).map_err(error)?;
    app.clipboard()
        .write_text(result.text)
        .map_err(|_| "CLIPBOARD_WRITE_FAILED".into())
}
#[tauri::command]
async fn export_video_transcript(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    task_id: String,
) -> Result<Option<String>, String> {
    let result = core.transcript_text(&task_id).map_err(error)?;
    let directory = result
        .source
        .path
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| p.to_path_buf())
        .unwrap_or(core.settings().download_directory);
    let filename = result
        .source
        .path
        .file_stem()
        .ok_or("IO_ERROR")?
        .to_string_lossy()
        .into_owned()
        + ".txt";
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导出视频文案")
        .set_directory(directory)
        .set_file_name(filename)
        .add_filter("文本文件", &["txt"])
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|_| "IO_ERROR")? else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "IO_ERROR")?;
    yingqu_core::desktop_core::transcription::export_txt(&result, &path).map_err(error)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
#[tauri::command]
fn get_settings(core: State<AppCore>) -> Settings {
    core.settings()
}
#[tauri::command]
async fn import_library_video(app: tauri::AppHandle, core: State<'_, AppCore>) -> Result<Option<yingqu_core::library::ImportedVideo>, String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    app.dialog().file().set_title("导入视频").add_filter("视频文件", &["mp4","mov","mkv","webm"]).pick_file(move |file| { let _=tx.send(file); });
    let Some(file)=rx.await.map_err(|_|"IO_ERROR")? else { return Ok(None); };
    let path=file.into_path().map_err(|_|"IO_ERROR")?;
    core.import_local_video(path).await.map(Some).map_err(error)
}
#[tauri::command]
async fn cancel_library_import(core: State<'_, AppCore>) -> Result<(), String> {
    core.cancel_local_import().await;
    Ok(())
}
#[tauri::command]
async fn relink_library_file(app: tauri::AppHandle,core:State<'_,AppCore>,asset_id:String,file_id:String)->Result<Option<yingqu_core::library::ImportedVideo>,String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    app.dialog().file().set_title("重新关联视频").add_filter("视频文件", &["mp4","mov","mkv","webm"]).pick_file(move |file| {let _=tx.send(file);});
    let Some(file)=rx.await.map_err(|_|"IO_ERROR")? else {return Ok(None);};
    core.relink_local_video(asset_id,file_id,file.into_path().map_err(|_|"IO_ERROR")?).await.map(Some).map_err(error)
}
#[tauri::command]
fn get_asset_transcript(core: State<AppCore>, asset_id: String, file_id: String) -> Result<Option<yingqu_core::desktop_core::transcription::Snapshot>,String> {
    core.asset_transcript(&asset_id,&file_id).map_err(error)
}
#[tauri::command]
async fn start_asset_transcription(core: State<'_, AppCore>, asset_id: String, file_id: String, force: Option<bool>) -> Result<yingqu_core::desktop_core::transcription::Snapshot,String> {
    core.start_asset_transcription(&asset_id,&file_id,force.unwrap_or(false)).map_err(error)
}
#[tauri::command]
async fn cancel_asset_transcription(core: State<'_, AppCore>, file_id: String, operation_id: String) -> Result<(),String> {
    core.cancel_asset_transcription(&file_id,&operation_id).await.map_err(error)
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
fn clipboard_image(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<tauri::image::Image<'static>, String> {
    if width == 0
        || height == 0
        || (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            != Some(rgba.len())
    {
        return Err("INVALID_MEDIA_CONTENT".into());
    }
    Ok(tauri::image::Image::new_owned(rgba, width, height))
}

#[cfg(test)]
mod clipboard_image_tests {
    use super::clipboard_image;

    #[test]
    fn rgba_pixels_dimensions_and_transparency_are_preserved() {
        let pixels = vec![255, 0, 0, 255, 0, 100, 200, 128];
        let image = clipboard_image(pixels.clone(), 2, 1).unwrap();
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 1);
        assert_eq!(image.rgba(), pixels);
    }

    #[test]
    fn invalid_dimensions_lengths_and_overflow_are_rejected() {
        for (pixels, width, height) in [
            (vec![], 0, 1),
            (vec![], 1, 0),
            (vec![0; 3], 1, 1),
            (vec![0; 5], 1, 1),
            (vec![], u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                clipboard_image(pixels, width, height).unwrap_err(),
                "INVALID_MEDIA_CONTENT"
            );
        }
    }
}
#[tauri::command]
async fn copy_cover_image(
    app: tauri::AppHandle,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let image = clipboard_image(rgba, width, height)?;
    app.clipboard()
        .write_image(&image)
        .map_err(|_| "CLIPBOARD_WRITE_FAILED".into())
}
#[tauri::command]
fn get_profile_cookie(core: State<AppCore>, platform: String) -> Result<String, String> {
    core.cookie(&platform).map_err(error)
}
#[tauri::command]
fn get_profile_proxy(core: State<AppCore>, platform: String) -> Result<Option<String>, String> {
    core.proxy(&platform).map_err(error)
}
#[tauri::command]
fn get_task_cover(core: State<AppCore>, task_id: String) -> Result<Vec<u8>, String> {
    core.cover(&task_id).map_err(error)
}
#[tauri::command]
fn get_author_avatar(core: State<AppCore>, task_id: String) -> Result<Vec<u8>, String> {
    core.author_avatar(&task_id).map_err(error)
}
#[tauri::command]
async fn save_task_cover(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    task_id: String,
) -> Result<bool, String> {
    let (filename, body) = core.export_cover(&task_id).map_err(error)?;
    let extension = filename.rsplit('.').next().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("保存封面")
        .set_directory(core.settings().download_directory)
        .set_file_name(&filename)
        .add_filter("封面图片", &[extension])
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|_| "IO_ERROR")? else {
        return Ok(false);
    };
    let path = file.into_path().map_err(|_| "IO_ERROR")?;
    std::fs::write(path, body).map_err(|_| "IO_ERROR")?;
    Ok(true)
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
fn set_filename_template(core: State<AppCore>, template: String) -> Result<Settings, String> {
    core.set_filename_template(template).map_err(error)
}
#[tauri::command]
fn preview_filename(template: String) -> Result<yingqu_core::naming::FilenamePreview, String> {
    yingqu_core::naming::preview(&template).map_err(error)
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
async fn remove_history(core: State<'_, AppCore>, task_id: String) -> Result<(), String> {
    core.remove_after_transcription(&task_id, false)
        .await
        .map_err(error)
}
#[tauri::command]
fn clear_download_history(
    core: State<AppCore>,
    platform: Option<String>,
) -> Result<Vec<String>, String> {
    core.clear_history(platform.as_deref()).map_err(error)
}
#[tauri::command]
async fn delete_local_file(core: State<'_, AppCore>, task_id: String) -> Result<(), String> {
    core.remove_after_transcription(&task_id, true)
        .await
        .map_err(error)
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
fn prepare_video_playback(
    app: tauri::AppHandle,
    core: State<AppCore>,
    task_id: String,
) -> Result<String, String> {
    let path = core.playback_path(&task_id).map_err(error)?;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|_| "PLAYBACK_SOURCE_FAILED".to_string())?;
    Ok(path.to_string_lossy().into_owned())
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
    #[cfg(windows)]
    {
        let _ = app;
        launch_windows_installer(&path)
            .map(|_| ())
            .map_err(|_| "UPDATE_INSTALLER_OPEN_FAILED".into())
    }
    #[cfg(not(windows))]
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| "UPDATE_INSTALLER_OPEN_FAILED".into())
}

#[cfg(windows)]
fn launch_windows_installer(path: &std::path::Path) -> std::io::Result<std::process::Child> {
    use std::{os::windows::process::CommandExt, process::Stdio};
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};

    // The per-user NSIS installer is an executable, not a shell document. Avoid
    // ShellExecuteEx/COM on the async worker and let the installer outlive the app.
    std::process::Command::new(path)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
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
    variant: Option<String>,
    on_progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<RuntimeStatus, String> {
    let result = core
        .dependencies_variant(Some(tool), variant, move |value| {
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
    run(tauri::Builder::default(), context());
}

// Share one macro expansion with the isolated acceptance entry point. On macOS
// each expansion embeds an Info.plist symbol, so two expansions cannot link.
pub fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub(crate) fn run(builder: tauri::Builder<tauri::Wry>, context: tauri::Context<tauri::Wry>) {
    builder
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
                    TaskEvent::Transcription(snapshot) => {
                        let _ = handle.emit("transcription_updated", snapshot);
                    }
                    TaskEvent::AssetTranscription(snapshot) => {
                        let _ = handle.emit("asset_transcription_updated", snapshot);
                    }
                    TaskEvent::Keyframes(snapshot) => { let _=handle.emit("library_keyframes_updated",snapshot); }
                    TaskEvent::LibraryTransfer(snapshot) => { let _ = handle.emit("library_transfer_updated", snapshot); }
                    TaskEvent::LibraryChanged(change) => {
                        let _=handle.emit("library_changed",change);
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
            get_mcp_ipc_status,
            get_mcp_connection,
            set_mcp_service_enabled,
            set_mcp_write_enabled,
            copy_mcp_client_config,
            set_mcp_ipc_enabled,
            library_commands::search_library_assets,
            library_commands::get_workbench,
            library_transfer_commands::get_library_backup_info,
            library_transfer_commands::get_library_transfer,
            library_transfer_commands::start_library_backup,
            library_transfer_commands::start_library_restore,
            library_transfer_commands::cancel_library_transfer,
            library_transfer_commands::export_library_content,
            library_commands::record_library_visit,
            library_commands::get_library_asset,
            library_commands::archive_library_asset,
            library_commands::list_library_categories,
            library_commands::create_library_category,
            library_commands::update_library_category,
            library_commands::delete_library_category,
            library_commands::set_library_categories,
            library_commands::list_library_tags,
            library_commands::create_library_tag,
            library_commands::rename_library_tag,
            library_commands::delete_library_tag,
            library_commands::set_library_tags,
            library_commands::list_library_documents,
            library_commands::list_library_keyframes,
            library_commands::get_library_keyframe_image,
            library_commands::delete_library_keyframe,
            library_commands::get_library_keyframe_operation,
            library_commands::start_library_keyframes,
            library_commands::cancel_library_keyframes,
            library_commands::list_library_clips,
            library_commands::create_library_clip,
            library_commands::delete_library_clip,
            library_commands::prepare_library_clip,
            library_commands::get_library_document,
            library_commands::save_library_document,
            library_commands::list_library_ideas,
            library_commands::get_library_idea,
            library_commands::prepare_library_idea_reference,
            library_commands::create_library_idea,
            library_commands::edit_library_idea,
            library_commands::list_library_topics,
            library_commands::get_library_topic,
            library_commands::create_library_topic,
            library_commands::edit_library_topic,
            library_commands::add_library_topic_member,
            library_commands::remove_library_topic_member,
            library_commands::reorder_library_topic_members,
            library_commands::list_library_notes,
            library_commands::get_library_note,
            library_commands::create_library_note,
            library_commands::get_library_comparison_source,
            library_commands::create_library_comparison,
            library_commands::edit_library_comparison,
            library_commands::create_library_analysis,
            library_commands::edit_library_analysis,
            library_commands::edit_library_note,
            library_commands::get_library_cover,
            library_commands::prepare_library_playback,
            library_commands::reveal_library_file,
            library_commands::open_library_file,
            library_commands::export_library_document,
            import_library_video,
            cancel_library_import,
            relink_library_file,
            get_asset_transcript,
            start_asset_transcription,
            cancel_asset_transcription,
            list_video_transcripts,
            get_video_transcript,
            start_video_transcription,
            cancel_video_transcription,
            copy_video_transcript,
            export_video_transcript,
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
            copy_cover_image,
            get_profile_cookie,
            get_profile_proxy,
            get_task_cover,
            get_author_avatar,
            save_task_cover,
            get_task_image,
            save_profile,
            set_max_downloads,
            set_filename_template,
            preview_filename,
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
            prepare_video_playback,
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
        .build(context)
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
            // Cocoa's native Quit / Cmd-Q emits Exit without ExitRequested.
            // Finish the same cleanup before the native termination returns.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Exit if !app.state::<AtomicBool>().swap(true, Ordering::SeqCst) => {
                tauri::async_runtime::block_on(async {
                    app.state::<Arc<UpdateDownloads>>().shutdown().await;
                    app.state::<AppCore>().shutdown().await;
                });
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(app),
            _ => {}
        });
}

#[cfg(all(test, windows))]
mod update_installer_tests {
    use super::launch_windows_installer;
    use std::time::{Duration, Instant};

    #[test]
    fn launches_executable_in_literal_unicode_path_without_blocking_or_exiting_host() {
        let directory = tempfile::Builder::new()
            .prefix("更新 安装包 & (test)-")
            .tempdir()
            .unwrap();
        let path = directory.path().join("安装包 & (test).exe");
        let system_root = std::env::var_os("SystemRoot").unwrap();
        std::fs::copy(
            std::path::Path::new(&system_root).join("System32/cmd.exe"),
            &path,
        )
        .unwrap();

        // With null stdin, this benign executable exits without user interaction.
        // No shell should interpret the spaces or metacharacters in its path.
        let mut child = launch_windows_installer(&path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "fixture failed: {status}");
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("fixture did not exit after receiving EOF");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn missing_or_invalid_executables_report_launch_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("installer.exe");
        assert!(launch_windows_installer(&path).is_err());
        std::fs::write(&path, b"not a Windows executable").unwrap();
        assert!(launch_windows_installer(&path).is_err());
    }
}
