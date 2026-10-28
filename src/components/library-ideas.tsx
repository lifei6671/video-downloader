import { useEffect, useRef, useState } from 'react';
import { AiSourceView } from '@/components/library-ai-source';
import { Plus } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type IdeaReferenceInput, type IdeaReferenceView, type IdeaSummary, type TopicMemberKind } from '@/lib/library-api';
import { ideaReferenceKey, useLibraryIdea } from '@/hooks/use-library-idea';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryError } from '@/components/library-organization';
import { TopicMemberPicker } from '@/components/library-topic-picker';
import { IdeaReferenceReader, IdeaTopicPicker } from '@/components/library-idea-references';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';

const materialKinds: TopicMemberKind[] = ['video', 'clip'];
const referenceNames = { video: '视频', clip: '片段', topic: '研究专题' };
export function LibraryIdeas({ initialId, seed, active, revision, navigation, back, openAsset }: { initialId?: string; seed?: IdeaReferenceInput; active: boolean; revision: number; navigation: LibraryEditorNavigation; back: () => void; openAsset: (id: string, trigger: HTMLElement | null) => void }) {
  const editor = useLibraryIdea(initialId, seed, active, revision, navigation);
  const { state, setState } = editor;
  const [ideas, setIdeas] = useState<IdeaSummary[]>([]);
  const [listError, setListError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [picker, setPicker] = useState<'material' | 'topic' | null>(null);
  const [source, setSource] = useState<IdeaReferenceView>();
  const [actionError, setActionError] = useState('');
  const addTrigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void libraryApi.ideas().then(value => { if (!disposed) { setIdeas(value); setListError(''); } }).catch(reason => { if (!disposed) setListError(errorText(reason)); });
    return () => { disposed = true; };
  }, [active, revision, refresh, state.id, state.expectedRevision]);
  const disabled = state.loading || state.needsRead || editor.saving || editor.referenceBusy;
  function closePicker() { setPicker(null); requestAnimationFrame(() => addTrigger.current?.focus({ preventScroll: true })); }
  function viewAsset(id: string, trigger: HTMLElement | null) { navigation.request(() => openAsset(id, trigger)); }
  async function copy(text: string) { try { await navigator.clipboard.writeText(text); toast.success('已复制'); } catch (reason) { setActionError(errorText(reason)); } }
  return <section className="library-ideas" tabIndex={-1} hidden={!active} aria-label="选题与草稿">
    <div className="library-section-heading"><Button size="sm" variant="ghost" onClick={() => navigation.request(back)}>返回知识库</Button><h2>选题与草稿</h2></div>
    <div className="ideas-layout"><aside aria-label="选题列表"><Button size="sm" variant="outline" disabled={editor.saving || editor.referenceBusy} onClick={() => editor.select(null)}><Plus />新建选题</Button><LibraryError message={listError} />{listError && <Button size="sm" variant="outline" onClick={() => setRefresh(old => old + 1)}>重新读取选题列表</Button>}<ul>{ideas.map(idea => <li key={idea.id}><Button className="idea-title" variant="ghost" title={idea.title} aria-pressed={state.id === idea.id} disabled={state.id === idea.id || editor.saving || editor.referenceBusy} onClick={() => editor.select(idea.id)}>{idea.title}</Button></li>)}</ul>{!ideas.length && !listError && <p className="text-xs text-muted-foreground">暂无已保存选题</p>}</aside>
    <div className="idea-editor" aria-busy={state.loading}><LibraryError message={state.error || actionError} />{state.loading && <p role="status">读取中…</p>}
      {state.needsRead && state.id && <Button className="self-start" size="sm" variant="outline" disabled={editor.saving} onClick={() => void editor.reloadLatest()}>重新读取已保存草稿</Button>}
      {(state.conflict || state.serverRevision > state.expectedRevision) && <Button className="self-start" size="sm" variant="outline" disabled={state.loading || editor.saving} onClick={() => void editor.reloadLatest()}>读取最新草稿并保留当前内容</Button>}
      <AiSourceView source={state.aiSource} />
      <label>选题<Input aria-label="选题标题" maxLength={128} value={state.title} disabled={disabled} onChange={e => setState(old => ({ ...old, title: e.target.value }))} /></label>
      <label>观点<Textarea aria-label="自己的观点" value={state.viewpoint} disabled={disabled} onChange={e => setState(old => ({ ...old, viewpoint: e.target.value }))} /></label>
      <label>草稿<Textarea className="idea-body" aria-label="自己的草稿" value={state.body} disabled={disabled} onChange={e => setState(old => ({ ...old, body: e.target.value }))} /></label>
      <div className="library-writing-toolbar"><Button size="sm" disabled={!editor.canSave} onClick={() => void editor.save()}>{editor.saving ? '保存中…' : '保存草稿'}</Button>{editor.changed && <span role="status" className="text-xs text-muted-foreground">未保存</span>}{state.id && !editor.changed && !state.loading && <span role="status" className="text-xs text-muted-foreground">已保存 r{state.expectedRevision}</span>}</div>
      <div className="library-section-heading"><h3>参考资料</h3><DropdownMenu><DropdownMenuTrigger asChild><Button ref={addTrigger} size="sm" variant="outline" disabled={disabled}><Plus />添加参考</Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuItem onSelect={() => setPicker('material')}>视频或片段</DropdownMenuItem><DropdownMenuItem onSelect={() => setPicker('topic')}>研究专题</DropdownMenuItem></DropdownMenuContent></DropdownMenu></div>
      {!state.references.length && !state.loading && <p className="text-xs text-muted-foreground">暂无参考资料</p>}
      <ul className="idea-references">{state.references.map(r => <li key={ideaReferenceKey(r.reference)} data-idea-reference={r.reference.resource_id}><div className="idea-reference-info"><span className="text-xs text-muted-foreground">{referenceNames[r.reference.kind]}{r.topic_revision != null ? ` r${r.topic_revision}` : ''}</span><strong title={r.title}>{r.title || '来源标题未知'}</strong>{r.reference.kind === 'clip' && <span className="idea-reference-owner text-xs text-muted-foreground" title={r.asset_title ?? ''}>{r.asset_title || '来源未知'}</span>}<div className="flex flex-wrap gap-2">{r.missing && <Badge variant="outline">引用缺失</Badge>}{r.archived && <Badge variant="outline">已移出知识库</Badge>}{r.file_state === 'missing' && <Badge variant="outline">文件缺失</Badge>}{r.file_state === 'changed' && <Badge variant="outline">文件已变化</Badge>}{r.file_state === 'unknown' && <Badge variant="outline">文件状态未知</Badge>}</div></div><div className="library-file-actions"><Button size="sm" variant="outline" disabled={disabled || r.missing} onClick={event => { if (r.reference.kind === 'video' && r.reference.asset_id) viewAsset(r.reference.asset_id, event.currentTarget); else setSource(r); }}>打开来源</Button><Button size="sm" variant="ghost" disabled={disabled} onClick={() => { setState(old => ({ ...old, references: old.references.filter(ref => ideaReferenceKey(ref.reference) !== ideaReferenceKey(r.reference)) })); toast.info('参考已移除，保存后生效'); }}>移除参考</Button></div></li>)}</ul>
      {editor.recovered.length > 0 && <details className="library-recovered-drafts"><summary>保留的草稿副本</summary>{editor.recovered.map((text, i) => <div key={i}><Textarea aria-label={`保留的草稿 ${i + 1}`} readOnly value={text} /><Button size="sm" variant="outline" onClick={() => void copy(text)}>复制保留草稿 {i + 1}</Button></div>)}<Button size="sm" variant="ghost" disabled={disabled} onClick={editor.clearRecovered}>移除保留副本</Button></details>}
    </div></div>
    {picker === 'material' && <TopicMemberPicker context="参考" allowedKinds={materialKinds} busy={editor.referenceBusy} blocked={false} actionError={state.error} add={input => editor.addReference({ kind: input.kind as 'video' | 'clip', asset_id: input.asset_id, resource_id: input.resource_id })} close={closePicker} />}
    {picker === 'topic' && <IdeaTopicPicker busy={editor.referenceBusy} error={state.error} add={editor.addReference} close={closePicker} />}
    {source && <IdeaReferenceReader source={source} close={() => setSource(undefined)} openVideo={id => viewAsset(id, null)} />}
  </section>;
}
