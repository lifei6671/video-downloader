import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryAssetDetail, type LibraryCitation, type LibraryDocumentContent, type LibraryNote, type AnalysisInput, type ComparisonEdit } from '@/lib/library-api';
import { analysisInput, blankAnalysis, analysisCanSave } from '@/lib/note-analysis';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';

export async function readLibraryDocument(assetId: string, id: string) {
  const document = await libraryApi.document(assetId, id);
  let origin = document;
  // Source links refer to existing immutable editions, so they cannot form cycles.
  while (!origin.source && origin.summary.source_document_id) origin = await libraryApi.document(assetId, origin.summary.source_document_id);
  return { document, origin };
}
type Mode = 'recognized' | 'edited' | 'notes';
interface WritingState {
  comparison?: ComparisonEdit; savedComparison?: string;
  analysis?: AnalysisInput; savedAnalysis?: string; noteOrigin: 'human' | 'ai'; modelName: string;
  mode: Mode; id: string | null; body: string; savedBody: string; kind: string;
  document?: LibraryDocumentContent; origin?: LibraryDocumentContent; note?: LibraryNote;
  citations: LibraryCitation[]; expectedVersion: number; expectedRevision: number; needsRead: boolean; loading: boolean; error: string; conflict: boolean;
}
const blank = (mode: Mode, expectedVersion: number): WritingState => ({ mode, id: null, body: '', savedBody: '', kind: 'note', citations: [], expectedVersion, expectedRevision: 0, needsRead: false, loading: false, error: '', conflict: false, noteOrigin: 'human', modelName: '' });
function noteState(note: LibraryNote, expected: number): WritingState {
  const comparison = note.comparison ? { title: note.comparison.title, point_texts: note.comparison.points.map(p => p.text) } : undefined;
  const analysis = note.analysis ? analysisInput(note.analysis) : undefined;
  return { ...blank('notes', expected), id: note.id, body: note.body, savedBody: note.body, kind: note.kind, note, expectedRevision: note.revision, citations: note.citations, comparison, savedComparison: comparison ? JSON.stringify(comparison) : undefined, analysis, savedAnalysis: analysis ? JSON.stringify(analysis) : undefined, noteOrigin: note.origin, modelName: note.model_name ?? '' };
}

