import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { BookmarkPlus, Play, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryCitation, type LibraryClip, type LibraryFile, type IdeaReferenceInput } from '@/lib/library-api';
import { clipTime, coveringRange } from '@/lib/clip-playback';
import type { LibraryVideoHandle } from '@/components/library-video';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { CitationReader } from '@/components/library-writing';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Spinner } from '@/components/ui/spinner';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from '@/components/ui/dropdown-menu';

const milliseconds = (value: string) => /^\d+(?:\.\d{1,3})?$/.test(value) ? Math.round(Number(value) * 1000) : NaN;
export interface ClipDraft { citation?: LibraryCitation; sequence: number; range?: { start_ms: number; end_ms: number } }

export function LibraryClips({ onCreateIdea, assetId, file, active, revision, player, ready, draft, setDraft, ensurePlayer, play, navigation, extractFrames, framesBusy }: {
  onCreateIdea?: (input: IdeaReferenceInput) => void; assetId: string; file?: LibraryFile; active: boolean; revision: number;
  player: React.RefObject<LibraryVideoHandle | null>; ready: number;
  draft: ClipDraft | null; setDraft: (draft: ClipDraft | null) => void;
  ensurePlayer: () => void; play: (clip: LibraryClip) => void; navigation: LibraryEditorNavigation;
  extractFrames?: (clip: LibraryClip, count: number) => void; framesBusy?: boolean;
}) {
  const [clips, setClips] = useState<LibraryClip[]>([]);
  const [reload, setReload] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [title, setTitle] = useState('');
  const [reason, setReason] = useState('');
  const [start, setStart] = useState('');
  const [end, setEnd] = useState('');
  const [saving, setSaving] = useState(false);
  const pending = useRef(false);
  const [deleting, setDeleting] = useState('');
  const [citation, setCitation] = useState<LibraryCitation | null>(null);
  const titleInput = useRef<HTMLInputElement>(null);
  const clock = player.current?.clock();
  const startMs = milliseconds(start), endMs = milliseconds(end);
  const range = coveringRange(clock?.ranges ?? [], startMs, endMs);
  const canSave = !!file && !file.missing && !file.changed && !!title.trim() && !!range;
  useEffect(() => {
    if (!active) return;
    let disposed = false; setLoading(true);
    void libraryApi.clips(assetId).then(value => { if (!disposed) { setClips(value); setError(''); } })
      .catch(reason => { if (!disposed) setError(errorText(reason)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [assetId, active, revision, reload]);
  useEffect(() => {
    if (!draft) return;
    setTitle('片段收藏'); setReason(''); setStart(draft.range ? (draft.range.start_ms / 1000).toFixed(3) : ''); setEnd(draft.range ? (draft.range.end_ms / 1000).toFixed(3) : ''); setError('');
    titleInput.current?.focus();
  }, [draft?.sequence]);
  const discard = useCallback(() => setDraft(null), [setDraft]);
  const save = useCallback(async () => {
    if (pending.current || !file || !draft) return false;
    const playable = coveringRange(player.current?.clock()?.ranges ?? [], startMs, endMs);
    if (!playable) { setError('起止时间须在同一可播放范围内，且结束晚于开始'); return false; }
    pending.current = true; setSaving(true); setError('');
    try {
      await libraryApi.createClip(assetId, { file_id: file.id, title, reason, start_ms: startMs, end_ms: endMs, playable, document_id: draft.citation?.document_id ?? null, quote: draft.citation?.quote ?? null });
      toast.success('片段已收藏'); setDraft(null); setReload(value => value + 1); return true;
    } catch (reason) { setError(errorText(reason)); return false; }
    finally { pending.current = false; setSaving(false); }
  }, [assetId, file, draft, startMs, endMs, player, title, reason, setDraft]);
  useLayoutEffect(() => { navigation.report(active ? { dirty: !!draft, busy: saving || !!deleting, canSave, save, discard } : null); }, [active, draft, saving, deleting, canSave, save, discard, navigation.report]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => {
    if (!draft) return;
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', warn); return () => window.removeEventListener('beforeunload', warn);
  }, [draft]);
  function mark(which: 'start' | 'end') {
    const value = player.current?.clock();
    if (!value) { setError('请先加载视频，再标记时间'); return; }
    (which === 'start' ? setStart : setEnd)((value.current_ms / 1000).toFixed(3)); setError('');
  }
  async function remove(clip: LibraryClip) {
    if (deleting || saving) return;
    setDeleting(clip.id); setError('');
    try { await libraryApi.deleteClip(assetId, clip.id); setClips(old => old.filter(value => value.id !== clip.id)); toast.success('片段收藏已删除'); }
    catch (reason) { setError(errorText(reason)); }
    finally { setDeleting(''); }
  }
  // Readiness refreshes the source-bound media clock without frame-by-frame React updates.
  void ready;
  return <section className="library-clips" aria-label="片段收藏" aria-busy={loading}>
    <div className="library-section-heading"><h3>片段收藏</h3><Button size="sm" variant="outline" disabled={!file || file.missing || file.changed || saving || !!draft} onClick={() => { ensurePlayer(); setDraft({ sequence: Date.now() }); }}><BookmarkPlus />收藏片段</Button></div>
    <LibraryError message={error} />
    {draft && <form className="library-clip-editor" onSubmit={event => { event.preventDefault(); void save(); }}>
      <label>标题<Input ref={titleInput} aria-label="片段标题" value={title} onChange={event => setTitle(event.target.value)} disabled={saving} /></label>
      <div className="library-clip-times">
        <label>开始时间（秒）<Input aria-label="片段开始时间" inputMode="decimal" value={start} placeholder="0.000" onChange={event => setStart(event.target.value)} disabled={saving} /><Button type="button" size="sm" variant="ghost" disabled={saving || !clock} onClick={() => mark('start')}>标为开始</Button></label>
        <label>结束时间（秒）<Input aria-label="片段结束时间" inputMode="decimal" value={end} placeholder="0.000" onChange={event => setEnd(event.target.value)} disabled={saving} /><Button type="button" size="sm" variant="ghost" disabled={saving || !clock} onClick={() => mark('end')}>标为结束</Button></label>
      </div>
      <p className="text-xs text-muted-foreground" role="status">{clock?.ranges.length ? `可播放范围：${clock.ranges.map(value => `${clipTime(value.start_ms)}–${clipTime(value.end_ms)}`).join('、')}` : '正在读取可播放范围…'}</p>
      {!!start && !!end && !range && <p role="alert" className="text-xs text-destructive">起止时间须在同一可播放范围内，且结束晚于开始</p>}
      <label>收藏理由<Textarea aria-label="收藏理由" value={reason} onChange={event => setReason(event.target.value)} disabled={saving} /></label>
      {draft.citation && <blockquote className="library-note-citation"><p>{draft.citation.quote}</p></blockquote>}
      <div className="library-file-actions"><Button size="sm" type="submit" disabled={saving || !canSave}>{saving && <Spinner />}保存片段</Button><Button size="sm" variant="ghost" type="button" disabled={saving} onClick={() => navigation.request(() => setDraft(null))}>取消</Button></div>
    </form>}
    {loading && <Spinner aria-label="正在读取片段收藏" />}
    {!loading && !clips.length && <p className="text-xs text-muted-foreground">暂无收藏片段</p>}
    <ul className="library-clip-list">{clips.map(clip => <li key={clip.id}>
      <div className="library-section-heading"><h4>{clip.title}</h4><span className="text-xs text-muted-foreground">{clipTime(clip.start_ms)}–{clipTime(clip.end_ms)}</span></div>
      {clip.reason && <p>{clip.reason}</p>}
      {clip.quote && <blockquote className="library-note-citation"><p>{clip.quote}</p><Button size="sm" variant="ghost" onClick={() => setCitation({ asset_id: assetId, document_id: clip.document_id!, quote: clip.quote! })}>查看引用原文 v{clip.document_version}</Button></blockquote>}
      {(clip.missing || clip.source_changed) && <p className="text-xs text-muted-foreground">{clip.missing ? '源文件缺失' : '源文件已变化，无法按原范围播放'}</p>}
      <div className="library-file-actions">{onCreateIdea && <Button size="sm" variant="outline" disabled={saving || !!deleting} onClick={() => onCreateIdea({ kind: 'clip', asset_id: assetId, resource_id: clip.id })}>用于选题</Button>}<Button size="sm" variant="outline" disabled={clip.missing || clip.source_changed || saving || !!deleting} onClick={() => navigation.request(() => play(clip))}><Play />播放片段</Button>{extractFrames && <DropdownMenu><DropdownMenuTrigger asChild><Button size="sm" variant="outline" aria-label={`提取关键帧 ${clip.title}`} disabled={clip.missing || clip.source_changed || framesBusy}>提取关键帧</Button></DropdownMenuTrigger><DropdownMenuContent>{[3, 5, 9].map(count => <DropdownMenuItem key={count} onSelect={() => extractFrames(clip, count)}>提取 {count} 帧</DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>}<Button size="icon-sm" variant="ghost" aria-label={`删除片段 ${clip.title}`} disabled={saving || !!deleting} onClick={() => void remove(clip)}>{deleting === clip.id ? <Spinner /> : <Trash2 />}</Button></div>
    </li>)}</ul>
    {citation && <CitationReader citation={citation} close={() => setCitation(null)} />}
  </section>;
}
