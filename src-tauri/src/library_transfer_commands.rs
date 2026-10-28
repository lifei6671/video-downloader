use super::{error, AppCore};
use std::path::PathBuf;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use yingqu_core::{
    desktop_core::library_transfer_operation::{BackupInfo, TransferSnapshot},
    library::{ContentFormat, ContentSelection},
};

async fn directory(
    app: &tauri::AppHandle,
    title: &str,
    initial: PathBuf,
) -> Result<Option<PathBuf>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(title)
        .set_directory(initial)
        .pick_folder(move |file| {
            let _ = tx.send(file);
        });
    rx.await
        .map_err(|_| "IO_ERROR")?
        .map(|file| file.into_path().map_err(|_| "IO_ERROR".into()))
        .transpose()
}
fn output_directory(parent: PathBuf, prefix: &str) -> PathBuf {
    parent.join(format!(
        "{prefix}-{}-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().to_string()[..8]
    ))
}
#[tauri::command(async)]
pub fn get_library_backup_info(core: State<AppCore>) -> Result<BackupInfo, String> {
    core.library_backup_info().map_err(error)
}
#[tauri::command]
pub fn get_library_transfer(core: State<AppCore>) -> Option<TransferSnapshot> {
    core.library_transfer()
}
#[tauri::command]
pub async fn start_library_backup(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    include_media: bool,
) -> Result<Option<TransferSnapshot>, String> {
    let Some(parent) =
        directory(&app, "选择备份保存位置", core.settings().download_directory).await?
    else {
        return Ok(None);
    };
    core.start_library_backup(output_directory(parent, "映取资料备份"), include_media)
        .map(Some)
        .map_err(error)
}
#[tauri::command]
pub async fn start_library_restore(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
) -> Result<Option<TransferSnapshot>, String> {
    if core
        .library_transfer()
        .is_some_and(|a| a.state == "running")
    {
        return Err("LIBRARY_TRANSFER_BUSY".into());
    }
    let info = core.library_backup_info().map_err(error)?;
    if !info.empty {
        return Err("LIBRARY_NOT_EMPTY".into());
    }
    if info.busy {
        return Err("LIBRARY_BUSY".into());
    }
    let Some(package) =
        directory(&app, "选择知识库备份包", core.settings().download_directory).await?
    else {
        return Ok(None);
    };
    if core
        .library_transfer()
        .is_some_and(|a| a.state == "running")
    {
        return Err("LIBRARY_TRANSFER_BUSY".into());
    }
    let Some(parent) = directory(
        &app,
        "选择恢复文件保存位置",
        core.settings().download_directory,
    )
    .await?
    else {
        return Ok(None);
    };
    core.start_library_restore(package, output_directory(parent, "映取恢复"))
        .map(Some)
        .map_err(error)
}
#[tauri::command]
pub fn cancel_library_transfer(core: State<AppCore>, operation_id: String) -> Result<(), String> {
    core.cancel_library_transfer(&operation_id).map_err(error)
}
#[tauri::command]
pub async fn export_library_content(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    asset_id: String,
    selection: ContentSelection,
    format: ContentFormat,
) -> Result<Option<String>, String> {
    let (title, version) = match &selection {
        ContentSelection::Document(id) => {
            let summary = core
                .library_documents(&asset_id)
                .map_err(error)?
                .into_iter()
                .find(|d| &d.id == id)
                .ok_or("DOCUMENT_NOT_FOUND")?;
            (
                if summary.kind == "recognized" {
                    "识别原文"
                } else {
                    "整理稿"
                },
                summary.version,
            )
        }
        ContentSelection::Note(id) => {
            let note = core.library_note(&asset_id, id).map_err(error)?;
            ("学习笔记", note.revision)
        }
    };
    let (label, ext) = match format {
        ContentFormat::Txt => ("文本文件", "txt"),
        ContentFormat::Markdown => ("Markdown", "md"),
        ContentFormat::Json => ("JSON", "json"),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导出已保存内容")
        .set_directory(core.settings().download_directory)
        .set_file_name(format!("{title}-v{version}.{ext}"))
        .add_filter(label, &[ext])
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|_| "IO_ERROR")? else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "IO_ERROR")?;
    let output = path.to_string_lossy().into_owned();
    let core = core.inner().clone();
    tokio::task::spawn_blocking(move || {
        core.export_library_content(&asset_id, &selection, format, &path)
    })
    .await
    .map_err(|_| "IO_ERROR")?
    .map_err(error)?;
    Ok(Some(output))
}
