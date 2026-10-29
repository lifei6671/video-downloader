import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface LibraryAsset {
  id: string; title: string; author: string | null; platform: string | null; video_id: string | null;
  canonical_url: string | null; created_at: number; updated_at: number; archived_at: number | null;
}
export interface LibraryFile {
  id: string; asset_id: string; path: string; size: number; mtime_ns: string | null;
  duration_ms: number | null; width: number | null; height: number | null; missing: boolean; changed: boolean;
}
export interface LibraryCategory { id: string; parent_id: string | null; name: string; position: number }
export interface LibraryTag { id: string; name: string }
export interface LibraryCursor { created_at: number; id: string }
export interface LibrarySearchRequest {
  query?: string; category_ids?: string[]; tag_ids?: string[];
  platform?: 'douyin' | 'tiktok' | 'youtube' | 'telegram' | 'local' | null;
  archived?: boolean; uncategorized?: boolean; unorganized?: boolean; file_state?: 'all' | 'available' | 'missing' | 'changed';
  cursor?: LibraryCursor | null; limit?: number;
}
export interface LibrarySearchHit {
  field: 'title' | 'author' | 'recognized' | 'edited' | 'note'; excerpt: string;
  document_id: string | null; document_version: number | null; note_id: string | null;
}
export interface LibrarySearchPage {
  items: { category_ids: string[]; tag_ids: string[]; has_transcript: boolean; asset: LibraryAsset; hit: LibrarySearchHit | null; files: { total: number; available: number; changed: number } }[];
  next_cursor: LibraryCursor | null;
}
export interface LibraryDocumentSummary {
  id: string; asset_id: string; file_id: string | null; kind: 'recognized' | 'edited'; version: number;
  source_document_id: string | null; characters: number; model_name: string | null; backend: string | null; created_at: number;
}
export interface LibraryDocumentContent {
  timing?: TranscriptTiming | null;
  summary: LibraryDocumentSummary; text: string;
  source: { path: string; size: number; mtime_ns: string } | null;
  source_changed: boolean | null; model_revision: string | null; pipeline_revision: string | null;
}
export interface TranscriptTiming {
  operation_id: string;
  schema_version: 1; time_base: 'media_zero'; timing_source: 'fsmn_vad'; precision: 'vad_segment';
  sample_rate: 16000; frames: number; vad_revision: string;
  segments: { start_ms: number; end_ms: number; text: string }[];
}
export interface ClipRange { start_ms: number; end_ms: number }
export interface LibraryClip extends ClipRange {
  id: string; asset_id: string; file_id: string; title: string; reason: string;
  document_id: string | null; document_version: number | null; quote: string | null;
  created_at: number; missing: boolean; source_changed: boolean;
}
export interface LibraryClipInput extends ClipRange {
  file_id: string; title: string; reason: string; playable: ClipRange;
  document_id: string | null; quote: string | null;
}
export type KeyframeRequest = { kind: 'position'; file_id: string; position_ms: number } | { kind: 'clip'; clip_id: string; count: number } | { kind: 'range'; file_id: string; start_ms: number; end_ms: number; count?: 3 | 5 | 9 };
export interface LibraryKeyframe {
  id: string; asset_id: string; file_id: string; clip_id: string | null; batch_id: string;
  sample_start_ms: number; sample_end_ms: number | null; sample_index: number; sample_count: number;
  requested_ms: number; actual_ms: number; pts: number; time_base_num: number; time_base_den: number;
  width: number; height: number; black: boolean; created_at: number; bytes: number; missing: boolean; source_changed: boolean;
}
export interface KeyframeSnapshot {
  id: string; sequence: number; asset_id: string; file_id: string; clip_id: string | null;
  state: 'running' | 'completed' | 'failed' | 'canceled'; completed: number; total: number;
  canceling: boolean; error: string | null; frame_ids: string[];
}
export interface LibraryCitation { asset_id: string; document_id: string; quote: string }
export type ObservationDimension = 'visual' | 'captions' | 'transition' | 'sound';
export type ObservationEvidence = 'video' | 'audio' | 'keyframe' | 'unverified';
export interface ObservationInput { dimension: ObservationDimension; evidence: ObservationEvidence; text: string; clip_id: string; keyframe_id: string | null }
export interface LibraryObservation extends ObservationInput { file_id: string; clip_title: string; start_ms: number; end_ms: number; frame_actual_ms: number | null }
export type CopywritingAnalysis = { kind: 'copywriting'; opening: string; viewpoint: string; evidence: string; turn: string; ending: string; takeaways: string };
export type AnalysisInput = CopywritingAnalysis | { kind: 'technique'; observations: ObservationInput[] };
export interface LibraryNoteAnalysis { content: CopywritingAnalysis | { kind: 'technique'; observations: LibraryObservation[] }; human_revised: boolean }
export interface LibraryNoteInput { kind: string; body: string; citations?: LibraryCitation[] }
export interface LibraryNoteSummary {
  id: string; asset_id: string; kind: string; origin: 'human' | 'ai'; model_name: string | null;
  revision: number; characters: number; citation_count: number; created_at: number; updated_at: number;
}
export interface ComparisonPin { asset_id: string; document_id: string | null; copywriting: { id: string; revision: number } | null; technique: { id: string; revision: number } | null; clip_ids: string[] }
export interface ComparisonAnalysis { id: string; revision: number; origin: 'human' | 'ai'; model_name: string | null; human_revised: boolean; excerpt: string; structure: string[] | null }
export interface ComparisonSource { pin: ComparisonPin; title: string; document: { id: string; version: number; opening: string } | null; copywriting: ComparisonAnalysis | null; technique: ComparisonAnalysis | null; clips: Omit<LibraryClip, 'asset_id' | 'reason' | 'created_at' | 'missing' | 'source_changed'>[] }
export interface NoteComparison { title: string; asset_ids: string[]; points: { text: string; sources: ComparisonSource[] }[] }
export interface ComparisonEdit { title: string; point_texts: string[] }
export interface LibraryNote {
  comparison?: NoteComparison | null;
  analysis?: LibraryNoteAnalysis | null;
  id: string; asset_id: string; kind: string; body: string; origin: 'human' | 'ai'; model_name: string | null;
  citations: LibraryCitation[]; revision: number; created_at: number; updated_at: number;
}
export interface LibraryAssetDetail {
  asset: LibraryAsset; files: LibraryFile[]; category_ids: string[]; tag_ids: string[];
  documents: LibraryDocumentSummary[]; notes: LibraryNoteSummary[];
}
export interface LibraryChange { scope: 'assets' | 'categories' | 'tags' | 'documents' | 'notes' | 'visits' | 'operations' | 'clips' | 'keyframes' | 'topics' | 'ideas'; asset_ids: string[] }

