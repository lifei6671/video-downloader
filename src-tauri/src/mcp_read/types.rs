#[cfg(feature = "mcp")]
use rmcp::schemars;
use serde::{Deserialize, Serialize};

fn page_limit() -> u32 {
    20
}
fn text_length() -> u32 {
    4000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct ListInput {
    #[cfg_attr(feature = "mcp", schemars(length(max = 512)))]
    pub query: String,
    pub cursor: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 100)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub limit: u32,
}
impl Default for ListInput {
    fn default() -> Self {
        Self {
            query: String::new(),
            cursor: None,
            limit: page_limit(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchCursor {
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub created_at: u64,
    pub id: String,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FileFilter {
    #[default]
    All,
    Available,
    Missing,
    Changed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct SearchInput {
    #[cfg_attr(feature = "mcp", schemars(length(max = 512)))]
    pub query: String,
    #[cfg_attr(feature = "mcp", schemars(length(max = 100)))]
    pub category_ids: Vec<String>,
    #[cfg_attr(feature = "mcp", schemars(length(max = 100)))]
    pub tag_ids: Vec<String>,
    pub platform: Option<String>,
    pub archived: bool,
    pub uncategorized: bool,
    pub unorganized: bool,
    pub file_state: FileFilter,
    pub cursor: Option<SearchCursor>,
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 100)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub limit: u32,
}
impl Default for SearchInput {
    fn default() -> Self {
        Self {
            query: String::new(),
            category_ids: vec![],
            tag_ids: vec![],
            platform: None,
            archived: false,
            uncategorized: false,
            unorganized: false,
            file_state: FileFilter::All,
            cursor: None,
            limit: page_limit(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AssetInput {
    pub asset_id: String,
    #[serde(default)]
    pub document_cursor: Option<String>,
    #[serde(default = "page_limit")]
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 100)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub limit: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TranscriptInput {
    pub asset_id: String,
    #[serde(default)]
    pub document_id: Option<String>,
    /// Pin the expected immutable document version. With no ID, prefer latest edited, then recognized.
    #[serde(default)]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub version: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub offset: u64,
    #[serde(default = "text_length")]
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 16000)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub length: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NotesInput {
    pub asset_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "page_limit")]
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 100)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub limit: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub note_id: String,
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Pin this revision when reading multiple pages of a mutable note.
    #[serde(default)]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub revision: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub offset: u64,
    #[serde(default = "text_length")]
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 16000)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub length: u32,
    #[serde(default)]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub citation_offset: u64,
    #[serde(default = "page_limit")]
    #[cfg_attr(feature = "mcp", schemars(range(min = 1, max = 100)))]
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub citation_limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub position: i64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub asset_count: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub id: String,
    pub name: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub asset_count: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Categories {
    pub categories: Vec<Category>,
    pub next_cursor: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Tags {
    pub tags: Vec<Tag>,
    pub next_cursor: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub platform: Option<String>,
    pub video_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub created_at: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub updated_at: u64,
    pub archived: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileCounts {
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub available: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub changed: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchHit {
    pub field: String,
    pub excerpt: String,
    pub document_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub document_version: Option<u64>,
    pub note_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub note_revision: Option<u64>,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchItem {
    pub asset: Asset,
    pub hit: Option<SearchHit>,
    pub files: FileCounts,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<SearchCursor>,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileSummary {
    pub id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub size: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub duration_ms: Option<u64>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub width: Option<u64>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub height: Option<u64>,
    pub missing: bool,
    pub changed: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DocumentSummary {
    pub id: String,
    pub asset_id: String,
    pub kind: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub version: u64,
    pub source_document_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub created_at: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoteSummary {
    pub id: String,
    pub asset_id: String,
    pub kind: String,
    pub origin: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub revision: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub citation_count: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub created_at: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub updated_at: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NotesPage {
    pub notes: Vec<NoteSummary>,
    pub next_cursor: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AssetDetail {
    pub asset: Asset,
    pub files: Vec<FileSummary>,
    pub category_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub documents: Page<DocumentSummary>,
    pub notes: NotesPage,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TextSlice {
    pub text: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total_characters: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub offset: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub length: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub next_offset: Option<u64>,
    pub eof: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DocumentReference {
    pub asset_id: String,
    pub document_id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub document_version: u64,
    pub kind: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub offset: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub length: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Transcript {
    pub document: DocumentSummary,
    pub content: TextSlice,
    pub reference: DocumentReference,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub reference: DocumentReference,
    pub quote_excerpt: String,
    pub quote_truncated: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoteReference {
    pub asset_id: String,
    pub note_id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub revision: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub offset: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub length: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub summary: NoteSummary,
    pub content: TextSlice,
    pub reference: NoteReference,
    pub citations: Vec<Citation>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub citation_offset: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub citation_total: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub next_citation_offset: Option<u64>,
}

#[cfg(feature = "mcp")]
fn json_integer(schema: &mut schemars::Schema) {
    // JSON Schema integer/minimum carry the contract. The TS SDK's default
    // validator ignores the nonstandard unsigned integer formats and logs every occurrence.
    if let Some(object) = schema.as_object_mut() {
        object.remove("format");
    }
}
