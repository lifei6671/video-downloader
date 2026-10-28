import { useEffect, useRef, useState } from 'react';
import { Archive, FolderTree, Library, Plus, RotateCcw, Search, SlidersHorizontal, Tags, X } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText } from '@/api';
import { libraryApi, type LibraryDestination, type LibrarySearchRequest, type IdeaReferenceInput } from '@/lib/library-api';
import { useLibrary } from '@/hooks/use-library';
import { LibraryCover } from '@/components/library-cover';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryIdeas } from '@/components/library-ideas';
import { LibraryTopics } from '@/components/library-topics';
import { LibraryComparisonWorkspace } from '@/components/library-comparison';
import { LibraryAssetDetailView } from '@/components/library-asset-detail';
import { categoryRows, LibraryError, MembershipDialog, OrganizationDialog } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Badge } from '@/components/ui/badge';
import { InputGroup, InputGroupAddon, InputGroupInput, InputGroupButton } from '@/components/ui/input-group';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyContent } from '@/components/ui/empty';
import { Spinner } from '@/components/ui/spinner';

const platformNames: Record<string, string> = { douyin: '抖音', tiktok: 'TikTok', youtube: 'YouTube', telegram: 'Telegram', local: '本地导入' };
const hitNames = { title: '标题', author: '作者', recognized: '识别原文', edited: '整理稿', note: '笔记' };
const initialRequest: LibrarySearchRequest = { query: '', category_ids: [], tag_ids: [], platform: null, archived: false, uncategorized: false, unorganized: false, file_state: 'all', limit: 20 };