export interface WorkbenchOperation {
  id: string; kind: 'download' | 'transcription' | 'import' | 'backup' | 'restore' | 'keyframes'; title: string; state: string; phase: string | null;
  error: string | null; asset_id: string | null; file_id: string | null; task_id: string | null;
}
export interface WorkbenchSnapshot {
  ideas: { items: IdeaSummary[]; total: number };
  recent: { items: LibraryAsset[]; total: number };
  uncategorized: { items: LibraryAsset[]; total: number };
  unorganized: { items: LibraryAsset[]; total: number };
  operations: WorkbenchOperation[];
}
export interface AiSource { model_name: string; citations: LibraryCitation[] }
export interface ResearchTopic { ai_source?: AiSource; id: string; title: string; description: string; revision: number; created_at: number; updated_at: number }
export type TopicMemberKind = 'video' | 'clip' | 'note';
export interface TopicMember { id: string; topic_id: string; kind: TopicMemberKind; asset_id: string; resource_id: string; label: string; asset_title: string; position: number; created_at: number }
export interface TopicMemberView { member: TopicMember; title: string; asset_title: string; missing: boolean; archived: boolean; file_state: 'available' | 'missing' | 'changed' | 'unknown' | null; note_revision: number | null }
export interface ResearchTopicDetail { topic: ResearchTopic; members: TopicMemberView[] }
export interface TopicInput { title: string; description: string }
export interface TopicMemberInput { kind: TopicMemberKind; asset_id: string; resource_id: string }
export interface LibraryIdea { ai_source?: AiSource; id: string; title: string; viewpoint: string; body: string; revision: number; created_at: number; updated_at: number }
export interface IdeaSummary { id: string; title: string; revision: number; characters: number; reference_count: number; created_at: number; updated_at: number }
export type IdeaReferenceKind = 'video' | 'clip' | 'topic';
export interface IdeaReferenceInput { kind: IdeaReferenceKind; asset_id: string | null; resource_id: string }
export interface IdeaReferenceView { reference: IdeaReferenceInput; title: string; asset_title: string | null; missing: boolean; archived: boolean; file_state: string | null; topic_revision: number | null }
export interface IdeaDetail { idea: LibraryIdea; references: IdeaReferenceView[] }
export interface IdeaInput { title: string; viewpoint: string; body: string; references: IdeaReferenceInput[] }
export interface LibraryWritingSelection { documentId?: string; noteId?: string }
export interface LibraryDestination extends LibraryWritingSelection { view?: 'ideas'; ideaId?: string; ideaReference?: IdeaReferenceInput; sequence: number; assetId?: string; fileId?: string; scope?: 'uncategorized' | 'unorganized'; error?: string }