export function useLibraryWriting(detail: LibraryAssetDetail, active: boolean, navigation: LibraryEditorNavigation) {
  const assetId = detail.asset.id;
  const latest = Math.max(0, ...detail.documents.filter(d => d.kind === 'edited').map(d => d.version));
  const [state, setState] = useState<WritingState>(() => blank('recognized', latest));
  const [saving, setSaving] = useState(false);
  const [recovered, setRecovered] = useState<string[]>([]);
  const generation = useRef(0);
  const savePending = useRef(false);
  const latestDetail = useRef(detail); latestDetail.current = detail;
  const dirty = state.mode !== 'recognized' && (!!state.comparison && JSON.stringify(state.comparison) !== state.savedComparison || state.body !== state.savedBody || !!state.analysis && JSON.stringify(state.analysis) !== state.savedAnalysis || !state.id && (state.citations.length > 0 || !!state.analysis && (state.noteOrigin === 'ai' || !!state.modelName)));
  const canSave = dirty && (state.comparison ? !!state.comparison.title.trim() && state.comparison.point_texts.every(t => !!t.trim()) : state.analysis ? analysisCanSave(state.analysis) && (!!state.id || state.noteOrigin === 'human' || !!state.modelName.trim()) : !!state.body.trim()) && !state.loading && !state.conflict;
  async function load(mode: Mode, id: string | null, preserveOnFailure = false) {
    const token = ++generation.current;
    const expected = Math.max(0, ...latestDetail.current.documents.filter(d => d.kind === 'edited').map(d => d.version));
    setState(old => preserveOnFailure ? { ...old, loading: !!id, error: '' } : { ...blank(mode, expected), id, loading: !!id });
    if (!id) return;
    try {
      if (mode === 'notes') {
        const note = await libraryApi.note(assetId, id);
        if (token === generation.current) setState(noteState(note, expected));
      } else {
        const value = await readLibraryDocument(assetId, id);
        if (token === generation.current) setState({ ...blank(mode, expected), id, ...value, body: value.document.text, savedBody: value.document.text });
      }
    } catch (reason) { if (token === generation.current) setState(old => ({ ...old, loading: false, needsRead: !!id, error: errorText(reason) })); }
  }
  useEffect(() => {
    const first = latestDetail.current.documents.find(d => d.kind === 'recognized') ?? latestDetail.current.documents[0];
    void load(first?.kind ?? 'recognized', first?.id ?? null);
    return () => { generation.current++; };
  }, [assetId]);
  const fileKey = JSON.stringify(detail.files);
  useEffect(() => {
    if (!state.id || state.mode === 'notes' || state.loading) return;
    let disposed = false;
    const id = state.id;
    void readLibraryDocument(assetId, id).then(value => {
      if (!disposed) setState(old => old.id === id ? { ...old, ...value } : old);
    }).catch(reason => { if (!disposed) setState(old => old.id === id ? { ...old, error: errorText(reason) } : old); });
    return () => { disposed = true; };
  }, [assetId, fileKey]);

  const save = useCallback(async () => {
    if (savePending.current || !canSave) return false;
    savePending.current = true; setSaving(true); setState(old => ({ ...old, error: '' }));
    try {
      if (state.mode === 'edited') {
        const id = await libraryApi.saveDocument(assetId, state.body, state.expectedVersion, state.document?.summary.source_document_id ?? state.document?.summary.id ?? null);
        setState(old => ({ ...old, id, savedBody: state.body, expectedVersion: state.expectedVersion + 1, document: undefined, conflict: false }));
        try { const value = await readLibraryDocument(assetId, id); setState(old => ({ ...old, ...value, needsRead: false })); }
        catch (reason) { setState(old => ({ ...old, needsRead: true, error: `内容已保存，重新读取失败：${errorText(reason)}` })); }
      } else if (state.mode === 'notes') {
        const id = state.id ?? (state.analysis ? await libraryApi.createAnalysis(assetId, { content: state.analysis, origin: state.noteOrigin, model_name: state.noteOrigin === 'ai' ? state.modelName.trim() : null, citations: state.citations }) : await libraryApi.createNote(assetId, { kind: state.kind, body: state.body, citations: state.citations }));
        if (state.id) { if (state.comparison) await libraryApi.editComparison(assetId, state.id, state.comparison, state.expectedRevision); else if (state.analysis) await libraryApi.editAnalysis(assetId, state.id, state.analysis, state.expectedRevision); else await libraryApi.editNote(assetId, state.id, state.body, state.expectedRevision); }
        setState(old => ({ ...old, id, savedBody: state.body, savedComparison: state.comparison ? JSON.stringify(state.comparison) : undefined, savedAnalysis: state.analysis ? JSON.stringify(state.analysis) : undefined, expectedRevision: state.id ? state.expectedRevision + 1 : 1, conflict: false }));
        try { const note = await libraryApi.note(assetId, id); setState(noteState(note, state.expectedVersion)); }
        catch (reason) { setState(old => ({ ...old, needsRead: true, error: `内容已保存，重新读取失败：${errorText(reason)}` })); }

      }
      toast.success('已保存'); return true;
    } catch (reason) {
      const conflict = String(reason) === 'DOCUMENT_CONFLICT' || String(reason) === 'NOTE_CONFLICT';
      setState(old => ({ ...old, conflict, error: conflict ? '内容已更新，请重新读取后再保存' : String(reason) === 'INVALID_INPUT' ? '内容不能为空，且不能超过 16 MiB' : errorText(reason) })); return false;
    } finally { savePending.current = false; setSaving(false); }
  }, [assetId, canSave, state]);
  const discard = useCallback(() => { setRecovered([]); setState(old => ({ ...old, body: old.savedBody, comparison: old.savedComparison ? JSON.parse(old.savedComparison) as ComparisonEdit : undefined, analysis: old.savedAnalysis ? JSON.parse(old.savedAnalysis) as AnalysisInput : undefined, noteOrigin: old.note?.origin ?? 'human', modelName: old.note?.model_name ?? '', citations: old.id ? old.citations : [], conflict: false, error: '' })); }, []);
  useLayoutEffect(() => { navigation.report(active ? { dirty: dirty || recovered.length > 0, busy: saving, canSave: canSave && recovered.length === 0, save, discard } : null); }, [active, dirty, saving, canSave, save, discard, navigation.report, recovered.length]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => {
    if (!dirty && !recovered.length) return;
    const beforeUnload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, [dirty, recovered.length]);
  function select(mode: Mode, id: string | null) { navigation.request(() => { void load(mode, id); }); }
  function createDraft() {
    const current = state.document;
    navigation.request(() => { generation.current++; setState({ ...blank('edited', latest), body: current?.text ?? '', document: current, origin: state.origin }); });
  }
  function changeNoteKind(kind: string) {
    const analysis = blankAnalysis(kind, state.body);
    setState(old => ({ ...old, kind, analysis, savedAnalysis: analysis ? JSON.stringify(blankAnalysis(kind)) : undefined }));
  }
  function createNote(citation?: LibraryCitation) { navigation.request(() => { generation.current++; setState({ ...blank('notes', latest), citations: citation ? [citation] : [] }); }); }
  async function reloadConflict() {
    const token = ++generation.current;
    const original = state.comparison ? JSON.stringify({ edit: state.comparison, comparison: state.note?.comparison }, null, 2) : state.analysis ? JSON.stringify({ content: state.analysis, origin: state.noteOrigin, model_name: state.modelName, citations: state.citations }, null, 2) : state.body;
    setState(old => ({ ...old, loading: true }));
    try {
      if (state.mode === 'notes' && state.id) {
        const note = await libraryApi.note(assetId, state.id);
        if (token !== generation.current) return;
        setState(noteState(note, latest));
      } else {
        const documents = await libraryApi.documents(assetId);
        const newest = documents.filter(d => d.kind === state.mode).sort((a, b) => b.version - a.version)[0];
        if (!newest) throw 'DOCUMENT_NOT_FOUND';
        const value = await readLibraryDocument(assetId, newest.id);
        if (token !== generation.current) return;
        setState({ ...blank(state.mode, state.mode === 'edited' ? newest.version : latest), id: newest.id, ...value, body: value.document.text, savedBody: value.document.text });
      }
      if (dirty) setRecovered(old => old.includes(original) ? old : [...old, original]);
    } catch (reason) { if (token === generation.current) setState(old => ({ ...old, loading: false, needsRead: !!old.id, error: errorText(reason) })); }
  }
  return { state, setState, changeNoteKind, reloadSaved: () => navigation.request(() => { void load(state.mode, state.id, true); }), dirty, canSave, saving, save, select, createDraft, createNote, reloadConflict, recovered };
}
