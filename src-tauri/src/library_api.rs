use super::{Core, TaskEvent};
use crate::library::{
    self, AssetDetail, Category, Citation, DocumentContent, DocumentSummary, NewNote, Note,
    NoteOrigin, NoteSummary, SearchPage, SearchRequest, Tag,
};
use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryScope {
    Assets,
    Categories,
    Tags,
    Documents,
    Notes,
    Visits,
    Operations,
    Clips,
    Keyframes,
    Topics,
    Ideas,
}
#[derive(Clone, Debug, Serialize)]
pub struct LibraryChange {
    pub scope: LibraryScope,
    pub asset_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub kind: String,
    pub body: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

pub(super) fn database_error(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<rusqlite::Error>().is_some()
        || error.downcast_ref::<serde_json::Error>().is_some()
    {
        error.context("DATABASE_ERROR")
    } else {
        error
    }
}
impl Core {
    pub fn library_keyframes(&self, asset: &str) -> Result<Vec<library::Keyframe>> { self.library_read(|db|library::keyframes(db,asset)) }
    pub fn library_keyframe_image(&self, asset: &str, id: &str) -> Result<Vec<u8>> { self.library_read(|db|library::keyframe_image(db,asset,id)) }
    pub fn delete_library_keyframe(&self, asset: &str, id: &str) -> Result<()> { self.library_write(LibraryScope::Keyframes,vec![asset.into()],|db|library::delete_keyframe(db,asset,id)) }