export function KnowledgeLibrary({ active, navigation, destination }: { active: boolean; navigation: LibraryEditorNavigation; destination?: LibraryDestination }) {
  const appliedDestination = useRef(0);
  const ideaSequence = useRef(0);
  const [ideaSession, setIdeaSession] = useState<{ key: number; id?: string; reference?: IdeaReferenceInput } | null>(null);
  const [ideaSource, setIdeaSource] = useState(false);
  const ideaTrigger = useRef<HTMLElement | null>(null);
  const [detailFile, setDetailFile] = useState<string>();
  const [detailInitialError, setDetailInitialError] = useState<string>();
  useEffect(() => {
    if (!active || !destination || appliedDestination.current === destination.sequence) return;
    appliedDestination.current = destination.sequence;
    setIdeaSession(destination.view === 'ideas' ? { key: ++ideaSequence.current, id: destination.ideaId, reference: destination.ideaReference } : null); setIdeaSource(false);
    setActionError(destination.error ? errorText(destination.error) : '');
    setTopicsOpen(false); setCompareIds(null); setDetailId(destination.assetId ?? null); setDetailFile(destination.fileId); setDetailInitialError(destination.error);
    setQuery(''); setRequest({ ...initialRequest, uncategorized: destination.scope === 'uncategorized', unorganized: destination.scope === 'unorganized' });
  }, [active, destination]);
  const [topicsOpen, setTopicsOpen] = useState(false);
  const [compareIds, setCompareIds] = useState<string[] | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const detailTrigger = useRef<HTMLElement | null>(null);
  const [request, setRequest] = useState(initialRequest);
  const [query, setQuery] = useState('');
  const data = useLibrary(request, active);
  const [categoriesOpen, setCategoriesOpen] = useState(false);
  const [manager, setManager] = useState<'categories' | 'tags' | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [membershipIds, setMembershipIds] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [importing, setImporting] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const [actionError, setActionError] = useState('');
  const listRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const organizationTrigger = useRef<HTMLElement | null>(null);
  const scopeKey = JSON.stringify(request);
  useEffect(() => { setSelected([]); listRef.current?.scrollTo(0, 0); }, [scopeKey]);
  useEffect(() => {
    if (!data.loading && !data.error) setSelected(old => old.filter(id => data.items.some(item => item.asset.id === id)));
  }, [data.items, data.loading, data.error]);
  useEffect(() => {
    if (data.loading || !data.organizationReady) return;
    setRequest(old => {
      const category_ids = old.category_ids?.filter(id => data.categories.some(category => category.id === id));
      const tag_ids = old.tag_ids?.filter(id => data.tags.some(tag => tag.id === id));
      return category_ids?.length === old.category_ids?.length && tag_ids?.length === old.tag_ids?.length ? old : { ...old, category_ids, tag_ids };
    });
  }, [data.categories, data.tags, data.loading, data.organizationReady]);
  const filtered = !!request.query || !!request.category_ids?.length || !!request.tag_ids?.length || !!request.platform || request.uncategorized || request.unorganized || request.file_state !== 'all';
  function filter(patch: Partial<LibrarySearchRequest>) { setRequest(old => ({ ...old, ...patch, cursor: null })); }
  function clearFilters() { setQuery(''); setRequest({ ...initialRequest, archived: request.archived }); }
  async function importVideo() {
    if (importing) return;
    setImporting(true); setActionError('');
    try { const result = await api.importLibraryVideo(); if (result) { toast.success(result.already_present ? '视频已在知识库中' : '视频已导入'); data.refresh(); } else toast.info('已取消导入'); }
    catch (reason) {
      if (String(reason) === 'CANCELED') toast.info('已取消导入');
      else { const message = errorText(reason); setActionError(message); toast.error(message); }
    }
    finally { setImporting(false); setCanceling(false); }
  }
  async function archive(ids: string[], archived: boolean) {
    if (busy) return;
    setBusy(true); setActionError(''); let completed = 0;
    try {
      for (const id of ids) {
        await libraryApi.archive(id, archived); completed++;
        setSelected(old => old.filter(value => value !== id));
      }
      toast.success(archived ? `已移出 ${completed} 个视频` : `已恢复 ${completed} 个视频`);
    } catch (reason) { setActionError(`${completed ? `已完成 ${completed} 个视频，其余未完成。` : ''}${errorText(reason)}`); }
    finally { setBusy(false); data.refresh(); }
  }
  function openMembership(ids: string[], trigger: HTMLElement) { organizationTrigger.current = trigger; setMembershipIds(ids); }
  function closeMembership() { setMembershipIds(null); requestAnimationFrame(() => { if (organizationTrigger.current?.isConnected) organizationTrigger.current.focus(); else listRef.current?.focus(); }); }
  function openDetail(id: string, trigger: HTMLElement) { detailTrigger.current = trigger; setDetailFile(undefined); setDetailInitialError(undefined); setActionError(''); setDetailId(id); }
  function backToList() { setDetailId(null); requestAnimationFrame(() => { if (detailTrigger.current?.isConnected) detailTrigger.current.focus({ preventScroll: true }); else listRef.current?.focus({ preventScroll: true }); }); }
  function startIdea(reference?: IdeaReferenceInput) { navigation.request(() => { setIdeaSession({ key: ++ideaSequence.current, reference }); setIdeaSource(false); setCompareIds(null); }); }
  function readIdeaAsset(id: string, trigger: HTMLElement | null) { ideaTrigger.current = trigger; setDetailId(id); setDetailFile(undefined); setDetailInitialError(undefined); setIdeaSource(true); }
  function returnToIdea() { setIdeaSource(false); requestAnimationFrame(() => { if (ideaTrigger.current?.isConnected) ideaTrigger.current.focus({ preventScroll: true }); else document.querySelector<HTMLElement>('.library-ideas')?.focus(); }); }
  function closeIdeas() { setIdeaSession(null); setIdeaSource(false); setDetailId(null); setTopicsOpen(false); requestAnimationFrame(() => listRef.current?.focus()); }
  const allSelected = data.items.length > 0 && data.items.every(item => selected.includes(item.asset.id));
  return <div className="knowledge-library" hidden={!active}>
    {ideaSession && <LibraryIdeas key={ideaSession.key} initialId={ideaSession.id} seed={ideaSession.reference} active={active && !ideaSource} revision={data.revision} navigation={navigation} back={closeIdeas} openAsset={readIdeaAsset} />}
    {topicsOpen && <LibraryTopics onCreateIdea={startIdea} active={active && !detailId && !ideaSession} revision={data.revision} navigation={navigation} back={() => setTopicsOpen(false)} openAsset={openDetail} />}
    {compareIds && <LibraryComparisonWorkspace assetIds={compareIds} active={active} navigation={navigation} back={() => { setCompareIds(null); requestAnimationFrame(() => listRef.current?.focus()); }} />}
    {!compareIds && detailId && (!ideaSession || ideaSource) && <LibraryAssetDetailView onCreateIdea={startIdea} backLabel={ideaSource ? '返回草稿' : topicsOpen ? '返回专题' : undefined} key={detailId} entryId={appliedDestination.current} initialFileId={detailFile} initialError={detailInitialError} assetId={detailId} active={active} revision={data.revision} categories={data.categories} tags={data.tags} navigation={navigation} back={() => navigation.request(ideaSource ? returnToIdea : backToList)} organize={trigger => openMembership([detailId], trigger)} />}
    <div className="library-list-view" hidden={!!detailId || !!compareIds || topicsOpen || !!ideaSession}>
    <div className="library-toolbar">
      <Button size="sm" variant="outline" onClick={() => setTopicsOpen(true)}>研究专题</Button>
      <Button size="sm" variant="outline" onClick={() => startIdea()}>选题与草稿</Button>
      <Button className="library-category-toggle" size="sm" variant="outline" aria-expanded={categoriesOpen} aria-controls="library-categories" onClick={() => setCategoriesOpen(open => !open)}><FolderTree />分类</Button>
      <form className="library-search" onSubmit={event => { event.preventDefault(); filter({ query: query.trim() }); }}>
        <InputGroup><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput ref={searchRef} aria-label="搜索知识库" placeholder="搜索标题、作者、文案或笔记" maxLength={512} value={query} onChange={event => setQuery(event.target.value)} />{query && <InputGroupAddon align="inline-end"><InputGroupButton aria-label="清空知识库搜索" size="icon-xs" onClick={() => { setQuery(''); filter({ query: '' }); searchRef.current?.focus(); }}><X /></InputGroupButton></InputGroupAddon>}</InputGroup>
        <Button type="submit" size="sm" variant="outline">搜索</Button>
      </form>
      <div className="flex flex-wrap items-center gap-2"><Button variant="ghost" size="sm" disabled={data.loading} onClick={data.refresh} aria-label="刷新知识库"><RotateCcw /></Button><Button size="sm" disabled={importing} onClick={() => void importVideo()}>{importing ? <Spinner /> : <Plus />}{importing ? '导入中…' : '导入视频'}</Button>{importing && <Button size="sm" variant="outline" disabled={canceling} onClick={() => {
        setCanceling(true); void api.cancelLibraryImport().then(() => toast.info('已请求取消导入')).catch(reason => { setActionError(errorText(reason)); setCanceling(false); });
      }}>{canceling ? '取消中…' : '取消导入'}</Button>}</div>
    </div>
    <div className="library-browser">
      <aside id="library-categories" className="library-categories" data-open={categoriesOpen} aria-label="分类筛选">
        <nav aria-label="知识库范围" className="library-scopes">
          <Button variant="ghost" aria-pressed={!request.archived && !request.uncategorized && !request.unorganized && !request.category_ids?.length} onClick={() => filter({ category_ids: [], uncategorized: false, unorganized: false, archived: false })}><Library />全部视频</Button>
          <Button variant="ghost" aria-pressed={!!request.uncategorized} onClick={() => filter({ category_ids: [], uncategorized: true, unorganized: false, archived: false })}><FolderTree />待分类</Button>
          <Button variant="ghost" aria-pressed={!!request.unorganized} onClick={() => filter({ category_ids: [], uncategorized: false, unorganized: true, archived: false })}><Library />待整理</Button>
          <Button variant="ghost" aria-pressed={!!request.archived} onClick={() => filter({ category_ids: [], uncategorized: false, unorganized: false, archived: true })}><Archive />已移出</Button>
        </nav>
        <div className="library-section-heading"><h2>分类</h2><Button size="icon-sm" variant="ghost" aria-label="管理分类" onClick={event => { organizationTrigger.current = event.currentTarget; setManager('categories'); }}><SlidersHorizontal /></Button></div>
        <ul className="library-category-tree" aria-label="视频分类">{categoryRows(data.categories).map(({ category, depth }) => <li key={category.id}>
          <label className="library-choice" style={{ paddingInlineStart: 8 + depth * 12 }} title={category.name}><Checkbox checked={request.category_ids?.includes(category.id) ?? false} onCheckedChange={value => filter({ uncategorized: false, category_ids: value ? [...(request.category_ids ?? []), category.id] : request.category_ids?.filter(id => id !== category.id) })} /><span className="truncate">{category.name}</span></label>
        </li>)}</ul>
        {!data.categories.length && !data.loading && <Button size="sm" variant="ghost" onClick={event => { organizationTrigger.current = event.currentTarget; setManager('categories'); }}><Plus />新建分类</Button>}
      </aside>
      <section className="library-results" aria-label="视频资料">
        <div className="library-filters">
          <Select value={request.platform ?? 'all'} onValueChange={value => filter({ platform: value === 'all' ? null : value as LibrarySearchRequest['platform'] })}><SelectTrigger aria-label="筛选来源平台"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">全部平台</SelectItem>{Object.entries(platformNames).map(([id, name]) => <SelectItem key={id} value={id}>{name}</SelectItem>)}</SelectContent></Select>
          <Select value={request.file_state ?? 'all'} onValueChange={value => filter({ file_state: value as LibrarySearchRequest['file_state'] })}><SelectTrigger aria-label="筛选文件状态"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">全部文件状态</SelectItem><SelectItem value="available">文件可用</SelectItem><SelectItem value="missing">文件缺失</SelectItem><SelectItem value="changed">文件已变化</SelectItem></SelectContent></Select>
          <Button size="sm" variant="outline" onClick={event => { organizationTrigger.current = event.currentTarget; setManager('tags'); }}><Tags />管理标签</Button>
          {filtered && (data.items.length > 0 || data.loading) && <Button size="sm" variant="ghost" onClick={clearFilters}>清除筛选</Button>}
        </div>
        {data.tags.length > 0 && <fieldset className="library-tag-filters"><legend className="sr-only">按标签筛选，多个标签同时满足</legend>{data.tags.map(tag => <label className="library-tag-choice" key={tag.id}><Checkbox checked={request.tag_ids?.includes(tag.id) ?? false} onCheckedChange={value => filter({ tag_ids: value ? [...(request.tag_ids ?? []), tag.id] : request.tag_ids?.filter(id => id !== tag.id) })} /><span className="break-words">{tag.name}</span></label>)}</fieldset>}
        <LibraryError message={actionError || data.error} />
        {data.error && <Button size="sm" variant="outline" className="self-start" onClick={data.refresh}>重新读取</Button>}
        <div className="library-selection-toolbar">
          <label className="library-choice"><Checkbox aria-label="全选已加载视频" checked={allSelected ? true : selected.length ? 'indeterminate' : false} disabled={data.loading || busy || !data.items.length} onCheckedChange={value => setSelected(value ? data.items.map(item => item.asset.id) : [])} /><span>{selected.length ? `已选择 ${selected.length} 个` : `已加载 ${data.items.length} 个`}</span></label>
          {selected.length > 0 && <><Button size="sm" variant="outline" disabled={busy || data.loading} onClick={() => { if (selected.length < 2 || selected.length > 4) setActionError('请选择 2 到 4 个视频进行对比'); else { setActionError(''); setCompareIds([...selected]); } }}>对比所选</Button><Button size="sm" variant="outline" disabled={busy || data.loading} onClick={event => openMembership([...selected], event.currentTarget)}>分类与标签</Button><Button size="sm" variant="ghost" disabled={busy || data.loading} onClick={() => void archive([...selected], !request.archived)}>{request.archived ? '恢复所选' : '移出知识库'}</Button><Button size="sm" variant="ghost" disabled={busy} onClick={() => setSelected([])}>取消选择</Button></>}
          {busy && <Spinner aria-label="正在保存" />}
        </div>
        <div ref={listRef} className="library-list" tabIndex={0} role="region" aria-label="知识库视频列表" aria-busy={data.loading}>
          {data.loading ? <div role="status" className="library-loading"><Spinner />正在读取视频…</div> : data.error && !data.items.length ? null : data.items.length ? <ul>{data.items.map(({ asset, files, hit }) => <li key={asset.id} data-library-asset={asset.id}>
            <Checkbox aria-label={`选择视频 ${asset.title || '未命名视频'}`} checked={selected.includes(asset.id)} disabled={busy} onCheckedChange={value => setSelected(old => value ? [...old, asset.id] : old.filter(id => id !== asset.id))} />
            <LibraryCover asset={asset} />
            <div className="library-asset-info"><h3><button type="button" className="library-asset-title" title={asset.title} aria-label={`查看视频资料 ${asset.title || '未命名视频'}`} onClick={event => openDetail(asset.id, event.currentTarget)}>{asset.title || '未命名视频'}</button></h3><div className="library-asset-meta"><span>{asset.author || '作者未知'}</span><span>{platformNames[asset.platform ?? 'local'] ?? '平台未知'}</span>{!files.available ? <Badge variant="outline">文件缺失</Badge> : files.changed ? <Badge variant="outline">文件已变化</Badge> : <span>{files.total} 个文件</span>}</div>{hit && <p className="library-hit"><span>{hitNames[hit.field]}{hit.document_version != null ? ` v${hit.document_version}` : ''}：</span>{hit.excerpt}</p>}</div>
            <div className="library-row-actions"><Button size="sm" variant="ghost" disabled={busy} onClick={event => openMembership([asset.id], event.currentTarget)}>分类与标签</Button><Button size="icon-sm" variant="ghost" disabled={busy} aria-label={`${request.archived ? '恢复视频' : '移出知识库'} ${asset.title}`} onClick={() => void archive([asset.id], !request.archived)}>{request.archived ? <RotateCcw /> : <Archive />}</Button></div>
          </li>)}</ul> : <Empty><EmptyHeader><EmptyMedia variant="icon"><Library /></EmptyMedia><EmptyTitle>{filtered ? '未找到匹配视频' : request.archived ? '暂无已移出视频' : '暂无视频'}</EmptyTitle></EmptyHeader><EmptyContent>{filtered ? <Button variant="outline" onClick={clearFilters}>清除筛选</Button> : !request.archived && <Button disabled={importing} onClick={() => void importVideo()}><Plus />导入视频</Button>}</EmptyContent></Empty>}
          {!data.loading && data.next_cursor && <Button className="library-load-more" variant="outline" disabled={data.moreLoading} onClick={() => void data.loadMore()}>{data.moreLoading && <Spinner />}{data.moreLoading ? '读取中…' : '加载更多'}</Button>}
        </div>
      </section>
    </div>
    </div>
    {manager && <OrganizationDialog kind={manager} categories={data.categories} tags={data.tags} changed={data.refresh} close={() => { setManager(null); requestAnimationFrame(() => organizationTrigger.current?.focus()); }} />}
    {membershipIds && <MembershipDialog assetIds={membershipIds} categories={data.categories} tags={data.tags} changed={data.refresh} close={closeMembership} />}
  </div>;
}
