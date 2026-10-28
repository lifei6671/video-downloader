import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type AiSource, type IdeaDetail, type IdeaInput, type IdeaReferenceInput, type IdeaReferenceView } from '@/lib/library-api';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';

export const ideaReferenceKey = (r: IdeaReferenceInput) => JSON.stringify([r.kind, r.asset_id, r.resource_id]);
interface State {
  aiSource?: AiSource;
  id: string | null; title: string; viewpoint: string; body: string;
  references: IdeaReferenceView[]; savedReferences: IdeaReferenceView[]; saved: string;
  expectedRevision: number; serverRevision: number; loading: boolean; needsRead: boolean; conflict: boolean; error: string;
}
const blankInput = (): IdeaInput => ({ title: '', viewpoint: '', body: '', references: [] });
const blank = (id: string | null = null): State => ({ ...blankInput(), references: [], savedReferences: [], id, saved: JSON.stringify(blankInput()), expectedRevision: 0, serverRevision: 0, loading: false, needsRead: false, conflict: false, error: '' });
function payload(s: State): IdeaInput { return { title: s.title, viewpoint: s.viewpoint, body: s.body, references: s.references.map(r => r.reference) }; }
function fromDetail(value: IdeaDetail): State {
  const s = { ...blank(value.idea.id), aiSource: value.idea.ai_source, title: value.idea.title, viewpoint: value.idea.viewpoint, body: value.idea.body, references: value.references, savedReferences: value.references, expectedRevision: value.idea.revision, serverRevision: value.idea.revision };
  return { ...s, saved: JSON.stringify(payload(s)) };
}
export function useLibraryIdea(initialId: string | undefined, seed: IdeaReferenceInput | undefined, active: boolean, revision: number, navigation: LibraryEditorNavigation) {
  const [state, setState] = useState<State>(() => blank(initialId ?? null));
  const [saving, setSaving] = useState(false);
  const [referenceBusy, setReferenceBusy] = useState(false);
  const [recovered, setRecovered] = useState<string[]>([]);
  const generation = useRef(0);
  const writePending = useRef(false);
  const referencePending = useRef(false);
  const mounted = useRef(true);
  const changed = JSON.stringify(payload(state)) !== state.saved;
  const dirty = changed || recovered.length > 0;
  const canSave = changed && !!state.title.trim() && !state.loading && !state.needsRead && !state.conflict && !saving && !referenceBusy;
  async function read(id: string, token: number) {
    const value = await libraryApi.idea(id);
    if (value.idea.id !== id) throw Error('选题资料不匹配');
    if (token === generation.current && mounted.current) setState(fromDetail(value));
    return value;
  }
  async function load(id: string | null) {
    const token = ++generation.current; setRecovered([]); setState({ ...blank(id), loading: !!id });
    if (!id) return;
    try { await read(id, token); }
    catch (reason) { if (token === generation.current && mounted.current) setState(old => ({ ...old, loading: false, needsRead: true, error: errorText(reason) })); }
  }
  useEffect(() => {
    mounted.current = true;
    if (initialId) void load(initialId);
    else if (seed) {
      const token = ++generation.current; setState(old => ({ ...old, loading: true }));
      void libraryApi.prepareIdeaReference(seed).then(value => { if (token === generation.current) setState(old => ({ ...old, references: [value], loading: false })); })
        .catch(reason => { if (token === generation.current) setState(old => ({ ...old, loading: false, error: `参考资料未加入：${errorText(reason)}` })); });
    }
    return () => { mounted.current = false; generation.current++; };
  }, []);
  useEffect(() => {
    if (!active || !state.id || state.loading || state.needsRead) return;
    const id = state.id; let disposed = false;
    void libraryApi.idea(id).then(value => {
      if (disposed || value.idea.id !== id) return;
      const live = new Map(value.references.map(r => [ideaReferenceKey(r.reference), r]));
      setState(old => old.id !== id || value.idea.revision < old.expectedRevision ? old : { ...old, serverRevision: value.idea.revision, references: old.references.map(r => live.get(ideaReferenceKey(r.reference)) ?? r) });
    }).catch(reason => { if (!disposed) setState(old => old.id !== id ? old : { ...old, error: errorText(reason) }); });
    return () => { disposed = true; };
  }, [active, revision, state.id]);
  const save = useCallback(async () => {
    if (!canSave || writePending.current) return false;
    writePending.current = true; setSaving(true); setState(old => ({ ...old, error: '' }));
    try {
      const input = payload(state);
      const id = state.id ?? await libraryApi.createIdea(input);
      if (state.id) await libraryApi.editIdea(state.id, input, state.expectedRevision);
      setState(old => ({ ...old, id, saved: JSON.stringify(input), savedReferences: state.references, expectedRevision: state.id ? state.expectedRevision + 1 : 1, conflict: false, needsRead: true }));
      const token = ++generation.current;
      try { await read(id, token); }
      catch (reason) { setState(old => ({ ...old, loading: false, needsRead: true, error: `草稿已保存，重新读取失败：${errorText(reason)}` })); }
      toast.success('选题已保存'); return true;
    } catch (reason) {
      setState(old => ({ ...old, conflict: String(reason) === 'IDEA_CONFLICT', error: errorText(reason) })); return false;
    } finally { writePending.current = false; setSaving(false); }
  }, [canSave, state]);
  const discard = useCallback(() => {
    setRecovered([]); setState(old => { const saved = JSON.parse(old.saved) as IdeaInput; return { ...old, title: saved.title, viewpoint: saved.viewpoint, body: saved.body, references: old.savedReferences, conflict: false, error: '' }; });
  }, []);
  useLayoutEffect(() => { navigation.report(active ? { dirty, busy: saving || referenceBusy, canSave: canSave && !recovered.length, save, discard } : null); }, [active, dirty, saving, referenceBusy, canSave, recovered.length, save, discard, navigation.report]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => { if (!dirty) return; const guard = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ''; }; window.addEventListener('beforeunload', guard); return () => window.removeEventListener('beforeunload', guard); }, [dirty]);
  async function reloadLatest() {
    if (!state.id) return;
    const local = changed ? JSON.stringify({ ...payload(state), expected_revision: state.expectedRevision }, null, 2) : null;
    const token = ++generation.current; setState(old => ({ ...old, loading: true }));
    try { await read(state.id, token); if (token === generation.current && local) setRecovered(old => [...old, local]); }
    catch (reason) { if (token === generation.current) setState(old => ({ ...old, loading: false, error: errorText(reason) })); }
  }
  async function addReference(input: IdeaReferenceInput) {
    if (referencePending.current || saving) return false;
    if (state.references.some(r => ideaReferenceKey(r.reference) === ideaReferenceKey(input))) { setState(old => ({ ...old, error: errorText('IDEA_REFERENCE_EXISTS') })); return false; }
    referencePending.current = true; setReferenceBusy(true); setState(old => ({ ...old, error: '' }));
    try {
      const value = await libraryApi.prepareIdeaReference(input);
      if (!mounted.current) return false;
      setState(old => ({ ...old, references: [...old.references, value] })); toast.success('已加入参考列表'); return true;
    } catch (reason) { if (mounted.current) setState(old => ({ ...old, error: `参考资料未加入：${errorText(reason)}` })); return false; }
    finally { referencePending.current = false; if (mounted.current) setReferenceBusy(false); }
  }
  return { state, setState, saving, referenceBusy, dirty, changed, canSave, recovered, clearRecovered: () => { setRecovered([]); toast.success('已移除保留副本'); }, save, addReference, reloadLatest, select: (id: string | null) => navigation.request(() => { void load(id); }) };
}
