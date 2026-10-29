import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { libraryApi, type ComparisonPin, type ComparisonSource, type LibraryAssetDetail, type LibraryClip, type LibraryNote } from '@/lib/library-api';
import { errorText } from '@/api';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryError } from '@/components/library-organization';
import { AnalysisSnapshot, ComparisonSourceView } from '@/components/library-comparison-source';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';

type Column = { detail: LibraryAssetDetail; clips: LibraryClip[]; source: ComparisonSource };
type Point = { text: string; sources: ComparisonSource[] };
const emptyPoint = (): Point => ({ text: '', sources: [] });
export function LibraryComparisonWorkspace({ assetIds, active, navigation, back }: { assetIds: string[]; active: boolean; navigation: LibraryEditorNavigation; back: () => void }) {
  const [columns, setColumns] = useState<Record<string, Column>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [title, setTitle] = useState('');
  const [points, setPoints] = useState<Point[]>([emptyPoint()]);
  const [owner, setOwner] = useState(assetIds[0]);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<LibraryNote>();
  const [showSaved, setShowSaved] = useState(false);
  const pending = useRef(false);
  const versions = useRef<Record<string, number>>({});
  const disposed = useRef(false);
  const dirty = !!title || points.some(p => !!p.text || p.sources.length > 0);
  const canSave = dirty && !!title.trim() && points.every(p => !!p.text.trim() && p.sources.length > 0) && !saving;
  async function load(id: string, pin?: ComparisonPin) {
    const token = (versions.current[id] ?? 0) + 1; versions.current[id] = token;
    setLoading(old => ({ ...old, [id]: true })); setErrors(old => ({ ...old, [id]: '' }));
    try {
      const [detail, clips] = await Promise.all([libraryApi.asset(id), libraryApi.clips(id)]);
      if (detail.asset.id !== id || clips.some(c => c.asset_id !== id)) throw Error('来源资产不匹配');
      const latest = (kind: string) => detail.notes.filter(n => n.kind === kind).sort((a, b) => b.updated_at - a.updated_at || b.id.localeCompare(a.id))[0];
      const copywriting = latest('copywriting_analysis'), technique = latest('technique_analysis');
      const document = [...detail.documents].sort((a, b) => b.version - a.version).find(d => d.kind === 'recognized') ?? [...detail.documents].sort((a, b) => b.version - a.version)[0];
      const source = await libraryApi.comparisonSource(pin ?? { asset_id: id, document_id: document?.id ?? null, copywriting: copywriting ? { id: copywriting.id, revision: copywriting.revision } : null, technique: technique ? { id: technique.id, revision: technique.revision } : null, clip_ids: clips.slice(0, 5).map(c => c.id) });
      if (source.pin.asset_id !== id) throw Error('来源资产不匹配');
      if (!disposed.current && versions.current[id] === token) setColumns(old => ({ ...old, [id]: { detail, clips, source } }));
    } catch (reason) { if (!disposed.current && versions.current[id] === token) setErrors(old => ({ ...old, [id]: errorText(reason) })); }
    finally { if (!disposed.current && versions.current[id] === token) setLoading(old => ({ ...old, [id]: false })); }
  }
  useEffect(() => { disposed.current = false; assetIds.forEach(id => { void load(id); }); return () => { disposed.current = true; }; }, [assetIds]);
  const discard = useCallback(() => { setTitle(''); setPoints([emptyPoint()]); setError(''); }, []);
  const save = useCallback(async () => {
    if (!canSave || pending.current) return false;
    pending.current = true; setSaving(true); setError('');
    try {
      const id = await libraryApi.createComparison(owner, { title, asset_ids: assetIds, points: points.map(p => ({ text: p.text, sources: p.sources.map(s => s.pin) })) });
      // Commit succeeded. A follow-up read failure must not cause duplicate creation.
      discard(); setSaved(undefined); toast.success(`比较笔记已保存到 ${columns[owner]?.detail.asset.title || '所选视频'}`);
      try { setSaved(await libraryApi.note(owner, id)); setShowSaved(true); }
      catch (reason) { setError(`比较笔记已保存，读取失败：${errorText(reason)}`); }
      return true;
    } catch (reason) { setError(errorText(reason)); return false; }
    finally { pending.current = false; setSaving(false); }
  }, [canSave, owner, title, points, assetIds, columns, discard]);
  useLayoutEffect(() => { navigation.report(active ? { dirty, busy: saving, canSave, save, discard } : null); }, [navigation.report, active, dirty, saving, canSave, save, discard]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => { if (!dirty) return; const guard = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ''; }; window.addEventListener('beforeunload', guard); return () => window.removeEventListener('beforeunload', guard); }, [dirty]);
  function point(index: number, update: (p: Point) => Point) { setPoints(old => old.map((p, i) => i === index ? update(p) : p)); }
  return <section className="library-comparison" aria-label="跨视频对比工作区">
    <div className="library-section-heading"><Button size="sm" variant="ghost" onClick={() => navigation.request(back)}>返回知识库</Button><h2>跨视频对比</h2></div>
    <div className="comparison-scroll" onFocusCapture={event => event.target.scrollIntoView({ block: 'nearest', inline: 'nearest' })}><div className="comparison-grid" style={{ gridTemplateColumns: `repeat(${assetIds.length}, minmax(260px, 1fr))` }}>{assetIds.map(id => {
      const c = columns[id], source = c?.source;
      const choose = (patch: Partial<ComparisonPin>) => { if (source) void load(id, { ...source.pin, ...patch }); };
      return <article className="comparison-column" key={id} data-comparison-asset={id} aria-busy={!!loading[id]}><header><h3 title={c?.detail.asset.title}>{c?.detail.asset.title || '读取视频…'}</h3><LibraryError message={errors[id]} />{loading[id] && <p role="status">读取中…</p>}<Button size="sm" variant="ghost" disabled={loading[id]} onClick={() => void load(id)}>读取最新来源</Button></header>
        {source && <><section><h4>原文首段</h4><label>文案版本<select aria-label={`文案版本 ${id}`} value={source.pin.document_id ?? ''} disabled={loading[id]} onChange={e => choose({ document_id: e.target.value || null })}><option value="">未知</option>{c.detail.documents.map(d => <option key={d.id} value={d.id}>{d.kind === 'recognized' ? '识别原文' : '整理稿'} v{d.version}</option>)}</select></label><p>{source.document?.opening || '未知'}</p></section>
        {(['copywriting', 'technique'] as const).map(kind => <section key={kind}><h4>{kind === 'copywriting' ? '文案结构' : '制作手法拆解'}</h4><label>拆解版本<select aria-label={`${kind === 'copywriting' ? '文案结构' : '制作手法'}版本 ${id}`} value={source.pin[kind]?.id ?? ''} disabled={loading[id]} onChange={e => { const note = c.detail.notes.find(n => n.id === e.target.value); choose({ [kind]: note ? { id: note.id, revision: note.revision } : null }); }}><option value="">未知</option>{c.detail.notes.filter(n => n.kind === `${kind}_analysis`).map(n => <option key={n.id} value={n.id}>{n.origin === 'ai' ? 'AI' : '人工'} r{n.revision} [{n.id.slice(0, 8)}]</option>)}</select></label><AnalysisSnapshot value={source[kind]} /></section>)}
        <section><h4>片段</h4>{!c.clips.length && <p>未知</p>}{c.clips.map(clip => <label className="library-choice" key={clip.id}><Checkbox aria-label={`对比片段 ${id} ${clip.id}`} checked={source.pin.clip_ids.includes(clip.id)} disabled={loading[id] || !source.pin.clip_ids.includes(clip.id) && source.pin.clip_ids.length >= 8} onCheckedChange={checked => choose({ clip_ids: checked ? [...source.pin.clip_ids, clip.id] : source.pin.clip_ids.filter(v => v !== clip.id) })} /><span>{clip.title} {clip.start_ms}–{clip.end_ms} ms{clip.missing ? ' 文件缺失' : clip.source_changed ? ' 文件已变化' : ''}</span></label>)}{!source.clips.length && !!c.clips.length && <p>未知</p>}</section><ComparisonSourceView source={source} compact /></>}
      </article>;
    })}</div></div>
    <section className="comparison-notes" aria-label="比较笔记"><h3>比较笔记</h3><label>标题<Input aria-label="比较标题" maxLength={100} disabled={saving} value={title} onChange={e => setTitle(e.target.value)} /></label>
      {points.map((p, index) => <div className="comparison-point" key={index}><label>结论 {index + 1}<Textarea aria-label={`比较结论 ${index + 1}`} disabled={saving} value={p.text} onChange={e => point(index, old => ({ ...old, text: e.target.value }))} /></label><fieldset disabled={saving}><legend>结论来源</legend>{assetIds.map(id => <label className="library-choice" key={id}><Checkbox aria-label={`结论 ${index + 1} 来源 ${id}`} checked={p.sources.some(s => s.pin.asset_id === id)} disabled={saving || !columns[id] || loading[id] || !!errors[id]} onCheckedChange={checked => point(index, old => ({ ...old, sources: checked ? [...old.sources, structuredClone(columns[id].source)] : old.sources.filter(s => s.pin.asset_id !== id) }))} /><span>{columns[id]?.detail.asset.title || id}</span></label>)}</fieldset>{p.sources.map(s => <ComparisonSourceView key={s.pin.asset_id} source={s} />)}{points.length > 1 && <Button size="sm" variant="ghost" disabled={saving} onClick={() => setPoints(old => old.filter((_, i) => i !== index))}>删除结论 {index + 1}</Button>}</div>)}
      <div className="library-writing-toolbar"><Button size="sm" variant="outline" disabled={saving || points.length >= 24} onClick={() => setPoints(old => [...old, emptyPoint()])}>添加结论</Button><label>保存到视频<select aria-label="比较笔记所属视频" disabled={saving} value={owner} onChange={e => setOwner(e.target.value)}>{assetIds.map(id => <option key={id} value={id}>{columns[id]?.detail.asset.title || id}</option>)}</select></label><Button size="sm" disabled={!canSave} onClick={() => void save()}>{saving ? '保存中…' : '保存比较笔记'}</Button>{saved && <Button size="sm" variant="outline" onClick={() => setShowSaved(true)}>查看已保存比较</Button>}</div><LibraryError message={error} />
    </section>
    {showSaved && saved?.comparison && <Dialog open onOpenChange={setShowSaved}><DialogContent className="comparison-source-dialog"><DialogHeader><DialogTitle>{saved.comparison.title}</DialogTitle></DialogHeader>{saved.comparison.points.map((p, i) => <div key={i}><p className="whitespace-pre-wrap">{p.text}</p>{p.sources.map(s => <ComparisonSourceView key={s.pin.asset_id} source={s} />)}</div>)}</DialogContent></Dialog>}
  </section>;
}
