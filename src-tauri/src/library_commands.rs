use super::{error, AppCore};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use yingqu_core::{desktop_core::library_api::NoteInput, library::*};

#[tauri::command(async)]
pub fn list_library_keyframes(core: State<AppCore>, asset_id: String) -> Result<Vec<Keyframe>,String> {
    core.library_keyframes(&asset_id).map_err(error)
}
#[tauri::command(async)]
pub fn get_library_keyframe_image(core: State<AppCore>, asset_id: String, keyframe_id: String) -> Result<tauri::ipc::Response,String> {
    core.library_keyframe_image(&asset_id,&keyframe_id).map(tauri::ipc::Response::new).map_err(error)
}
#[tauri::command]
pub fn delete_library_keyframe(core: State<AppCore>, asset_id: String, keyframe_id: String) -> Result<(),String> {
    core.delete_library_keyframe(&asset_id,&keyframe_id).map_err(error)
}
#[tauri::command]
pub fn get_library_keyframe_operation(core: State<AppCore>) -> Option<yingqu_core::desktop_core::keyframes::KeyframeSnapshot> { core.keyframe_operation() }
#[tauri::command]
pub async fn start_library_keyframes(core: State<'_,AppCore>, asset_id: String, request: yingqu_core::desktop_core::keyframes::KeyframeRequest) -> Result<yingqu_core::desktop_core::keyframes::KeyframeSnapshot,String> { core.inner().start_keyframes(&asset_id,request).map_err(error) }
#[tauri::command]
pub async fn cancel_library_keyframes(core: State<'_,AppCore>, operation_id: String) -> Result<(),String> { core.cancel_keyframes(&operation_id).await.map_err(error) }

#[tauri::command]
pub fn list_library_clips(core: State<AppCore>, asset_id: String) -> Result<Vec<Clip>,String> {
    core.library_clips(&asset_id).map_err(error)
}
#[tauri::command]
pub fn create_library_clip(core: State<AppCore>, asset_id: String, input: ClipInput) -> Result<String,String> {
    core.create_library_clip(&asset_id,&input).map_err(error)
}
#[tauri::command]
pub fn delete_library_clip(core: State<AppCore>, asset_id: String, clip_id: String) -> Result<(),String> {
    core.delete_library_clip(&asset_id,&clip_id).map_err(error)
}
#[derive(serde::Serialize)]
pub struct ClipPlayback { clip: Clip, path: String }
#[tauri::command]
pub fn prepare_library_clip(app: tauri::AppHandle, core: State<AppCore>, asset_id: String, clip_id: String) -> Result<ClipPlayback,String> {
    let (clip,path)=core.library_clip_playback(&asset_id,&clip_id).map_err(error)?;
    app.asset_protocol_scope().allow_file(&path).map_err(|_|"PLAYBACK_SOURCE_FAILED".to_string())?;
    Ok(ClipPlayback {clip,path:path.to_string_lossy().into_owned()})
}

#[tauri::command(async)]
pub fn search_library_assets(
    core: State<AppCore>,
    request: SearchRequest,
) -> Result<SearchPage, String> {
    core.search_library(request).map_err(error)
}
#[tauri::command]
pub fn get_library_asset(core: State<AppCore>, asset_id: String) -> Result<AssetDetail, String> {
    core.library_asset(&asset_id).map_err(error)
}
#[tauri::command]
pub fn archive_library_asset(
    core: State<AppCore>,
    asset_id: String,
    archived: bool,
) -> Result<(), String> {
    core.archive_library_asset(&asset_id, archived)
        .map_err(error)
}
#[tauri::command(async)]
pub fn get_task_collection_states(core: State<AppCore>, task_ids: Vec<String>) -> Result<Vec<TaskCollectionState>, String> {
    core.task_collection_states(&task_ids).map_err(error)
}
#[tauri::command]
pub fn set_task_collected(core: State<AppCore>, task_id: String, collected: bool) -> Result<TaskCollectionState, String> {
    core.set_task_collected(&task_id, collected).map_err(error)
}
#[tauri::command]
pub fn list_library_categories(core: State<AppCore>) -> Result<Vec<Category>, String> {
    core.library_categories().map_err(error)
}
#[tauri::command]
pub fn create_library_category(
    core: State<AppCore>,
    parent_id: Option<String>,
    name: String,
) -> Result<String, String> {
    core.create_library_category(parent_id.as_deref(), &name)
        .map_err(error)
}
#[tauri::command]
pub fn update_library_category(
    core: State<AppCore>,
    category_id: String,
    parent_id: Option<String>,
    name: String,
    position: i64,
) -> Result<(), String> {
    core.update_library_category(&category_id, parent_id.as_deref(), &name, position)
        .map_err(error)
}
#[tauri::command]
pub fn delete_library_category(core: State<AppCore>, category_id: String) -> Result<(), String> {
    core.delete_library_category(&category_id).map_err(error)
}
#[tauri::command]
pub fn set_library_categories(
    core: State<AppCore>,
    asset_ids: Vec<String>,
    category_id: String,
    add: bool,
) -> Result<(), String> {
    core.set_library_categories(&asset_ids, &category_id, add)
        .map_err(error)
}
#[tauri::command]
pub fn list_library_tags(core: State<AppCore>) -> Result<Vec<Tag>, String> {
    core.library_tags().map_err(error)
}
#[tauri::command]
pub fn create_library_tag(core: State<AppCore>, name: String) -> Result<String, String> {
    core.create_library_tag(&name).map_err(error)
}
#[tauri::command]
pub fn rename_library_tag(
    core: State<AppCore>,
    tag_id: String,
    name: String,
) -> Result<(), String> {
    core.rename_library_tag(&tag_id, &name).map_err(error)
}
#[tauri::command]
pub fn delete_library_tag(core: State<AppCore>, tag_id: String) -> Result<(), String> {
    core.delete_library_tag(&tag_id).map_err(error)
}
#[tauri::command]
pub fn set_library_tags(
    core: State<AppCore>,
    asset_ids: Vec<String>,
    tag_id: String,
    add: bool,
) -> Result<(), String> {
    core.set_library_tags(&asset_ids, &tag_id, add)
        .map_err(error)
}
#[tauri::command]
pub fn list_library_documents(
    core: State<AppCore>,
    asset_id: String,
) -> Result<Vec<DocumentSummary>, String> {
    core.library_documents(&asset_id).map_err(error)
}
#[tauri::command]
pub fn get_library_document(
    core: State<AppCore>,
    asset_id: String,
    document_id: String,
) -> Result<DocumentContent, String> {
    core.library_document(&asset_id, &document_id)
        .map_err(error)
}
#[tauri::command]
pub fn save_library_document(
    core: State<AppCore>,
    asset_id: String,
    text: String,
    expected_version: u64,
    source_document_id: Option<String>,
) -> Result<String, String> {
    core.save_library_document(
        &asset_id,
        &text,
        expected_version,
        source_document_id.as_deref(),
    )
    .map_err(error)
}
#[tauri::command]
pub fn list_library_ideas(core:State<AppCore>)->Result<Vec<IdeaSummary>,String> { core.library_ideas().map_err(error) }
#[tauri::command]
pub fn get_library_idea(core:State<AppCore>,idea_id:String)->Result<IdeaDetail,String> { core.library_idea(&idea_id).map_err(error) }
#[tauri::command]
pub fn prepare_library_idea_reference(core:State<AppCore>,input:IdeaReferenceInput)->Result<IdeaReferenceView,String> { core.library_idea_reference(&input).map_err(error) }
#[tauri::command]
pub fn create_library_idea(core:State<AppCore>,input:IdeaInput)->Result<String,String> { core.create_library_idea(input).map_err(error) }
#[tauri::command]
pub fn edit_library_idea(core:State<AppCore>,idea_id:String,input:IdeaInput,expected_revision:u64)->Result<(),String> { core.edit_library_idea(&idea_id,input,expected_revision).map_err(error) }
#[tauri::command]
pub fn list_library_topics(core:State<AppCore>)->Result<Vec<ResearchTopic>,String> { core.library_topics().map_err(error) }
#[tauri::command]
pub fn get_library_topic(core:State<AppCore>,topic_id:String)->Result<ResearchTopicDetail,String> { core.library_topic(&topic_id).map_err(error) }
#[tauri::command]
pub fn create_library_topic(core:State<AppCore>,input:TopicInput)->Result<String,String> { core.create_library_topic(input).map_err(error) }
#[tauri::command]
pub fn edit_library_topic(core:State<AppCore>,topic_id:String,input:TopicInput,expected_revision:u64)->Result<(),String> { core.edit_library_topic(&topic_id,input,expected_revision).map_err(error) }
#[tauri::command]
pub fn add_library_topic_member(core:State<AppCore>,topic_id:String,input:TopicMemberInput,expected_revision:u64)->Result<String,String> { core.add_library_topic_member(&topic_id,input,expected_revision).map_err(error) }
#[tauri::command]
pub fn remove_library_topic_member(core:State<AppCore>,topic_id:String,member_id:String,expected_revision:u64)->Result<(),String> { core.remove_library_topic_member(&topic_id,&member_id,expected_revision).map_err(error) }
#[tauri::command]
pub fn reorder_library_topic_members(core:State<AppCore>,topic_id:String,member_ids:Vec<String>,expected_revision:u64)->Result<(),String> { core.reorder_library_topic_members(&topic_id,member_ids,expected_revision).map_err(error) }
#[tauri::command]
pub fn list_library_notes(
    core: State<AppCore>,
    asset_id: String,
) -> Result<Vec<NoteSummary>, String> {
    core.library_notes(&asset_id).map_err(error)
}
#[tauri::command]
pub fn get_library_note(
    core: State<AppCore>,
    asset_id: String,
    note_id: String,
) -> Result<Note, String> {
    core.library_note(&asset_id, &note_id).map_err(error)
}
#[tauri::command]
pub fn get_library_comparison_source(core: State<AppCore>, pin: ComparisonPin) -> Result<ComparisonSource,String> { core.library_comparison_source(&pin).map_err(error) }
#[tauri::command]
pub fn create_library_comparison(core: State<AppCore>, asset_id: String, input: NewComparison) -> Result<String,String> { core.create_library_comparison(&asset_id,input).map_err(error) }
#[tauri::command]
pub fn edit_library_comparison(core: State<AppCore>, asset_id: String, note_id: String, input: ComparisonEdit, expected_revision: u64) -> Result<(),String> { core.edit_library_comparison(&asset_id,&note_id,input,expected_revision).map_err(error) }
#[tauri::command]
pub fn create_library_analysis(core: State<AppCore>, asset_id: String, input: NewAnalysis) -> Result<String,String> { core.create_library_analysis(&asset_id,input).map_err(error) }
#[tauri::command]
pub fn edit_library_analysis(core: State<AppCore>, asset_id: String, note_id: String, content: AnalysisInput, expected_revision: u64) -> Result<(),String> { core.edit_library_analysis(&asset_id,&note_id,content,expected_revision).map_err(error) }

#[tauri::command]
pub fn create_library_note(
    core: State<AppCore>,
    asset_id: String,
    input: NoteInput,
) -> Result<String, String> {
    core.create_library_note(&asset_id, input).map_err(error)
}
#[tauri::command]
pub fn edit_library_note(
    core: State<AppCore>,
    asset_id: String,
    note_id: String,
    body: String,
    expected_revision: u64,
) -> Result<(), String> {
    core.edit_library_note(&asset_id, &note_id, &body, expected_revision)
        .map_err(error)
}
#[tauri::command]
pub fn get_library_cover(core: State<AppCore>, asset_id: String) -> Result<Vec<u8>, String> {
    core.library_cover(&asset_id).map_err(error)
}
#[tauri::command]
pub fn prepare_library_playback(
    app: tauri::AppHandle,
    core: State<AppCore>,
    asset_id: String,
    file_id: String,
) -> Result<String, String> {
    let path = core
        .library_playback_path(&asset_id, &file_id)
        .map_err(error)?;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|_| "PLAYBACK_SOURCE_FAILED".to_string())?;
    Ok(path.to_string_lossy().into_owned())
}
#[tauri::command]
pub fn reveal_library_file(
    app: tauri::AppHandle,
    core: State<AppCore>,
    asset_id: String,
    file_id: String,
) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(
            core.library_playback_path(&asset_id, &file_id)
                .map_err(error)?,
        )
        .map_err(|_| "IO_ERROR".into())
}

