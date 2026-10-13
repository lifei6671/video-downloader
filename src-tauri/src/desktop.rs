#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use yingqu_core::{
    desktop_core::{Core, Status, Task, TaskEvent},
    settings::{ProfileUpdate, Settings},
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
fn get_profile_cookie(core: State<AppCore>, platform: String) -> Result<String, String> {
    core.cookie(&platform).map_err(error)
}
#[tauri::command]
fn get_task_cover(core: State<AppCore>, task_id: String) -> Result<Vec<u8>, String> {
    core.cover(&task_id).map_err(error)
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
async fn resolve_video(core: State<'_, AppCore>, input: String) -> Result<Task, String> {
    core.resolve(input).map_err(error)
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
    let path = task.path.filter(|p| p.is_file()).ok_or("FILE_MISSING")?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|_| "IO_ERROR".into())
}
fn main() {
    let closing = Arc::new(AtomicBool::new(false));
    let close_guard = closing.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_tasks,
            get_settings,
            get_profile_cookie,
            get_task_cover,
            save_profile,
            set_max_downloads,
            resolve_video,
            start_download,
            cancel_task,
            retry_task,
            remove_history,
            delete_local_file,
            select_download_directory,
            reveal_download
        ])
        .build(tauri::generate_context!())
        .expect("无法启动映取")
        .run(move |app, event| match event {
            tauri::RunEvent::WindowEvent {
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } => {
                api.prevent_close();
                if !close_guard.swap(true, Ordering::SeqCst) {
                    let core = app.state::<AppCore>().inner().clone();
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        core.shutdown().await;
                        handle.exit(0);
                    });
                }
            }
            tauri::RunEvent::ExitRequested { api, code, .. } if code.is_none() => {
                api.prevent_exit();
                if !closing.swap(true, Ordering::SeqCst) {
                    let core = app.state::<AppCore>().inner().clone();
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        core.shutdown().await;
                        handle.exit(0);
                    });
                }
            }
            _ => {}
        });
}
