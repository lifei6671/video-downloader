import { useEffect, useRef, useState } from 'react';
import { libraryApi, type IdeaReferenceInput, type IdeaReferenceView, type ResearchTopic, type ResearchTopicDetail, type TopicMemberView } from '@/lib/library-api';
import { errorText } from '@/api';
import { LibraryError } from '@/components/library-organization';
import { TopicSourceDialog } from '@/components/library-topic-source';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';

export function IdeaTopicPicker({ busy, error, add, close }: { busy: boolean; error: string; add: (input: IdeaReferenceInput) => Promise<boolean>; close: () => void }) {
  const [topics, setTopics] = useState<ResearchTopic[]>([]);
  const [selected, setSelected] = useState('');
  const [loading, setLoading] = useState(true);
  const [readError, setReadError] = useState('');
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    let disposed = false; setLoading(true); setReadError('');
    void libraryApi.topics().then(value => { if (!disposed) setTopics(value); }).catch(reason => { if (!disposed) setReadError(errorText(reason)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [refresh]);
  return <Dialog open onOpenChange={open => { if (!open && !busy) close(); }}><DialogContent showCloseButton={false} aria-describedby={undefined}><DialogHeader><DialogTitle>引用研究专题</DialogTitle></DialogHeader><LibraryError message={readError || error} />{loading && <p role="status">读取中…</p>}
    <label>研究专题<Select value={selected} disabled={busy || loading} onValueChange={setSelected}><SelectTrigger aria-label="参考研究专题"><SelectValue placeholder="选择专题" /></SelectTrigger><SelectContent>{topics.map(t => <SelectItem key={t.id} value={t.id}>{t.title}</SelectItem>)}</SelectContent></Select></label>
    {!loading && !topics.length && !readError && <p>暂无研究专题</p>}{readError && <Button variant="outline" size="sm" onClick={() => setRefresh(old => old + 1)}>重新读取专题</Button>}
    <Button disabled={!selected || busy || loading || !!readError} onClick={() => { void add({ kind: 'topic', asset_id: null, resource_id: selected }).then(ok => { if (ok) close(); }); }}>{busy ? '读取中…' : '加入参考'}</Button><Button size="sm" variant="ghost" disabled={busy} onClick={close}>关闭</Button>
  </DialogContent></Dialog>;
}
export function IdeaReferenceReader({ source, close, openVideo }: { source: IdeaReferenceView; close: () => void; openVideo: (assetId: string) => void }) {
  if (source.reference.kind === 'clip' && source.reference.asset_id) return <TopicSourceDialog entry={{ title: source.title, asset_title: source.asset_title ?? '来源未知', archived: source.archived, member: { kind: 'clip', asset_id: source.reference.asset_id, resource_id: source.reference.resource_id } }} close={close} />;
  return <TopicReader id={source.reference.resource_id} close={close} openVideo={openVideo} />;
}
function TopicReader({ id, close, openVideo }: { id: string; close: () => void; openVideo: (assetId: string) => void }) {
  const [value, setValue] = useState<ResearchTopicDetail>();
  const [error, setError] = useState('');
  const [entry, setEntry] = useState<TopicMemberView>();
  const [refresh, setRefresh] = useState(0);
  const focus = useRef(document.activeElement as HTMLElement);
  useEffect(() => {
    let disposed = false; setError('');
    void libraryApi.topic(id).then(data => { if (data.topic.id !== id || data.members.some(m => m.member.topic_id !== id)) throw Error('专题来源不匹配'); if (!disposed) setValue(data); }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; };
  }, [id, refresh]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="idea-source-dialog" aria-describedby={undefined} onCloseAutoFocus={event => { event.preventDefault(); if (focus.current?.isConnected) focus.current.focus({ preventScroll: true }); }}><DialogHeader><DialogTitle>{value?.topic.title || '研究专题'}</DialogTitle></DialogHeader><LibraryError message={error} />{error && <Button size="sm" variant="outline" onClick={() => setRefresh(old => old + 1)}>重新读取来源</Button>}{!value && !error && <p role="status">读取中…</p>}
    {value && <><p className="text-xs text-muted-foreground">研究专题 r{value.topic.revision}</p><pre className="idea-source-text">{value.topic.description}</pre><ul className="idea-topic-members">{value.members.map(m => <li key={m.member.id}><span>{m.title || '未命名资料'}</span>{m.missing && <Badge variant="outline">引用缺失</Badge>}{m.archived && <Badge variant="outline">已移出知识库</Badge>}<Button size="sm" variant="outline" disabled={m.missing} onClick={() => { if (m.member.kind === 'video') { close(); openVideo(m.member.asset_id); } else setEntry(m); }}>打开专题资料</Button></li>)}</ul>{!value.members.length && <p>暂无专题资料</p>}</>}
    {entry && <TopicSourceDialog key={entry.member.id} entry={entry} close={() => setEntry(undefined)} />}
  </DialogContent></Dialog>;
}