#[tauri::command]
pub fn open_library_file(
    app: tauri::AppHandle,
    core: State<AppCore>,
    asset_id: String,
    file_id: String,
) -> Result<(), String> {
    app.opener()
        .open_path(
            core.library_playback_path(&asset_id, &file_id)
                .map_err(error)?
                .to_string_lossy(),
            None::<&str>,
        )
        .map_err(|_| "PLAYBACK_OPEN_FAILED".into())
}

#[tauri::command]
pub async fn export_library_document(
    app: tauri::AppHandle,
    core: State<'_, AppCore>,
    asset_id: String,
    document_id: String,
) -> Result<Option<String>, String> {
    let document = core
        .library_documents(&asset_id)
        .map_err(error)?
        .into_iter()
        .find(|d| d.id == document_id)
        .ok_or("DOCUMENT_NOT_FOUND")?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导出文案")
        .set_directory(core.settings().download_directory)
        .set_file_name(format!(
            "{}-v{}.txt",
            if document.kind == "recognized" {
                "识别原文"
            } else {
                "整理稿"
            },
            document.version
        ))
        .add_filter("文本文件", &["txt"])
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|_| "IO_ERROR")? else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "IO_ERROR")?;
    core.export_library_document(&asset_id, &document_id, &path)
        .map_err(error)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command(async)]
pub fn get_workbench(
    core: State<AppCore>,
) -> Result<yingqu_core::desktop_core::workbench::Workbench, String> {
    core.workbench().map_err(error)
}
#[tauri::command]
pub fn record_library_visit(core: State<AppCore>, asset_id: String) -> Result<(), String> {
    core.record_library_visit(&asset_id).map_err(error)
}
