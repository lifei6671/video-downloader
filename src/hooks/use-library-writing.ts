import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryAssetDetail, type LibraryDocumentContent, type LibraryNote, type LibraryWritingSelection } from '@/lib/library-api';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';

export async function readLibraryDocument(assetId: string, id: string) {
  const document = await libraryApi.document(assetId, id);
  let origin = document;
  while (!origin.source && origin.summary.source_document_id) origin = await libraryApi.document(assetId, origin.summary.source_document_id);
  return { document, origin };
}
type Mode = 'original' | 'summary';
interface WritingState {
  mode: Mode; id: string | null; body: string; savedBody: string;
  document?: LibraryDocumentContent; note?: LibraryNote;
  expectedVersion: number; expectedRevision: number;
  loading: boolean; needsRead: boolean; error: string; conflict: boolean;
}
const blank = (mode: Mode, expectedVersion: number): WritingState => ({ mode, id: null, body: '', savedBody: '', expectedVersion, expectedRevision: 0, loading: false, needsRead: false, error: '', conflict: false });
const latestDocument = (detail: LibraryAssetDetail) => [...detail.documents].sort((a, b) => b.created_at - a.created_at || (a.kind === b.kind ? b.version - a.version : a.kind === 'edited' ? -1 : 1))[0];
const latestNote = (detail: LibraryAssetDetail) => detail.notes.filter(note => note.origin === 'ai' || ['copywriting', 'technique', 'copywriting_analysis', 'technique_analysis'].includes(note.kind)).sort((a, b) => b.updated_at - a.updated_at || Number(a.kind.endsWith('_analysis')) - Number(b.kind.endsWith('_analysis')))[0];
const editedVersion = (detail: LibraryAssetDetail) => Math.max(0, ...detail.documents.filter(d => d.kind === 'edited').map(d => d.version));

export function useLibraryWriting(detail: LibraryAssetDetail, active: boolean, navigation: LibraryEditorNavigation, selection?: LibraryWritingSelection) {
  const assetId = detail.asset.id;
  const [state, setState] = useState<WritingState>(() => blank('original', editedVersion(detail)));
  const [saving, setSaving] = useState(false);
  const [recovered, setRecovered] = useState<string[]>([]);
  const generation = useRef(0);
  const savePending = useRef(false);
  const latestDetail = useRef(detail); latestDetail.current = detail;
  const dirty = state.body !== state.savedBody;
  const canSave = dirty && !!state.body.trim() && !state.loading && !state.needsRead && !state.conflict;

  async function load(mode: Mode, id: string | null) {
    const token = ++generation.current;
    const expectedVersion = editedVersion(latestDetail.current);
    setState({ ...blank(mode, expectedVersion), id, loading: !!id });
    if (!id) return;
    try {
      if (mode === 'summary') {
        const note = await libraryApi.note(assetId, id);
        if (token === generation.current) setState({ ...blank(mode, expectedVersion), id, note, body: note.body, savedBody: note.body, expectedRevision: note.revision });
      } else {
        const { document } = await readLibraryDocument(assetId, id);
        if (token === generation.current) setState({ ...blank(mode, expectedVersion), id, document, body: document.text, savedBody: document.text });
      }
    } catch (reason) { if (token === generation.current) setState(old => ({ ...old, loading: false, needsRead: true, error: errorText(reason) })); }
  }
  useEffect(() => {
    const first = latestDetail.current.documents.find(d => d.id === selection?.documentId) ?? latestDocument(latestDetail.current);
    void load(selection?.noteId ? 'summary' : 'original', selection?.noteId ?? first?.id ?? null);
    return () => { generation.current++; };
  }, [assetId, selection?.documentId, selection?.noteId]);

  const save = useCallback(async () => {
    if (savePending.current || !canSave) return false;
    savePending.current = true; setSaving(true); setState(old => ({ ...old, error: '' }));
    try {
      let id: string;
      if (state.mode === 'original') {
        // Keep immutable recognition evidence and existing citations intact.
        id = await libraryApi.saveDocument(assetId, state.body, state.expectedVersion, state.document?.summary.source_document_id ?? state.document?.summary.id ?? null);
        setState(old => ({ ...old, id, savedBody: state.body, expectedVersion: state.expectedVersion + 1, document: undefined, needsRead: true }));
        try { const { document } = await readLibraryDocument(assetId, id); setState(old => ({ ...old, document, needsRead: false })); }
        catch (reason) { setState(old => ({ ...old, error: `内容已保存，重新读取失败：${errorText(reason)}` })); }
      } else {
        // Historical structured analyses remain intact; a plain Markdown edit is a new free-text summary.
        const structured = !!state.note?.analysis || !!state.note?.comparison;
        id = state.id && !structured ? state.id : await libraryApi.createNote(assetId, { kind: 'copywriting', body: state.body, citations: state.note?.citations ?? [] });
        if (state.id && !structured) await libraryApi.editNote(assetId, state.id, state.body, state.expectedRevision);
        setState(old => ({ ...old, id, savedBody: state.body, expectedRevision: state.id && !structured ? state.expectedRevision + 1 : 1, needsRead: true }));
        try { const note = await libraryApi.note(assetId, id); setState(old => ({ ...old, note, needsRead: false })); }
        catch (reason) { setState(old => ({ ...old, error: `内容已保存，重新读取失败：${errorText(reason)}` })); }
      }
      toast.success('已保存'); return true;
    } catch (reason) {
      const conflict = String(reason) === 'DOCUMENT_CONFLICT' || String(reason) === 'NOTE_CONFLICT';
      setState(old => ({ ...old, conflict, error: conflict ? '内容已更新，请重新读取后再保存' : errorText(reason) })); return false;
    } finally { savePending.current = false; setSaving(false); }
  }, [assetId, canSave, state]);
  const discard = useCallback(() => { setRecovered([]); setState(old => ({ ...old, body: old.savedBody, conflict: false, error: '' })); }, []);
  useLayoutEffect(() => { navigation.report(active ? { dirty: dirty || recovered.length > 0, busy: saving, canSave: canSave && recovered.length === 0, save, discard } : null); }, [active, dirty, saving, canSave, save, discard, navigation.report, recovered.length]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => {
    if (!dirty && !recovered.length) return;
    const beforeUnload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, [dirty, recovered.length]);
  function select(mode: Mode, id?: string | null) {
    navigation.request(() => { setRecovered([]); void load(mode, id === null ? null : id ?? (mode === 'original' ? latestDocument(latestDetail.current)?.id : latestNote(latestDetail.current)?.id) ?? null); });
  }
  async function reloadConflict() {
    const original = state.body;
    const mode = state.mode;
    const token = ++generation.current;
    setState(old => ({ ...old, loading: true }));
    const latest = await libraryApi.asset(assetId).catch(reason => { if (token === generation.current) setState(old => ({ ...old, loading: false, error: errorText(reason) })); return undefined; });
    if (!latest || token !== generation.current) return;
    latestDetail.current = latest;
    await load(mode, mode === 'original' ? latestDocument(latest)?.id ?? null : state.id);
    if (dirty) setRecovered(old => old.includes(original) ? old : [...old, original]);
  }
  const newer = state.mode === 'original' ? !!latestDocument(detail) && latestDocument(detail)?.id !== state.id : detail.notes.some(n => n.id === state.id && n.revision > state.expectedRevision) || !state.id && !!latestNote(detail);
  return { state, setState, dirty, canSave, saving, save, select, newer, reloadConflict, recovered, restore: (body: string) => { setState(old => ({ ...old, body })); setRecovered([]); }, reloadSaved: () => { void load(state.mode, state.id); } };
}
