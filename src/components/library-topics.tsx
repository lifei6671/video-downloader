import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { AiSourceView } from '@/components/library-ai-source';
import { ArrowDown, ArrowUp, Plus } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type ResearchTopic, type ResearchTopicDetail, type TopicInput, type TopicMemberView, type IdeaReferenceInput } from '@/lib/library-api';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryError } from '@/components/library-organization';
import { TopicMemberPicker } from '@/components/library-topic-picker';
import { TopicSourceDialog } from '@/components/library-topic-source';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';

const empty = (): TopicInput => ({ title: '', description: '' });
const kindNames = { video: '视频', clip: '片段', note: '笔记' };
export function LibraryTopics({ onCreateIdea, active, revision, navigation, back, openAsset }: { onCreateIdea?: (input: IdeaReferenceInput) => void; active: boolean; revision: number; navigation: LibraryEditorNavigation; back: () => void; openAsset: (id: string, trigger: HTMLElement) => void }) {
  const [topics, setTopics] = useState<ResearchTopic[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<ResearchTopicDetail>();
  const [form, setForm] = useState<TopicInput>(empty);
  const [savedForm, setSavedForm] = useState<TopicInput>(empty);
  const [formRevision, setFormRevision] = useState(0);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState('');
  const [error, setError] = useState('');
  const [needsRead, setNeedsRead] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [picker, setPicker] = useState(false);
  const [source, setSource] = useState<TopicMemberView>();
  const generation = useRef(0);
  const pending = useRef(false);
  const pickerTrigger = useRef<HTMLElement | null>(null);
  function closePicker() { setPicker(false); requestAnimationFrame(() => { if (pickerTrigger.current?.isConnected) pickerTrigger.current.focus({ preventScroll: true }); }); }
  const dirty = form.title !== savedForm.title || form.description !== savedForm.description;
  const dirtyRef = useRef(dirty); dirtyRef.current = dirty;
  const canSave = dirty && !!form.title.trim() && !busy && !loading && !needsRead;
  function apply(value: ResearchTopicDetail, reset: boolean) {
    setDetail(value); setNeedsRead(false);
    if (reset || !dirtyRef.current) { const input = { title: value.topic.title, description: value.topic.description }; setForm(input); setSavedForm(input); setFormRevision(value.topic.revision); }
  }
  async function read(id: string, reset = false) {
    const token = ++generation.current; setLoading(true); setError('');
    try {
      const value = await libraryApi.topic(id);
      if (value.topic.id !== id || value.members.some(m => m.member.topic_id !== id)) throw Error('专题资料不匹配');
      if (token === generation.current) apply(value, reset);
    } catch (reason) { if (token === generation.current) { setError(errorText(reason)); setNeedsRead(true); } }
    finally { if (token === generation.current) setLoading(false); }
  }
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void libraryApi.topics().then(value => { if (!disposed) { setTopics(value); setListError(''); } }).catch(reason => { if (!disposed) setListError(errorText(reason)); });
    return () => { disposed = true; };
  }, [active, revision, refresh]);
  useEffect(() => { if (active && selected) void read(selected); return () => { generation.current++; }; }, [active, selected, revision, refresh]);
  const discard = useCallback(() => { setForm(savedForm); setError(''); }, [savedForm]);
  const save = useCallback(async () => {
    if (!canSave || pending.current) return false;
    pending.current = true; setBusy(true); setError(''); let committed = false;
    try {
      const id = selected ?? await libraryApi.createTopic(form);
      if (selected) await libraryApi.editTopic(selected, form, formRevision);
      committed = true; setSavedForm(form); setSelected(id); setFormRevision(selected ? formRevision + 1 : 1);
      const value = await libraryApi.topic(id); apply(value, true); setRefresh(old => old + 1); toast.success('专题已保存'); return true;
    } catch (reason) {
      setError(`${committed ? '专题已保存，读取失败：' : ''}${errorText(reason)}`); setNeedsRead(committed || String(reason) === 'TOPIC_CONFLICT'); return committed;
    } finally { pending.current = false; setBusy(false); }
  }, [canSave, selected, form, formRevision]);
  useLayoutEffect(() => { navigation.report(active ? { dirty, busy, canSave, save, discard } : null); }, [active, dirty, busy, canSave, save, discard, navigation.report]);
  useLayoutEffect(() => () => navigation.report(null), [navigation.report]);
  useEffect(() => { if (!dirty) return; const guard = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ''; }; window.addEventListener('beforeunload', guard); return () => window.removeEventListener('beforeunload', guard); }, [dirty]);
  function select(id: string | null) { navigation.request(() => { generation.current++; setSelected(id); setDetail(undefined); setForm(empty()); setSavedForm(empty()); setFormRevision(0); setNeedsRead(false); setError(''); }); }
  async function mutate(action: (id: string, revision: number) => Promise<unknown>, message: string) {
    if (!detail || pending.current || dirty || needsRead || loading) return false;
    pending.current = true; setBusy(true); setError(''); let committed = false;
    try {
      await action(detail.topic.id, detail.topic.revision); committed = true;
      apply(await libraryApi.topic(detail.topic.id), true); setRefresh(old => old + 1); toast.success(message); return true;
    } catch (reason) { setError(`${committed ? '操作已保存，读取失败：' : ''}${errorText(reason)}`); setNeedsRead(committed || String(reason) === 'TOPIC_CONFLICT'); return committed; }
    finally { pending.current = false; setBusy(false); }
  }
  function move(index: number, direction: -1 | 1) {
    if (!detail) return;
    const order = detail.members.map(m => m.member.id);
    [order[index], order[index + direction]] = [order[index + direction], order[index]];
    void mutate((id, rev) => libraryApi.reorderTopicMembers(id, order, rev), '顺序已保存');
  }
  const disabled = busy || loading || needsRead || dirty;
  return <section className="library-topics" hidden={!active} aria-label="研究专题">
    <div className="library-section-heading"><Button size="sm" variant="ghost" onClick={() => navigation.request(back)}>返回视频列表</Button><h2>研究专题</h2></div>
    <div className="topics-layout"><aside aria-label="专题列表"><Button size="sm" variant="outline" disabled={busy} onClick={() => select(null)}><Plus />新建专题</Button><LibraryError message={listError} />{listError && <Button size="sm" variant="outline" onClick={() => setRefresh(old => old + 1)}>重新读取专题列表</Button>}<ul>{topics.map(topic => <li key={topic.id}><Button variant="ghost" className="topic-title" aria-pressed={selected === topic.id} disabled={busy || selected === topic.id} onClick={() => select(topic.id)} title={topic.title}>{topic.title}</Button></li>)}</ul>{!topics.length && !listError && <p className="text-xs text-muted-foreground">暂无专题</p>}</aside>
    <div className="topic-detail" aria-busy={loading}><h3>{selected ? '专题详情' : '新建专题'}</h3><LibraryError message={error} />{loading && <p role="status">读取中…</p>}
      <AiSourceView source={detail?.topic.ai_source} />
      <label>标题<Input aria-label="专题标题" maxLength={80} disabled={busy || loading} value={form.title} onChange={e => setForm(old => ({ ...old, title: e.target.value }))} /></label><label>研究内容<Textarea aria-label="专题研究内容" disabled={busy || loading} value={form.description} onChange={e => setForm(old => ({ ...old, description: e.target.value }))} /></label>
      <div className="library-writing-toolbar"><Button size="sm" disabled={!canSave} onClick={() => void save()}>{busy ? '保存中…' : '保存专题'}</Button>{dirty && <span role="status" className="text-xs text-muted-foreground">未保存</span>}{selected && <Button size="sm" variant="outline" disabled={busy} onClick={() => navigation.request(() => { discard(); setRefresh(old => old + 1); })}>重新读取专题</Button>}</div>
      {detail && <><div className="library-section-heading"><h3>专题资料</h3><div className="library-file-actions">{onCreateIdea && <Button size="sm" variant="outline" disabled={busy || loading || needsRead} onClick={() => onCreateIdea({ kind: 'topic', asset_id: null, resource_id: detail.topic.id })}>写选题</Button>}<Button size="sm" variant="outline" disabled={disabled} onClick={event => { pickerTrigger.current = event.currentTarget; setPicker(true); }}><Plus />添加资料</Button></div></div>{!detail.members.length && <p className="text-xs text-muted-foreground">暂无专题资料</p>}
      <ol className="topic-members">{detail.members.map((entry, index) => <li key={entry.member.id} data-topic-member={entry.member.id}><div className="topic-member-info"><span className="text-xs text-muted-foreground">{kindNames[entry.member.kind]}</span><strong>{entry.title || '未命名资料'}</strong><span className="topic-member-source text-xs text-muted-foreground" title={entry.asset_title}>{entry.member.kind !== 'video' ? entry.asset_title || '来源标题未知' : ''}{entry.note_revision != null ? ` r${entry.note_revision}` : ''}</span><div className="topic-member-states">{entry.missing && <Badge variant="outline">引用缺失</Badge>}{entry.archived && <Badge variant="outline">已移出知识库</Badge>}{entry.file_state === 'missing' && <Badge variant="outline">文件缺失</Badge>}{entry.file_state === 'changed' && <Badge variant="outline">文件已变化</Badge>}{entry.file_state === 'unknown' && <Badge variant="outline">文件状态未知</Badge>}</div></div><div className="topic-member-actions">
        <Button size="sm" variant="outline" disabled={busy || entry.missing} onClick={event => { const trigger = event.currentTarget; if (entry.member.kind === 'video') navigation.request(() => openAsset(entry.member.asset_id, trigger)); else setSource(entry); }}>打开资料</Button>
        <Button size="icon-sm" variant="ghost" aria-label={`上移资料 ${index + 1}`} disabled={disabled || index === 0} onClick={() => move(index, -1)}><ArrowUp /></Button><Button size="icon-sm" variant="ghost" aria-label={`下移资料 ${index + 1}`} disabled={disabled || index === detail.members.length - 1} onClick={() => move(index, 1)}><ArrowDown /></Button><Button size="sm" variant="ghost" disabled={disabled} onClick={() => void mutate((id, rev) => libraryApi.removeTopicMember(id, entry.member.id, rev), '已从专题移除')}>移除成员</Button>
      </div></li>)}</ol></>}
    </div></div>
    {picker && <TopicMemberPicker busy={busy} blocked={needsRead} actionError={error} add={input => mutate((id, rev) => libraryApi.addTopicMember(id, input, rev), '资料已加入专题')} close={closePicker} />}
    {source && <TopicSourceDialog key={source.member.id} entry={source} close={() => setSource(undefined)} />}
  </section>;
}
