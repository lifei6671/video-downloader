import { useEffect, useState } from 'react';
import { Pencil, Plus, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryCategory, type LibraryTag, type LibraryAssetDetail } from '@/lib/library-api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Spinner } from '@/components/ui/spinner';

export function categoryRows(categories: LibraryCategory[]): { category: LibraryCategory; depth: number }[] {
  return [...categories].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name)).map(category => ({ category, depth: 0 }));
}

export function LibraryError({ message }: { message: string }) {
  return message ? <Alert variant="destructive" role="alert"><AlertDescription>{message}</AlertDescription></Alert> : null;
}

type CategoryEdit = { id: string | null; name: string; position: number };
export function OrganizationDialog({ kind, categories, tags, close, changed }: {
  kind: 'categories' | 'tags'; categories: LibraryCategory[]; tags: LibraryTag[]; close: () => void; changed: () => void;
}) {
  const [edit, setEdit] = useState<CategoryEdit>({ id: null, name: '', position: 0 });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const categoryMode = kind === 'categories';
  const noun = categoryMode ? '分类' : '标签';
  const rows = categoryMode ? categoryRows(categories).map(row => ({ ...row.category, depth: row.depth })) : tags.map(tag => ({ ...tag, depth: 0 }));
  async function run(action: () => Promise<unknown>, success: string) {
    if (busy) return;
    setBusy(true); setError('');
    try { await action(); changed(); toast.success(success); setEdit({ id: null, name: '', position: 0 }); }
    catch (reason) { setError(errorText(reason)); }
    finally { setBusy(false); }
  }
  return <Dialog open onOpenChange={open => { if (!open && !busy) close(); }}><DialogContent showCloseButton={!busy} className="library-organization-dialog" onEscapeKeyDown={event => { if (busy) event.preventDefault(); }} onInteractOutside={event => { if (busy) event.preventDefault(); }}>
    <DialogHeader><DialogTitle>管理{noun}</DialogTitle><DialogDescription className="sr-only">{categoryMode ? '修改和删除分类' : '新建、修改和删除标签'}</DialogDescription></DialogHeader>
    <LibraryError message={error} />
    {(!categoryMode || edit.id) && <form className="library-organization-form" onSubmit={event => {
      event.preventDefault(); const name = edit.name.trim(); if (!name) return;
      if (Array.from(name).length > 128) { setError(`${noun}名称最多 128 个字符`); return; }
      void run(() => categoryMode ? edit.id ? libraryApi.updateCategory(edit.id, name, null, edit.position) : libraryApi.createCategory(name, null) : edit.id ? libraryApi.renameTag(edit.id, name) : libraryApi.createTag(name), `${noun}已${edit.id ? '更新' : '新建'}`);
    }}>
      <label className="library-field">{noun}名称<Input autoFocus value={edit.name} maxLength={256} disabled={busy} onChange={event => setEdit(old => ({ ...old, name: event.target.value }))} /></label>
      <div className="flex flex-wrap gap-2"><Button size="sm" disabled={busy || !edit.name.trim() || !Number.isSafeInteger(edit.position) || edit.position < 0}>{busy ? <Spinner /> : <Plus />}{edit.id ? '保存修改' : `新建${noun}`}</Button>{edit.id && <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => { setEdit({ id: null, name: '', position: 0 }); setError(''); }}>取消修改</Button>}</div>
    </form>}
    <ul className="library-organization-list" aria-label={`${noun}管理列表`}>{rows.length ? rows.map(row => <li key={row.id}>
      <span className="min-w-0 flex-1 truncate" style={{ paddingInlineStart: row.depth * 12 }} title={row.name}>{row.name}</span>
      <Button size="icon-sm" variant="ghost" aria-label={`修改${noun} ${row.name}`} disabled={busy} onClick={() => { setError(''); const category = categories.find(item => item.id === row.id); setEdit({ id: row.id, name: row.name, position: category?.position ?? 0 }); }}><Pencil /></Button>
      <Button size="icon-sm" variant="ghost" aria-label={`删除${noun} ${row.name}`} disabled={busy} onClick={() => void run(() => categoryMode ? libraryApi.deleteCategory(row.id) : libraryApi.deleteTag(row.id), `${noun}已删除`)}><Trash2 /></Button>
    </li>) : <li className="text-muted-foreground">暂无{noun}</li>}</ul>
    <DialogFooter><Button variant="outline" disabled={busy} onClick={close}>完成</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export function MembershipDialog({ assetIds, categories, tags, close, changed }: {
  assetIds: string[]; categories: LibraryCategory[]; tags: LibraryTag[]; close: () => void; changed: () => void;
}) {
  const [details, setDetails] = useState<LibraryAssetDetail[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [edits, setEdits] = useState<Record<string, boolean>>({});
  useEffect(() => {
    let disposed = false;
    setLoading(true); setError('');
    void Promise.all(assetIds.map(id => libraryApi.asset(id))).then(next => { if (!disposed) setDetails(next); })
      .catch(reason => { if (!disposed) setError(errorText(reason)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [assetIds, retry]);
  async function save() {
    if (busy) return;
    setBusy(true); setError(''); let completed = 0;
    try {
      for (const [key, add] of Object.entries(edits)) {
        const [kind, id] = key.split(':');
        await (kind === 'category' ? libraryApi.setCategories(assetIds, id, add) : libraryApi.setTags(assetIds, id, add));
        completed++;
        setEdits(old => { const next = { ...old }; delete next[key]; return next; });
        setDetails(old => old.map(detail => { const field = kind === 'category' ? 'category_ids' : 'tag_ids'; return { ...detail, [field]: add ? [...new Set([...detail[field], id])] : detail[field].filter(value => value !== id) }; }));
      }
      toast.success('分类与标签已保存'); changed(); close();
    } catch (reason) { if (completed) changed(); setError(`${completed ? `已保存 ${completed} 项，其余修改未保存。` : ''}${errorText(reason)}`); }
    finally { setBusy(false); }
  }
  function choices(kind: 'category' | 'tag') {
    const rows = kind === 'category' ? categoryRows(categories).map(({ category, depth }) => ({ ...category, depth })) : tags.map(tag => ({ ...tag, depth: 0 }));
    return rows.map(row => {
      const key = `${kind}:${row.id}`;
      const count = details.filter(detail => (kind === 'category' ? detail.category_ids : detail.tag_ids).includes(row.id)).length;
      const checked = key in edits ? edits[key] : count === details.length ? true : count ? 'indeterminate' : false;
      return <label key={key} className="library-choice" style={{ paddingInlineStart: 8 + row.depth * 12 }}><Checkbox disabled={busy || loading || details.length !== assetIds.length} checked={checked} onCheckedChange={value => setEdits(old => ({ ...old, [key]: value === true }))} /><span className="min-w-0 break-words">{row.name}</span>{checked === 'indeterminate' && <span className="text-xs text-muted-foreground">部分已选</span>}</label>;
    });
  }
  return <Dialog open onOpenChange={open => { if (!open && !busy) close(); }}><DialogContent showCloseButton={!busy} className="library-membership-dialog" onEscapeKeyDown={event => { if (busy) event.preventDefault(); }} onInteractOutside={event => { if (busy) event.preventDefault(); }}>
    <DialogHeader><DialogTitle>分类与标签</DialogTitle><DialogDescription>已选择 {assetIds.length} 个视频</DialogDescription></DialogHeader>
    <LibraryError message={error} />
    {loading ? <div className="library-loading" role="status"><Spinner />正在读取分类与标签…</div> : details.length !== assetIds.length ? <Button variant="outline" onClick={() => setRetry(value => value + 1)}>重新读取</Button> : <div className="library-membership-columns"><section><h3>分类</h3>{categories.length ? choices('category') : <p className="text-muted-foreground">暂无分类</p>}</section><section><h3>标签</h3>{tags.length ? choices('tag') : <p className="text-muted-foreground">暂无标签</p>}</section></div>}
    <DialogFooter><Button variant="outline" disabled={busy} onClick={close}>取消</Button><Button disabled={busy || loading || !Object.keys(edits).length || details.length !== assetIds.length} onClick={() => void save()}>{busy && <Spinner />}{busy ? '保存中…' : '保存'}</Button></DialogFooter>
  </DialogContent></Dialog>;
}