export type LibraryExportFormat = 'txt' | 'markdown' | 'json';
export type LibraryContentSelection = { kind: 'document' | 'note'; id: string };
export interface LibraryBackupInfo { ideas?: number; topics?: number; assets: number; documents: number; notes: number; files: number; available_files: number; media_bytes: number; empty: boolean; busy: boolean }
export interface LibraryTransferSnapshot {
  id: string; order: number; sequence: number; kind: 'backup' | 'restore'; include_media: boolean; state: 'running' | 'completed' | 'failed' | 'canceled';
  phase: string; completed_files: number; total_files: number; bytes: number; canceling: boolean;
  path: string; error: string | null;
  result: { ideas?: number; topics?: number; assets: number; files?: number; documents?: number; notes?: number; missing_media: number; bytes?: number } | null;
}
export interface TaskCollectionState { task_id: string; asset_id: string; collected: boolean }
export const libraryApi = {
  taskCollectionStates: (taskIds: string[]) => invoke<TaskCollectionState[]>('get_task_collection_states', { taskIds }),
  setTaskCollected: (taskId: string, collected: boolean) => invoke<TaskCollectionState>('set_task_collected', { taskId, collected }),
  ideas: () => invoke<IdeaSummary[]>('list_library_ideas'),
  idea: (ideaId: string) => invoke<IdeaDetail>('get_library_idea', { ideaId }),
  prepareIdeaReference: (input: IdeaReferenceInput) => invoke<IdeaReferenceView>('prepare_library_idea_reference', { input }),
  createIdea: (input: IdeaInput) => invoke<string>('create_library_idea', { input }),
  editIdea: (ideaId: string, input: IdeaInput, expectedRevision: number) => invoke<void>('edit_library_idea', { ideaId, input, expectedRevision }),
  topics: () => invoke<ResearchTopic[]>('list_library_topics'),
  topic: (topicId: string) => invoke<ResearchTopicDetail>('get_library_topic', { topicId }),
  createTopic: (input: TopicInput) => invoke<string>('create_library_topic', { input }),
  editTopic: (topicId: string, input: TopicInput, expectedRevision: number) => invoke<void>('edit_library_topic', { topicId, input, expectedRevision }),
  addTopicMember: (topicId: string, input: TopicMemberInput, expectedRevision: number) => invoke<string>('add_library_topic_member', { topicId, input, expectedRevision }),
  removeTopicMember: (topicId: string, memberId: string, expectedRevision: number) => invoke<void>('remove_library_topic_member', { topicId, memberId, expectedRevision }),
  reorderTopicMembers: (topicId: string, memberIds: string[], expectedRevision: number) => invoke<void>('reorder_library_topic_members', { topicId, memberIds, expectedRevision }),
  comparisonSource: (pin: ComparisonPin) => invoke<ComparisonSource>('get_library_comparison_source', { pin }),
  createComparison: (assetId: string, input: { title: string; asset_ids: string[]; points: { text: string; sources: ComparisonPin[] }[] }) => invoke<string>('create_library_comparison', { assetId, input }),
  editComparison: (assetId: string, noteId: string, input: ComparisonEdit, expectedRevision: number) => invoke<void>('edit_library_comparison', { assetId, noteId, input, expectedRevision }),
  keyframes: (assetId: string) => invoke<LibraryKeyframe[]>('list_library_keyframes', { assetId }),
  keyframeImage: async (assetId: string, keyframeId: string) => new Uint8Array(await invoke<ArrayBuffer>('get_library_keyframe_image', { assetId, keyframeId })),
  deleteKeyframe: (assetId: string, keyframeId: string) => invoke<void>('delete_library_keyframe', { assetId, keyframeId }),
  keyframeOperation: () => invoke<KeyframeSnapshot | null>('get_library_keyframe_operation'),
  startKeyframes: (assetId: string, request: KeyframeRequest) => invoke<KeyframeSnapshot>('start_library_keyframes', { assetId, request }),
  cancelKeyframes: (operationId: string) => invoke<void>('cancel_library_keyframes', { operationId }),
  subscribeKeyframes: (callback: (snapshot: KeyframeSnapshot) => void) => listen<KeyframeSnapshot>('library_keyframes_updated', event => callback(event.payload)),
  clips: (assetId: string) => invoke<LibraryClip[]>('list_library_clips', { assetId }),
  createClip: (assetId: string, input: LibraryClipInput) => invoke<string>('create_library_clip', { assetId, input }),
  deleteClip: (assetId: string, clipId: string) => invoke<void>('delete_library_clip', { assetId, clipId }),
  clipPlayback: async (assetId: string, clipId: string) => {
    const value = await invoke<{ clip: LibraryClip; path: string }>('prepare_library_clip', { assetId, clipId });
    return { clip: value.clip, url: convertFileSrc(value.path) };
  },
  backupInfo: () => invoke<LibraryBackupInfo>('get_library_backup_info'),
  transfer: () => invoke<LibraryTransferSnapshot | null>('get_library_transfer'),
  startBackup: (includeMedia = false) => invoke<LibraryTransferSnapshot | null>('start_library_backup', { includeMedia }),
  startRestore: () => invoke<LibraryTransferSnapshot | null>('start_library_restore'),
  cancelTransfer: (operationId: string) => invoke<void>('cancel_library_transfer', { operationId }),
  subscribeTransfer: (callback: (snapshot: LibraryTransferSnapshot) => void) => listen<LibraryTransferSnapshot>('library_transfer_updated', event => callback(event.payload)),
  exportContent: (assetId: string, selection: LibraryContentSelection, format: LibraryExportFormat) => invoke<string | null>('export_library_content', { assetId, selection, format }),
  workbench: () => invoke<WorkbenchSnapshot>('get_workbench'),
  visit: (assetId: string) => invoke<void>('record_library_visit', { assetId }),
  search: (request: LibrarySearchRequest = {}) => invoke<LibrarySearchPage>('search_library_assets', { request }),
  asset: (assetId: string) => invoke<LibraryAssetDetail>('get_library_asset', { assetId }),
  archive: (assetId: string, archived: boolean) => invoke<void>('archive_library_asset', { assetId, archived }),
  categories: () => invoke<LibraryCategory[]>('list_library_categories'),
  createCategory: (name: string, parentId: string | null = null) => invoke<string>('create_library_category', { name, parentId }),
  updateCategory: (categoryId: string, name: string, parentId: string | null, position: number) => invoke<void>('update_library_category', { categoryId, name, parentId, position }),
  deleteCategory: (categoryId: string) => invoke<void>('delete_library_category', { categoryId }),
  setCategories: (assetIds: string[], categoryId: string, add: boolean) => invoke<void>('set_library_categories', { assetIds, categoryId, add }),
  tags: () => invoke<LibraryTag[]>('list_library_tags'),
  createTag: (name: string) => invoke<string>('create_library_tag', { name }),
  renameTag: (tagId: string, name: string) => invoke<void>('rename_library_tag', { tagId, name }),
  deleteTag: (tagId: string) => invoke<void>('delete_library_tag', { tagId }),
  setTags: (assetIds: string[], tagId: string, add: boolean) => invoke<void>('set_library_tags', { assetIds, tagId, add }),
  documents: (assetId: string) => invoke<LibraryDocumentSummary[]>('list_library_documents', { assetId }),
  document: (assetId: string, documentId: string) => invoke<LibraryDocumentContent>('get_library_document', { assetId, documentId }),
  saveDocument: (assetId: string, text: string, expectedVersion: number, sourceDocumentId: string | null = null) => invoke<string>('save_library_document', { assetId, text, expectedVersion, sourceDocumentId }),
  notes: (assetId: string) => invoke<LibraryNoteSummary[]>('list_library_notes', { assetId }),
  note: (assetId: string, noteId: string) => invoke<LibraryNote>('get_library_note', { assetId, noteId }),
  createNote: (assetId: string, input: LibraryNoteInput) => invoke<string>('create_library_note', { assetId, input }),
  createAnalysis: (assetId: string, input: { content: AnalysisInput; origin: 'human' | 'ai'; model_name: string | null; citations: LibraryCitation[] }) => invoke<string>('create_library_analysis', { assetId, input }),
  editAnalysis: (assetId: string, noteId: string, content: AnalysisInput, expectedRevision: number) => invoke<void>('edit_library_analysis', { assetId, noteId, content, expectedRevision }),
  editNote: (assetId: string, noteId: string, body: string, expectedRevision: number) => invoke<void>('edit_library_note', { assetId, noteId, body, expectedRevision }),
  exportDocument: (assetId: string, documentId: string) => invoke<string | null>('export_library_document', { assetId, documentId }),
  cover: async (assetId: string) => new Uint8Array(await invoke<number[]>('get_library_cover', { assetId })),
  playbackUrl: async (assetId: string, fileId: string) => convertFileSrc(await invoke<string>('prepare_library_playback', { assetId, fileId })),
  reveal: (assetId: string, fileId: string) => invoke<void>('reveal_library_file', { assetId, fileId }),
  open: (assetId: string, fileId: string) => invoke<void>('open_library_file', { assetId, fileId }),
  subscribe: (callback: (change: LibraryChange) => void) => listen<LibraryChange>('library_changed', event => callback(event.payload)),
};
