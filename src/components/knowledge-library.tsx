import { useEffect, useRef, useState } from 'react';
import { Archive, FolderTree, Library, Plus, Search, SlidersHorizontal, Tags, X } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryDestination, type LibrarySearchRequest, type IdeaReferenceInput, type LibraryWritingSelection, type LibrarySearchHit } from '@/lib/library-api';
import { useLibrary } from '@/hooks/use-library';
import { LibraryCover } from '@/components/library-cover';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryIdeas } from '@/components/library-ideas';
import { LibraryComparisonWorkspace } from '@/components/library-comparison';
import { LibraryAssetDetailView } from '@/components/library-asset-detail';
import { categoryRows, LibraryError, MembershipDialog, OrganizationDialog } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Badge } from '@/components/ui/badge';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyContent } from '@/components/ui/empty';
import { Input } from '@/components/ui/input';
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
  const [selection, setSelection] = useState<LibraryWritingSelection>();
  const [query, setQuery] = useState('');
  const categoryCreateTrigger = useRef<HTMLButtonElement>(null);
  const [newCategory, setNewCategory] = useState<string | null>(null);
  const [categoryBusy, setCategoryBusy] = useState(false);
  const [categoryError, setCategoryError] = useState('');
  const [detailFile, setDetailFile] = useState<string>();
  const [detailInitialError, setDetailInitialError] = useState<string>();
  useEffect(() => {
    if (!active || !destination || appliedDestination.current === destination.sequence) return;
    appliedDestination.current = destination.sequence;
    setIdeaSession(destination.view === 'ideas' ? { key: ++ideaSequence.current, id: destination.ideaId, reference: destination.ideaReference } : null); setIdeaSource(false);
    setActionError(destination.error ? errorText(destination.error) : '');
    setSelection({ documentId: destination.documentId, noteId: destination.noteId }); setCompareIds(null); setDetailId(destination.assetId ?? null); setDetailFile(destination.fileId); setDetailInitialError(destination.error);
    setRequest({ ...initialRequest, uncategorized: destination.scope === 'uncategorized', unorganized: destination.scope === 'unorganized' });
  }, [active, destination]);
  const [compareIds, setCompareIds] = useState<string[] | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const detailTrigger = useRef<HTMLElement | null>(null);
  const [request, setRequest] = useState(initialRequest);
  const data = useLibrary(request, active);
  const [categoriesOpen, setCategoriesOpen] = useState(false);
  const [manager, setManager] = useState<'categories' | 'tags' | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [membershipIds, setMembershipIds] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState('');
  const listRef = useRef<HTMLDivElement>(null);
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
  function clearFilters() { setQuery(''); setRequest({ ...initialRequest }); }
  const scope = request.uncategorized ? 'uncategorized' : request.unorganized ? 'unorganized' : 'all';
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
  function closeCategoryCreate() { setNewCategory(null); setCategoryError(''); requestAnimationFrame(() => categoryCreateTrigger.current?.focus()); }
  async function addCategory() {
    const name = newCategory?.trim();
    if (!name || categoryBusy) return;
    if (Array.from(name).length > 128) { setCategoryError('分类名称最多 128 个字符'); return; }
    if (data.categories.some(category => category.name === name)) { setCategoryError('分类名称已存在'); return; }
    setCategoryBusy(true); setCategoryError('');
    try { await libraryApi.createCategory(name, null); closeCategoryCreate(); data.refresh(); toast.success('分类已新建'); }
    catch (reason) { setCategoryError(errorText(reason)); }
    finally { setCategoryBusy(false); }
  }
  function openDetail(id: string, trigger: HTMLElement, hit?: LibrarySearchHit | null) { setSelection(hit ? { documentId: hit.document_id ?? undefined, noteId: hit.note_id ?? undefined } : undefined); detailTrigger.current = trigger; setDetailFile(undefined); setDetailInitialError(undefined); setActionError(''); setDetailId(id); }
  function backToList() {
    const assetId = detailId;
    setDetailId(null);
    requestAnimationFrame(() => {
      const trigger = detailTrigger.current?.isConnected ? detailTrigger.current : listRef.current?.querySelector<HTMLElement>(`[data-library-asset="${CSS.escape(assetId ?? '')}"] .library-asset-title`);
      (trigger ?? listRef.current)?.focus({ preventScroll: true });
    });
  }
  function startIdea(reference?: IdeaReferenceInput) { navigation.request(() => { setIdeaSession({ key: ++ideaSequence.current, reference }); setIdeaSource(false); setCompareIds(null); }); }
  function readIdeaAsset(id: string, trigger: HTMLElement | null) { ideaTrigger.current = trigger; setDetailId(id); setDetailFile(undefined); setDetailInitialError(undefined); setIdeaSource(true); }
  function returnToIdea() { setIdeaSource(false); requestAnimationFrame(() => { if (ideaTrigger.current?.isConnected) ideaTrigger.current.focus({ preventScroll: true }); else document.querySelector<HTMLElement>('.library-ideas')?.focus(); }); }
  function closeIdeas() { setIdeaSession(null); setIdeaSource(false); setDetailId(null); requestAnimationFrame(() => listRef.current?.focus()); }
  const allSelected = data.items.length > 0 && data.items.every(item => selected.includes(item.asset.id));
  return <div className="knowledge-library" hidden={!active}>
    {ideaSession && <LibraryIdeas key={ideaSession.key} initialId={ideaSession.id} seed={ideaSession.reference} active={active && !ideaSource} revision={data.revision} navigation={navigation} back={closeIdeas} openAsset={readIdeaAsset} />}
    {compareIds && <LibraryComparisonWorkspace assetIds={compareIds} active={active} navigation={navigation} back={() => { setCompareIds(null); requestAnimationFrame(() => listRef.current?.focus()); }} />}
    {!compareIds && detailId && (!ideaSession || ideaSource) && <LibraryAssetDetailView initialSelection={selection} backLabel={ideaSource ? '返回草稿' : undefined} key={detailId} entryId={appliedDestination.current} initialFileId={detailFile} initialError={detailInitialError} assetId={detailId} active={active} revision={data.revision} categories={data.categories} tags={data.tags} navigation={navigation} back={() => navigation.request(ideaSource ? returnToIdea : backToList)} organize={trigger => openMembership([detailId], trigger)} />}
    <Tabs className="library-list-view" hidden={!!detailId || !!compareIds || !!ideaSession} value={scope} onValueChange={value => filter({ category_ids: [], archived: false, uncategorized: value === 'uncategorized', unorganized: value === 'unorganized' })}>
    <div className="library-toolbar">
      <TabsList variant="line" className="library-scope-tabs" aria-label="素材库范围">
        <TabsTrigger value="all">全部视频</TabsTrigger>
        <TabsTrigger value="uncategorized">待分类</TabsTrigger>
        <TabsTrigger value="unorganized">待整理</TabsTrigger>
      </TabsList>
      <Button className="library-category-toggle" size="sm" variant="outline" aria-expanded={categoriesOpen} aria-controls="library-categories" onClick={() => setCategoriesOpen(open => !open)}><FolderTree />分类</Button>
    </div>
    <TabsContent value={scope} className="library-browser">
      <aside id="library-categories" className="library-categories" data-open={categoriesOpen} aria-label="分类筛选">
        <div className="library-section-heading"><h2>分类</h2><div className="flex items-center"><Button size="icon-sm" variant="ghost" aria-label="新建分类" ref={categoryCreateTrigger} disabled={categoryBusy || newCategory !== null} onClick={() => { setNewCategory(''); setCategoryError(''); }}><Plus /></Button><Button size="icon-sm" variant="ghost" aria-label="管理分类" onClick={event => { organizationTrigger.current = event.currentTarget; setManager('categories'); }}><SlidersHorizontal /></Button></div></div>
        <ul className="library-category-tree" aria-label="视频分类">{categoryRows(data.categories).map(({ category }) => <li key={category.id}>
          <label className="library-choice" title={category.name}><Checkbox checked={request.category_ids?.includes(category.id) ?? false} onCheckedChange={value => filter({ uncategorized: false, category_ids: value ? [...(request.category_ids ?? []), category.id] : request.category_ids?.filter(id => id !== category.id) })} /><span className="truncate">{category.name}</span></label>
        </li>)}</ul>
        {newCategory !== null && <form className="library-category-create" onSubmit={event => { event.preventDefault(); void addCategory(); }}><Input autoFocus aria-label="新分类名称" placeholder="分类名称" maxLength={256} value={newCategory} disabled={categoryBusy} onChange={event => setNewCategory(event.target.value)} onKeyDown={event => { if (event.key === 'Escape' && !categoryBusy) { event.preventDefault(); closeCategoryCreate(); } }} /><div className="flex gap-2"><Button size="sm" disabled={categoryBusy || !newCategory.trim()}>{categoryBusy && <Spinner />}添加</Button><Button size="sm" variant="ghost" type="button" disabled={categoryBusy} onClick={closeCategoryCreate}>取消</Button></div><LibraryError message={categoryError} /></form>}
      </aside>
      <section className="library-results" aria-label="视频资料">
        <form className="library-search" onSubmit={event => { event.preventDefault(); filter({ query: query.trim() }); }}><Input aria-label="搜索素材库" placeholder="搜索标题、作者、文案或笔记" maxLength={512} value={query} onChange={event => setQuery(event.target.value)} /><Button type="submit" size="sm" variant="outline"><Search />搜索</Button>{query && <Button size="icon-sm" variant="ghost" aria-label="清空素材库搜索" onClick={() => { setQuery(''); filter({ query: '' }); }}><X /></Button>}</form>
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
          {selected.length > 0 && <><Button size="sm" variant="outline" disabled={busy || data.loading} onClick={event => openMembership([...selected], event.currentTarget)}>分类与标签</Button><Button size="sm" variant="ghost" disabled={busy || data.loading} onClick={() => void archive([...selected], true)}>移出素材库</Button><Button size="sm" variant="ghost" disabled={busy} onClick={() => setSelected([])}>取消选择</Button></>}
          {busy && <Spinner aria-label="正在保存" />}
        </div>
        <div ref={listRef} className="library-list" tabIndex={0} role="region" aria-label="素材库视频列表" aria-busy={data.loading}>
          {data.loading ? <div role="status" className="library-loading"><Spinner />正在读取视频…</div> : data.error && !data.items.length ? null : data.items.length ? <ul>{data.items.map(({ asset, files, hit, category_ids, tag_ids, has_transcript }) => <li key={asset.id} data-library-asset={asset.id}>
            <Checkbox aria-label={`选择视频 ${asset.title || '未命名视频'}`} checked={selected.includes(asset.id)} disabled={busy} onCheckedChange={value => setSelected(old => value ? [...old, asset.id] : old.filter(id => id !== asset.id))} />
            <LibraryCover asset={asset} />
            <div className="library-asset-info"><h3><button type="button" className="library-asset-title" title={asset.title} aria-label={`查看视频资料 ${asset.title || '未命名视频'}`} onClick={event => openDetail(asset.id, event.currentTarget, hit)}>{asset.title || '未命名视频'}</button></h3><div className="library-asset-meta"><span>{asset.author || '作者未知'}</span><span>{platformNames[asset.platform ?? 'local'] ?? '平台未知'}</span>{!files.available ? <Badge variant="outline">文件缺失</Badge> : files.changed ? <Badge variant="outline">文件已变化</Badge> : <span>{files.total} 个文件</span>}</div><div className="library-asset-meta">{(category_ids ?? []).map(id => <Badge variant="outline" key={id}>{data.categories.find(c => c.id === id)?.name ?? '分类'}</Badge>)}{(tag_ids ?? []).map(id => <Badge variant="secondary" key={id}>{data.tags.find(t => t.id === id)?.name ?? '标签'}</Badge>)}<span>{has_transcript ? '已有文案' : '暂无文案'}</span></div>{hit && <button type="button" className="library-hit" onClick={event => openDetail(asset.id, event.currentTarget, hit)}><span>{hitNames[hit.field]}{hit.document_version != null ? ` v${hit.document_version}` : ''}：</span>{hit.excerpt}</button>}</div>
            <div className="library-row-actions"><Button size="sm" variant="ghost" disabled={busy} onClick={event => openMembership([asset.id], event.currentTarget)}>分类与标签</Button><Button size="icon-sm" variant="ghost" disabled={busy} aria-label={`${'移出素材库'} ${asset.title}`} onClick={() => void archive([asset.id], true)}><Archive /></Button></div>
          </li>)}</ul> : <Empty><EmptyHeader><EmptyMedia variant="icon"><Library /></EmptyMedia><EmptyTitle>{filtered ? '未找到匹配视频' : '暂无视频'}</EmptyTitle></EmptyHeader>{filtered && <EmptyContent><Button variant="outline" onClick={clearFilters}>清除筛选</Button></EmptyContent>}</Empty>}
          {!data.loading && data.next_cursor && <Button className="library-load-more" variant="outline" disabled={data.moreLoading} onClick={() => void data.loadMore()}>{data.moreLoading && <Spinner />}{data.moreLoading ? '读取中…' : '加载更多'}</Button>}
        </div>
      </section>
    </TabsContent>
    </Tabs>
    {manager && <OrganizationDialog kind={manager} categories={data.categories} tags={data.tags} changed={data.refresh} close={() => { setManager(null); requestAnimationFrame(() => organizationTrigger.current?.focus()); }} />}
    {membershipIds && <MembershipDialog assetIds={membershipIds} categories={data.categories} tags={data.tags} changed={data.refresh} close={closeMembership} />}
  </div>;
}
