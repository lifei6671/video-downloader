import { installComparisonFixture } from './library-comparison-fixture.js';
import { installAnalysisFixture } from './library-analysis-fixture.js';
// Mutable editions/notes and ASR summaries for UI tests; no files or model inference.
export function installLibraryWritingFixture({ api, libraryApi, state, calls, clone, saved, asset }, rich, workbench = false) {
  const documents = new Map(); const notes = new Map(); const snapshots = new Map(); const listeners = new Set();
  const body = '中文原文😀\n' + '这段文字用于学习视频口播中的结构、节奏和表达方式。\n'.repeat(rich ? 400 : 8);
  const source = id => { const f = window.libraryDetailTest.snapshot(id).files[0]; return f ? { path: f.path, size: f.size, mtime_ns: f.mtime_ns } : null; };
  state.assets.forEach(a => {
    const summary = { id: 'd-' + a.id, asset_id: a.id, file_id: 'f-' + a.id, kind: 'recognized', version: 2, source_document_id: null, characters: Array.from(body).length, model_name: null, backend: null, created_at: a.created_at };
    const doc = { summary, text: body, source: source(a.id), source_changed: false, model_revision: 'legacy-revision', pipeline_revision: 'fixture-v1' };
    documents.set(summary.id, doc);
    if (rich && (!workbench || a.id === 'a1')) {
      documents.set('old-' + a.id, { ...clone(doc), summary: { ...summary, id: 'old-' + a.id, version: 1, model_name: 'SenseVoiceSmall', backend: 'cpu' }, text: '旧版本识别原文😀' });
      documents.set('edited-' + a.id, { summary: { ...summary, id: 'edited-' + a.id, file_id: null, kind: 'edited', version: 1, source_document_id: summary.id, characters: 6, model_name: null }, text: '已保存整理稿', source: null, source_changed: null, model_revision: null, pipeline_revision: null });
    }
    if (!workbench || a.id === 'a2') notes.set('n-' + a.id, { id: 'n-' + a.id, asset_id: a.id, kind: 'note', body: '已保存学习笔记', origin: 'human', model_name: null, citations: [], revision: 1, created_at: a.created_at, updated_at: a.created_at });
  });
  const allDocuments = id => [...documents.values()].filter(d => d.summary.asset_id === id).sort((a, b) => a.summary.kind.localeCompare(b.summary.kind) || b.summary.version - a.summary.version);
  const allNotes = id => [...notes.values()].filter(n => n.asset_id === id);
  const summaryNote = n => ({ ...clone(n), body: undefined, citations: undefined, characters: Array.from(n.body).length, citation_count: n.citations.length });
  const readAsset = libraryApi.asset;
  libraryApi.asset = async id => ({ ...await readAsset(id), documents: allDocuments(id).map(d => clone(d.summary)), notes: allNotes(id).map(summaryNote) });
  let failure = null; let holdRead = false; let holdSave = false; let sequence = 100; let timestamp = 1793000200; const nextTimestamp = () => ++timestamp; let pendingReads = []; let pendingSaves = []; let exportMode = 'success'; let pendingExports = [];
  const fail = method => { if (failure === method) { failure = null; throw method === 'document' || method === 'note' ? 'DATABASE_ERROR' : method === 'export' ? 'IO_ERROR' : 'DATABASE_ERROR'; } };
  const maxEdited = id => Math.max(0, ...allDocuments(id).filter(d => d.summary.kind === 'edited').map(d => d.summary.version));
  const addEdited = (id, text, sourceId) => {
    const docId = 'edited-new-' + sequence++;
    const version = maxEdited(id) + 1;
    documents.set(docId, { summary: { id: docId, asset_id: id, file_id: null, kind: 'edited', version, source_document_id: sourceId, characters: Array.from(text).length, model_name: null, backend: null, created_at: 1793000100 }, text, source: null, source_changed: null, model_revision: null, pipeline_revision: null });
    saved('documents', [id]); return docId;
  };
  window.writingTest = {
    seedTiming: (id, segments, fileId = 'f-' + id) => {
      const doc = documents.get('d-' + id), file = window.libraryDetailTest.snapshot(id).files.find(f => f.id === fileId);
      doc.summary.file_id = fileId; doc.text = segments.map(s => s.text).join('\n'); doc.summary.characters = Array.from(doc.text).length;
      doc.source = { path: file.path, size: file.size, mtime_ns: file.mtime_ns };
      doc.timing = { operation_id: 'fixture-timing', schema_version: 1, time_base: 'media_zero', timing_source: 'fsmn_vad', precision: 'vad_segment', sample_rate: 16000, frames: 16000 * 60, vad_revision: 'synthetic', segments: clone(segments) };
      saved('documents', [id]);
    },
    releaseExports: () => pendingExports.splice(0).forEach(resolve => resolve()),
    body, calls, snapshot: id => ({ documents: clone(allDocuments(id)), notes: clone(allNotes(id)) }), failNext: method => { failure = method; },
    holdReads: () => { holdRead = true; }, releaseReads: () => { holdRead = false; pendingReads.splice(0).forEach(resolve => resolve()); },
    holdSaves: () => { holdSave = true; }, releaseSaves: () => { holdSave = false; pendingSaves.splice(0).forEach(resolve => resolve()); }, exportMode: mode => { exportMode = mode; },
    remoteEdit: id => addEdited(id, '另一个窗口保存的整理稿', 'd-' + id),
    remoteNote: id => { const n = notes.get('n-' + id); n.body = '另一个窗口保存的笔记'; n.revision++; saved('notes', [id]); },
    progress: fileId => { const s = snapshots.get(fileId); Object.assign(s, { sequence: s.sequence + 1, phase: 'recognizing', total_segments: 10, completed_segments: 3 }); listeners.forEach(fn => fn(clone(s))); },
    complete: (id, fileId) => {
      const s = snapshots.get(fileId); const version = Math.max(...allDocuments(id).filter(d => d.summary.kind === 'recognized').map(d => d.summary.version)) + 1;
      const old = documents.get('d-' + id); const next = clone(old); next.summary.id = 'recognition-' + sequence++; next.summary.version = version; next.summary.created_at = 1793000200; next.summary.file_id = fileId; next.text = '新提取的识别原文'; next.source = source(id); documents.set(next.summary.id, next);
      Object.assign(s, { sequence: s.sequence + 1, state: 'completed', phase: null, has_result: true }); saved('documents', [id]); listeners.forEach(fn => fn(clone(s)));
    },
  };
  libraryApi.documents = async id => allDocuments(id).map(d => clone(d.summary));
  libraryApi.document = async (assetId, id) => {
    calls.push({ method: 'document', assetId, id }); fail('document'); const doc = clone(documents.get(id)); if (!doc || doc.summary.asset_id !== assetId) throw 'DOCUMENT_NOT_FOUND';
    if (doc.source) { const f = window.libraryDetailTest.snapshot(assetId).files.find(f => f.id === doc.summary.file_id); doc.source_changed = !f || f.missing || f.changed || f.path !== doc.source.path || f.mtime_ns !== doc.source.mtime_ns; }
    if (holdRead) await new Promise(resolve => pendingReads.push(resolve)); return doc;
  };
  libraryApi.saveDocument = async (assetId, text, expectedVersion, sourceDocumentId) => {
    calls.push({ method: 'saveDocument', assetId, text, expectedVersion, sourceDocumentId }); fail('saveDocument');
    if (holdSave) await new Promise(resolve => pendingSaves.push(resolve));
    if (maxEdited(assetId) !== expectedVersion) throw 'DOCUMENT_CONFLICT';
    if (!text.trim()) throw 'INVALID_INPUT'; return addEdited(assetId, text, sourceDocumentId);
  };
  libraryApi.notes = async id => allNotes(id).map(summaryNote);
  libraryApi.note = async (assetId, id) => { fail('note'); const note = notes.get(id); if (!note || note.asset_id !== assetId) throw 'NOTE_NOT_FOUND'; return clone(note); };
  libraryApi.createNote = async (assetId, input) => {
    calls.push({ method: 'createNote', assetId, input: clone(input) }); fail('createNote');
    if (!input.body.trim()) throw 'INVALID_INPUT';
    for (const c of input.citations ?? []) if (documents.get(c.document_id)?.summary.asset_id !== c.asset_id || !documents.get(c.document_id).text.includes(c.quote)) throw 'INVALID_CITATION';
    const id = 'note-new-' + sequence++; notes.set(id, { ...clone(input), id, asset_id: assetId, citations: input.citations ?? [], origin: 'human', model_name: null, revision: 1, created_at: nextTimestamp(), updated_at: nextTimestamp() }); saved('notes', [assetId]); return id;
  };
  libraryApi.editNote = async (assetId, id, text, expectedRevision) => {
    calls.push({ method: 'editNote', assetId, id, text, expectedRevision }); fail('editNote'); const note = notes.get(id);
    if (note.revision !== expectedRevision) throw 'NOTE_CONFLICT'; note.body = text; note.revision++; saved('notes', [assetId]);
  };
  installAnalysisFixture({ libraryApi, notes, clone, saved, fail, calls, nextTimestamp, waitForSave: async () => { if (holdSave) await new Promise(resolve => pendingSaves.push(resolve)); } });
  if (new URLSearchParams(location.search).has('comparison')) installComparisonFixture({ libraryApi, notes, documents, clone, saved, fail, calls, asset });
  libraryApi.exportDocument = async (assetId, documentId) => { calls.push({ method: 'export', assetId, documentId }); fail('export'); if (exportMode === 'hold') await new Promise(resolve => pendingExports.push(resolve)); return exportMode === 'cancel' ? null : 'C:\\SyntheticExports\\文案.txt'; };
  libraryApi.exportContent = async (assetId, selection, format) => { if (selection.kind === 'document' && format === 'txt') return libraryApi.exportDocument(assetId, selection.id); calls.push({method:'exportContent', assetId, selection:clone(selection), format}); fail('export'); if (exportMode === 'hold') await new Promise(resolve => pendingExports.push(resolve)); return exportMode === 'cancel' ? null : 'C:\\SyntheticExports\\已保存内容.' + (format === 'markdown' ? 'md' : format); };
  api.subscribeAssetTranscripts = async fn => { listeners.add(fn); return () => listeners.delete(fn); };
  api.assetTranscript = async (assetId, fileId) => snapshots.get(fileId) ?? null;
  api.startAssetTranscription = async (assetId, fileId, force) => {
    calls.push({ method: 'recognize', assetId, fileId, force }); fail('recognize');
    const s = { asset_id: assetId, file_id: fileId, task_id: assetId, operation_id: 'operation-' + sequence++, sequence: sequence++, state: 'running', phase: 'extracting_audio', error_code: null, has_result: true, result: null, updated_at: 1793000000, source_changed: false };
    snapshots.set(fileId, s); listeners.forEach(fn => fn(clone(s))); return clone(s);
  };
  api.cancelAssetTranscription = async fileId => { const s = snapshots.get(fileId); Object.assign(s, { sequence: s.sequence + 1, state: 'canceled', error_code: 'CANCELED', phase: null }); listeners.forEach(fn => fn(clone(s))); };
}