    pub fn library_clips(&self, asset: &str) -> Result<Vec<library::Clip>> {
        self.library_read(|db|library::clips(db,asset))
    }
    pub fn create_library_clip(&self, asset: &str, input: &library::ClipInput) -> Result<String> {
        self.library_write(LibraryScope::Clips,vec![asset.into()],|db|library::create_clip(db,asset,input))
    }
    pub fn delete_library_clip(&self, asset: &str, id: &str) -> Result<()> {
        self.library_write(LibraryScope::Clips,vec![asset.into()],|db|library::delete_clip(db,asset,id))
    }
    pub fn library_clip_playback(&self, asset: &str, id: &str) -> Result<(library::Clip,PathBuf)> {
        self.library_read(|db|library::clip_playback(db,asset,id))
    }
    fn library_read<T>(&self, action: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        action(&self.inner.lock().unwrap().db).map_err(database_error)
    }
    fn library_write<T>(
        &self,
        scope: LibraryScope,
        asset_ids: Vec<String>,
        action: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let result = {
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            action(&inner.db).map_err(database_error)?
        };
        (self.emit)(TaskEvent::LibraryChanged(LibraryChange {
            scope,
            asset_ids,
        }));
        Ok(result)
    }
    pub fn search_library(&self, request: SearchRequest) -> Result<SearchPage> {
        self.library_read(|db| library::search_assets(db, &request))
    }
    pub fn library_asset(&self, id: &str) -> Result<AssetDetail> {
        self.library_read(|db| library::asset_detail(db, id))
    }
    pub fn archive_library_asset(&self, id: &str, archived: bool) -> Result<()> {
        self.library_write(LibraryScope::Assets, vec![id.into()], |db| {
            library::archive_asset(db, id, archived)
        })
    }
    pub fn task_collection_states(&self, task_ids: &[String]) -> Result<Vec<library::TaskCollectionState>> {
        self.library_read(|db| library::task_collection_states(db, task_ids))
    }
    pub fn set_task_collected(&self, task_id: &str, collected: bool) -> Result<library::TaskCollectionState> {
        let state = self.library_write(LibraryScope::Assets, vec![], |db| {
            library::set_task_collected(db, task_id, collected)
        })?;
        Ok(state)
    }
    pub fn library_categories(&self) -> Result<Vec<Category>> {
        self.library_read(|db| {
            let mut categories = library::categories(db)?;
            for category in &mut categories { category.parent_id = None; }
            Ok(categories)
        })
    }
    pub fn create_library_category(&self, parent: Option<&str>, name: &str) -> Result<String> {
        if parent.is_some() { bail!("INVALID_INPUT"); }
        self.library_write(LibraryScope::Categories, vec![], |db| {
            library::create_category(db, parent, name)
        })
    }
    pub fn update_library_category(
        &self,
        id: &str,
        parent: Option<&str>,
        name: &str,
        position: i64,
    ) -> Result<()> {
        if parent.is_some() { bail!("INVALID_INPUT"); }
        self.library_write(LibraryScope::Categories, vec![], |db| {
            // A flat editor must not rewrite historical hierarchy metadata.
            let saved_parent = library::categories(db)?
                .into_iter()
                .find(|category| category.id == id)
                .context("CATEGORY_NOT_FOUND")?
                .parent_id;
            library::update_category(db, id, saved_parent.as_deref(), name, position)
        })
    }
    pub fn delete_library_category(&self, id: &str) -> Result<()> {
        self.library_write(LibraryScope::Categories, vec![], |db| {
            library::delete_category(db, id)
        })
    }
    pub fn set_library_categories(&self, ids: &[String], category: &str, add: bool) -> Result<()> {
        self.library_write(LibraryScope::Assets, ids.to_vec(), |db| {
            library::set_category_membership(db, ids, category, add)
        })
    }
    pub fn library_tags(&self) -> Result<Vec<Tag>> {
        self.library_read(library::tags)
    }
    pub fn create_library_tag(&self, name: &str) -> Result<String> {
        self.library_write(LibraryScope::Tags, vec![], |db| {
            library::create_tag(db, name)
        })
    }
    pub fn rename_library_tag(&self, id: &str, name: &str) -> Result<()> {
        self.library_write(LibraryScope::Tags, vec![], |db| {
            library::rename_tag(db, id, name)
        })
    }
    pub fn delete_library_tag(&self, id: &str) -> Result<()> {
        self.library_write(LibraryScope::Tags, vec![], |db| library::delete_tag(db, id))
    }
    pub fn set_library_tags(&self, ids: &[String], tag: &str, add: bool) -> Result<()> {
        self.library_write(LibraryScope::Assets, ids.to_vec(), |db| {
            library::set_tag_membership(db, ids, tag, add)
        })
    }
    pub fn library_documents(&self, id: &str) -> Result<Vec<DocumentSummary>> {
        self.library_read(|db| library::document_summaries(db, id))
    }
    pub fn library_document(&self, asset: &str, id: &str) -> Result<DocumentContent> {
        self.library_read(|db| library::read_document(db, asset, id))
    }
    pub fn save_library_document(
        &self,
        asset: &str,
        text: &str,
        expected_version: u64,
        source: Option<&str>,
    ) -> Result<String> {
        self.library_write(LibraryScope::Documents, vec![asset.into()], |db| {
            library::save_edited(db, asset, text, expected_version, source)
        })
    }
    pub fn library_ideas(&self)->Result<Vec<library::IdeaSummary>> { self.library_read(library::idea_summaries) }
    pub fn library_idea(&self,id:&str)->Result<library::IdeaDetail> { self.library_read(|db|library::library_idea(db,id)) }
    pub fn library_idea_reference(&self,input:&library::IdeaReferenceInput)->Result<library::IdeaReferenceView> { self.library_read(|db|library::prepare_idea_reference(db,input)) }
    pub fn create_library_idea(&self,input:library::IdeaInput)->Result<String> { self.library_write(LibraryScope::Ideas,vec![],|db|library::create_idea(db,&input)) }
    pub fn edit_library_idea(&self,id:&str,input:library::IdeaInput,revision:u64)->Result<()> { self.library_write(LibraryScope::Ideas,vec![],|db|library::edit_idea(db,id,&input,revision)) }
    pub fn library_topics(&self) -> Result<Vec<library::ResearchTopic>> { self.library_read(library::research_topics) }
    pub fn library_topic(&self, id: &str) -> Result<library::ResearchTopicDetail> { self.library_read(|db|library::research_topic(db,id)) }
    pub fn create_library_topic(&self, input: library::TopicInput) -> Result<String> { self.library_write(LibraryScope::Topics,vec![],|db|library::create_research_topic(db,&input)) }
    pub fn edit_library_topic(&self,id:&str,input:library::TopicInput,revision:u64)->Result<()> { self.library_write(LibraryScope::Topics,vec![],|db|library::edit_research_topic(db,id,&input,revision)) }
    pub fn add_library_topic_member(&self,id:&str,input:library::TopicMemberInput,revision:u64)->Result<String> { self.library_write(LibraryScope::Topics,vec![input.asset_id.clone()],|db|library::add_topic_member(db,id,&input,revision)) }
    pub fn remove_library_topic_member(&self,id:&str,member:&str,revision:u64)->Result<()> { self.library_write(LibraryScope::Topics,vec![],|db|library::remove_topic_member(db,id,member,revision)) }
    pub fn reorder_library_topic_members(&self,id:&str,members:Vec<String>,revision:u64)->Result<()> { self.library_write(LibraryScope::Topics,vec![],|db|library::reorder_topic_members(db,id,&members,revision)) }
    pub fn library_notes(&self, id: &str) -> Result<Vec<NoteSummary>> {
        self.library_read(|db| library::note_summaries(db, id))
    }
    pub fn library_note(&self, asset: &str, id: &str) -> Result<Note> {
        self.library_read(|db| library::read_note(db, asset, id))
    }
    pub fn library_comparison_source(&self, pin: &library::ComparisonPin) -> Result<library::ComparisonSource> { self.library_read(|db| library::comparison_source(db,pin)) }
    pub fn create_library_comparison(&self, asset: &str, input: library::NewComparison) -> Result<String> { self.library_write(LibraryScope::Notes,vec![asset.into()],|db| library::create_comparison(db,asset,&input)) }
    pub fn edit_library_comparison(&self, asset: &str, id: &str, input: library::ComparisonEdit, revision: u64) -> Result<()> { self.library_write(LibraryScope::Notes,vec![asset.into()],|db| library::edit_comparison(db,asset,id,&input,revision)) }
    pub fn create_library_analysis(&self, asset: &str, input: library::NewAnalysis) -> Result<String> {
        self.library_write(LibraryScope::Notes, vec![asset.into()], |db| library::create_analysis(db,asset,&input))
    }
    pub fn edit_library_analysis(&self, asset: &str, id: &str, content: library::AnalysisInput, revision: u64) -> Result<()> {
        self.library_write(LibraryScope::Notes, vec![asset.into()], |db| library::edit_analysis(db,asset,id,&content,revision))
    }
    pub fn create_library_note(&self, asset: &str, input: NoteInput) -> Result<String> {
        if input.kind.trim() == "comparison" { bail!("COMPARISON_REQUIRED"); }
        self.library_write(LibraryScope::Notes, vec![asset.into()], |db| {
            library::create_note(
                db,
                asset,
                NewNote {
                    kind: &input.kind,
                    body: &input.body,
                    origin: NoteOrigin::Human,
                    model_name: None,
                    citations: &input.citations,
                },
            )
        })
    }
    pub fn edit_library_note(
        &self,
        asset: &str,
        id: &str,
        body: &str,
        revision: u64,
    ) -> Result<()> {
        self.library_write(LibraryScope::Notes, vec![asset.into()], |db| {
            let exists = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_notes WHERE id=?1 AND asset_id=?2)",
                rusqlite::params![id, asset],
                |r| r.get::<_, bool>(0),
            )?;
            if !exists {
                bail!("NOTE_NOT_FOUND");
            }
            library::edit_note(db, id, body, revision)
        })
    }
    pub fn export_library_document(
        &self,
        asset: &str,
        document: &str,
        target: &std::path::Path,
    ) -> Result<()> {
        let (text, protected) = self.library_read(|db| {
            let document = library::read_document(db, asset, document)?;
            let mut paths: Vec<PathBuf> = library::asset_files(db, asset)?.into_iter().map(|f| f.path).collect();
            let mut query = db.prepare("SELECT source_json FROM asset_documents WHERE asset_id=?1 AND source_json IS NOT NULL UNION SELECT source_json FROM asset_clips WHERE asset_id=?1 UNION SELECT source_json FROM asset_keyframes WHERE asset_id=?1")?;
            for row in query.query_map([asset], |r| r.get::<_, String>(0))? {
                let source: super::transcription::Source = serde_json::from_str(&row?)?;
                paths.push(source.path);
            }
            Ok((document.text, paths))
        })?;
        super::transcription::export_text(&text, target, &protected)
    }
    pub fn library_cover(&self, asset: &str) -> Result<Vec<u8>> {
        self.library_read(|db| library::asset_cover(db, asset))
    }
    pub fn library_playback_path(&self, asset: &str, file: &str) -> Result<PathBuf> {
        let file = self.library_read(|db| library::asset_file(db, asset, file))?;
        let metadata = std::fs::symlink_metadata(&file.path).context("FILE_MISSING")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("FILE_MISSING");
        }
        Ok(file.path)
    }
}

#[cfg(test)]
#[path = "library_api_tests.rs"]
mod tests;
