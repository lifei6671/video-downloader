import { useEffect, useRef, useState } from 'react';
import { libraryApi, type LibraryAsset, type LibraryAssetDetail, type LibraryClip, type LibraryCursor, type TopicMemberInput, type TopicMemberKind } from '@/lib/library-api';
import { errorText } from '@/api';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';

const defaultKinds: TopicMemberKind[] = ['video', 'clip', 'note'];
export function TopicMemberPicker({ context = '专题', allowedKinds = defaultKinds, busy, blocked, actionError, add, close }: { context?: '专题' | '参考'; allowedKinds?: TopicMemberKind[]; busy: boolean; blocked: boolean; actionError: string; add: (input: TopicMemberInput) => Promise<boolean>; close: () => void }) {
  const [query, setQuery] = useState('');
  const [submitted, setSubmitted] = useState('');
  const [assets, setAssets] = useState<LibraryAsset[]>([]);
  const [cursor, setCursor] = useState<LibraryCursor | null>(null);
  const [assetId, setAssetId] = useState('');
  const [kind, setKind] = useState<TopicMemberKind>(allowedKinds[0]);
  const [resourceId, setResourceId] = useState('');
  const [detail, setDetail] = useState<LibraryAssetDetail>();
  const [clips, setClips] = useState<LibraryClip[]>([]);
  const [loading, setLoading] = useState(true);
  const [reading, setReading] = useState(false);
  const [sourceRefresh, setSourceRefresh] = useState(0);
  const [error, setError] = useState('');
  const generation = useRef(0);
  async function search(next?: LibraryCursor) {
    const token = ++generation.current; setLoading(true); setError('');
    try {
      const page = await libraryApi.search({ query: submitted, archived: false, limit: 20, cursor: next });
      if (token === generation.current) { setAssets(old => next ? [...old, ...page.items.map(r => r.asset)] : page.items.map(r => r.asset)); setCursor(page.next_cursor); }
    } catch (reason) { if (token === generation.current) setError(errorText(reason)); }
    finally { if (token === generation.current) setLoading(false); }
  }
  useEffect(() => { setAssetId(''); setResourceId(''); void search(); return () => { generation.current++; }; }, [submitted]);
  useEffect(() => {
    setDetail(undefined); setClips([]); setResourceId('');
    if (!assetId) return;
    let disposed = false; setReading(true); setError('');
    void Promise.all([libraryApi.asset(assetId), libraryApi.clips(assetId)]).then(([value, rows]) => {
      if (value.asset.id !== assetId || rows.some(c => c.asset_id !== assetId)) throw Error('来源资产不匹配');
      if (!disposed) { setDetail(value); setClips(rows); }
    }).catch(reason => { if (!disposed) setError(errorText(reason)); }).finally(() => { if (!disposed) setReading(false); });
    return () => { disposed = true; };
  }, [assetId, sourceRefresh]);
  return <Dialog open onOpenChange={open => { if (!open && !busy) close(); }}><DialogContent showCloseButton={false}><DialogHeader><DialogTitle>{context === '参考' ? '添加参考资料' : '添加专题资料'}</DialogTitle></DialogHeader>
    <form className="library-writing-toolbar" onSubmit={event => { event.preventDefault(); setSubmitted(query.trim()); }}><Input aria-label={`搜索${context}来源视频`} placeholder="搜索视频" value={query} disabled={busy} onChange={e => setQuery(e.target.value)} /><Button size="sm" variant="outline" disabled={loading || busy}>搜索</Button></form>
    <label>视频<Select value={assetId} disabled={busy || loading} onValueChange={setAssetId}><SelectTrigger aria-label={`${context}来源视频`}><SelectValue placeholder="选择视频" /></SelectTrigger><SelectContent>{assets.map(a => <SelectItem key={a.id} value={a.id}>{a.title || '未命名视频'}</SelectItem>)}</SelectContent></Select></label>
    {cursor && <Button size="sm" variant="outline" disabled={loading || busy} onClick={() => void search(cursor)}>加载更多视频</Button>}
    {!assets.length && !loading && !error && <p>未找到视频</p>}
    <label>资料类型<Select value={kind} disabled={busy} onValueChange={value => { setKind(value as TopicMemberKind); setResourceId(''); }}><SelectTrigger aria-label={`${context}资料类型`}><SelectValue /></SelectTrigger><SelectContent>{allowedKinds.map(kind => <SelectItem key={kind} value={kind}>{{ video: '视频', clip: '片段', note: '笔记' }[kind]}</SelectItem>)}</SelectContent></Select></label>
    {kind !== 'video' && <label>{kind === 'clip' ? '片段' : '笔记'}<Select value={resourceId} disabled={busy || reading || !detail} onValueChange={setResourceId}><SelectTrigger aria-label={`${context}来源资料`}><SelectValue placeholder="选择资料" /></SelectTrigger><SelectContent>{kind === 'clip' ? clips.map(c => <SelectItem key={c.id} value={c.id}>{c.title} {c.start_ms}–{c.end_ms} ms</SelectItem>) : detail?.notes.map((n, i) => <SelectItem key={n.id} value={n.id}>笔记 {i + 1} r{n.revision}{n.origin === 'ai' ? ' AI' : ''}</SelectItem>)}</SelectContent></Select></label>}
    {detail && !reading && kind !== 'video' && !(kind === 'clip' ? clips.length : detail.notes.length) && <p>暂无{kind === 'clip' ? '片段' : '笔记'}</p>}
    <LibraryError message={error || actionError} />{error && <Button variant="outline" size="sm" disabled={loading || busy} onClick={() => { void search(); setSourceRefresh(old => old + 1); }}>重新读取资料</Button>}
    <Button disabled={busy || blocked || loading || reading || !detail || !!error || kind !== 'video' && !resourceId} onClick={() => { void add({ kind, asset_id: assetId, resource_id: kind === 'video' ? assetId : resourceId }).then(ok => { if (ok) close(); }); }}>{busy ? context === '参考' ? '读取中…' : '保存中…' : context === '参考' ? '加入参考' : '添加资料'}</Button>
    <Button size="sm" variant="ghost" disabled={busy} onClick={close}>关闭</Button>
  </DialogContent></Dialog>;
}
