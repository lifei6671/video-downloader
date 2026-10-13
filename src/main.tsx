import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Download, ListVideo, CircleCheck, Settings as SettingsIcon, Link, FolderOpen, Film, Eye, EyeOff, Copy, AlertCircle, HardDrive, Search, MoreHorizontal, RotateCcw, Trash2 } from 'lucide-react';
import { cn } from 'cn';
import { api, errorText, mergeTasks, type Task, type Settings, type Profile } from './api';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter, CardAction } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { InputGroup, InputGroupInput, InputGroupTextarea, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogAction, AlertDialogCancel } from '@/components/ui/alert-dialog';
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription, EmptyContent } from '@/components/ui/empty';
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field';
import { Checkbox } from '@/components/ui/checkbox';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Accordion, AccordionItem, AccordionTrigger, AccordionContent } from '@/components/ui/accordion';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Spinner } from '@/components/ui/spinner';
import { Separator } from '@/components/ui/separator';
import { TooltipProvider } from '@/components/ui/tooltip';
import { SidebarProvider, Sidebar, SidebarHeader, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarMenu, SidebarMenuItem, SidebarMenuButton, SidebarMenuBadge, SidebarSeparator, SidebarInset } from '@/components/ui/sidebar';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator } from '@/components/ui/dropdown-menu';
import { Toaster } from '@/components/ui/sonner';
import { toast } from 'sonner';
import { displayPath } from '@/lib/path';
import './style.css';

const brandIcon = new URL('../src-tauri/icons/64x64.png', import.meta.url).href;
const labels = { parsing: '解析中', ready: '已解析，可下载', queued: '等待中', downloading: '下载中', completed: '已完成', failed: '失败', canceled: '已取消' };
const statusVariant = { parsing: 'info', ready: 'info', queued: 'warning', downloading: 'info', completed: 'success', failed: 'destructive', canceled: 'muted' } as const;
const platformName = (platform?: string) => platform === 'douyin' ? '抖音' : platform === 'tiktok' ? 'TikTok' : '识别中';
function bytes(value: number | null) {
  if (value === null) return '—';
  if (value < 1024) return value + ' B';
  if (value < 1048576) return (value / 1024).toFixed(1) + ' KB';
  return (value / 1048576).toFixed(1) + ' MB';
}
function duration(ms?: number) {
  if (ms == null) return '—';
  const seconds = Math.floor(ms / 1000);
  return Math.floor(seconds / 60) + ':' + String(seconds % 60).padStart(2, '0');
}
function VideoCover({ task, small = false }: { task: Task; small?: boolean }) {
  const [failed, setFailed] = useState(false);
  const [local, setLocal] = useState<string>();
  useEffect(() => {
    setFailed(false); setLocal(undefined);
    let disposed = false; let objectUrl: string | undefined;
    if (task.cover_path) api.cover(task.id).then(data => {
      if (disposed) return;
      const bytes = new Uint8Array(data);
      objectUrl = URL.createObjectURL(new Blob([bytes], {type: bytes[0] === 0xff ? 'image/jpeg' : bytes[0] === 0x89 ? 'image/png' : 'image/webp'}));
      setLocal(objectUrl);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [task.id, task.cover_path, task.video?.cover_url]);
  const source = task.cover_path ? local : task.video?.cover_url;
  return <div className={cn('flex shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-muted', small ? 'h-14 w-[42px]' : 'max-w-[40%] self-start')}>
    {source && !failed ? <img className={small ? 'size-full object-contain' : 'h-auto w-auto max-h-[280px] max-w-full object-contain'} src={source} referrerPolicy="no-referrer" alt="作品封面" onError={() => setFailed(true)} /> : <Film className={cn('text-muted-foreground', !small && 'm-8')} aria-hidden="true" />}
  </div>;
}

function DownloadProgress({ task }: { task: Task }) {
  const percent = task.total_bytes ? Math.min(99, Math.floor(task.bytes / task.total_bytes * 100)) : null;
  return <div className="flex flex-col gap-2"><Progress aria-label="下载进度" value={percent} /><div className="flex items-center gap-4 text-xs tabular-nums text-muted-foreground">
    {percent == null ? <span className="flex items-center gap-2"><Spinner />正在下载</span> : <span>{percent}%</span>}<span>{bytes(task.bytes)} / {bytes(task.total_bytes)}</span><span>{task.speed_bps == null ? '速度 —' : bytes(task.speed_bps) + '/s'}</span>
  </div></div>;
}
function ProfileForm({ profile, saved }: { profile: Profile; saved: (settings: Settings) => void }) {
  const [cookie, setCookie] = useState('');
  const [cookieLoaded, setCookieLoaded] = useState(false);
  const [clearCookie, setClearCookie] = useState(false);
  const [proxy, setProxy] = useState('');
  const [proxyChanged, setProxyChanged] = useState(false);
  const [direct, setDirect] = useState(!profile.proxy);
  const [ua, setUa] = useState(profile.user_agent);
  const [show, setShow] = useState(true);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let disposed = false;
    api.cookie(profile.platform).then(value => { if (!disposed) { setCookie(value); setCookieLoaded(true); } })
      .catch(error => { if (!disposed) { setMessage(errorText(error)); setFailed(true); } });
    return () => { disposed = true; };
  }, [profile.platform]);
  async function save(event: React.FormEvent) {
    event.preventDefault(); setBusy(true); setMessage('');
    try {
      if (proxyChanged && !direct && !proxy.trim()) throw 'CONFIG_INVALID';
      saved(await api.save({ platform: profile.platform, user_agent: ua,
        cookie: clearCookie ? '' : cookie, proxy: proxyChanged ? direct ? '' : proxy : null }));
      if (clearCookie) setCookie('');
      setClearCookie(false); setProxy(''); setProxyChanged(false);
      setMessage('配置已保存，仅影响新任务'); setFailed(false);
    } catch (error) { setMessage(errorText(error)); setFailed(true); }
    finally { setBusy(false); }
  }
  return <Card size="sm" className="flex min-w-0 flex-col [--card-spacing:12px]"><CardHeader><CardTitle>{platformName(profile.platform)}</CardTitle><CardDescription>账号与网络独立配置</CardDescription><CardAction><Badge variant="secondary">{profile.cookie_configured ? 'Cookie 已配置' : 'Cookie 未配置'}</Badge></CardAction></CardHeader>
    <CardContent className="flex-1"><form id={'profile-' + profile.platform} onSubmit={save}><FieldGroup className="gap-3">
      <Field data-disabled={clearCookie}><FieldLabel htmlFor={profile.platform + '-cookie'}>网页端 Cookie</FieldLabel><InputGroup><InputGroupTextarea id={profile.platform + '-cookie'} autoComplete="off" rows={4} className={cn('field-sizing-fixed h-24 min-h-24 max-h-48 flex-none resize-y break-all', !show && 'cookie-masked')} value={cookie} disabled={clearCookie || !cookieLoaded} onChange={e => setCookie(e.target.value)} placeholder={cookieLoaded ? '粘贴浏览器中的 Cookie' : '正在读取 Cookie…'} /><InputGroupAddon align="block-end" className="justify-end"><InputGroupButton className="size-7" aria-label={show ? '隐藏 Cookie' : '显示 Cookie'} onClick={() => setShow(!show)}>{show ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup><FieldDescription>Cookie 保存在本机数据库，支持查看和编辑。</FieldDescription></Field>
      <Field orientation="horizontal"><Checkbox id={profile.platform + '-clear'} checked={clearCookie} onCheckedChange={value => setClearCookie(value === true)} /><FieldLabel htmlFor={profile.platform + '-clear'}>清除已保存 Cookie（保存后生效）</FieldLabel></Field>
      <Separator />
      <Field><FieldLabel id={profile.platform + '-mode'}>网络代理</FieldLabel><ToggleGroup size="sm" type="single" variant="outline" aria-labelledby={profile.platform + '-mode'} value={direct ? 'direct' : 'proxy'} onValueChange={value => { if (value) { setDirect(value === 'direct'); setProxyChanged(true); } }}><ToggleGroupItem value="direct">直连</ToggleGroupItem><ToggleGroupItem value="proxy">自定义代理</ToggleGroupItem></ToggleGroup></Field>
      {!direct && <Field><FieldLabel htmlFor={profile.platform + '-proxy'}>HTTP / SOCKS5 地址</FieldLabel><Input id={profile.platform + '-proxy'} className="min-h-8" type="password" autoComplete="off" value={proxy} onChange={e => { setProxy(e.target.value); setProxyChanged(true); }} placeholder={profile.proxy ? '已保存 ' + profile.proxy + ' · 留空保留' : 'socks5://127.0.0.1:7890'} /><FieldDescription>代理密码保存在系统凭据存储中。</FieldDescription></Field>}
      <Accordion type="single" collapsible><AccordionItem value="advanced"><AccordionTrigger className="py-2">高级设置 · User-Agent</AccordionTrigger><AccordionContent><Field><FieldLabel htmlFor={profile.platform + '-ua'}>与 Cookie 来源匹配的浏览器标识</FieldLabel><Textarea id={profile.platform + '-ua'} rows={3} value={ua} onChange={e => setUa(e.target.value)} /></Field></AccordionContent></AccordionItem></Accordion>
      {message && <Alert variant={failed ? 'destructive' : 'default'} role="status"><AlertDescription>{message}</AlertDescription></Alert>}
    </FieldGroup></form></CardContent><CardFooter className="justify-end py-2"><Button type="submit" form={'profile-' + profile.platform} disabled={busy || !cookieLoaded}>{busy && <Spinner data-icon="inline-start" />}{busy ? '保存中…' : '保存配置'}</Button></CardFooter>
  </Card>;
}
function App() {
  useEffect(() => {
    const preventContextMenu = (event: MouseEvent) => event.preventDefault();
    document.addEventListener('contextmenu', preventContextMenu, true);
    return () => document.removeEventListener('contextmenu', preventContextMenu, true);
  }, []);
  const [page, setPage] = useState('download');
  const [tasks, setTasks] = useState<Task[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [input, setInput] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const [confirm, setConfirm] = useState<{task: Task; action: 'cancel' | 'remove' | 'delete'} | null>(null);
  const [duplicate, setDuplicate] = useState<{task: Task; previous: Task} | null>(null);
  const duplicateTaskId = useRef<string | null>(null);
  useEffect(() => { if (duplicate) duplicateTaskId.current = duplicate.task.id; }, [duplicate]);
  const promptedDuplicates = useRef(new Set<string>());
  const [search, setSearch] = useState('');
  const [wide, setWide] = useState(window.innerWidth > 1100);
  const lastStatuses = useRef<Map<string, Task["status"]> | null>(null);
  const submittedInput = useRef('');
  const removedIds = useRef(new Set<string>());
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const cancelTrigger = useRef<HTMLButtonElement | null>(null);
  const preview = tasks.find(t => ['parsing', 'ready'].includes(t.status));
  const runningCount = tasks.filter(t => t.status === 'downloading').length;
  const queued = tasks.filter(t => t.status === 'queued').sort((a, b) => (a.queue_order ?? 0) - (b.queue_order ?? 0));
  const activeCount = runningCount + queued.length + tasks.filter(t => t.download_requested && t.status === 'parsing').length;
  const selectedTask = tasks.find(t => t.id === selected);
  const current = selectedTask?.status === 'completed' ? preview : selectedTask ?? preview;
  useEffect(() => {
    if (!connected) return;
    if (lastStatuses.current) {
      for (const task of tasks) {
        if (task.status === 'completed' && lastStatuses.current.get(task.id) !== 'completed') {
          toast.success('下载完成', { id: task.id, description: task.video?.title || '视频已保存到本地', duration: 5000 });
        }
      }
    }
    lastStatuses.current = new Map(tasks.map(task => [task.id, task.status]));
    if (selectedTask?.status === 'completed') {
      setSelected(null);
      if (input === submittedInput.current) { setInput(''); setError(''); }
    }
  }, [tasks, connected, selectedTask, input]);
  useEffect(() => {
    if (current?.status !== 'ready' || promptedDuplicates.current.has(current.id)) return;
    const previous = tasks.find(t => t.status === 'completed' && t.id !== current.id && t.video?.platform === current.video?.platform && t.video?.video_id === current.video?.video_id);
    if (previous) { promptedDuplicates.current.add(current.id); setDuplicate({task: current, previous}); }
  }, [current, tasks]);
  async function download(task: Task) {
    const previous = tasks.find(t => t.status === 'completed' && t.id !== task.id && t.video?.platform === task.video?.platform && t.video?.video_id === task.video?.video_id);
    if (previous) { setDuplicate({task, previous}); return; }
    try { await api.download(task.id); }
    catch (error) {
      if (String(error) !== 'ALREADY_DOWNLOADED') throw error;
      const snapshot = await api.tasks();
      setTasks(old => mergeTasks(old, snapshot, removedIds.current));
      // A completion may have raced the last UI snapshot. Recheck against the refreshed list.
      const previous = snapshot.find(t => t.status === 'completed' && t.video?.platform === task.video?.platform && t.video?.video_id === task.video?.video_id);
      if (previous) setDuplicate({task, previous}); else throw error;
    }
  }
  useEffect(() => {
    const media = matchMedia('(min-width: 1101px)');
    const update = () => setWide(media.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    (async () => {
      try {
        unsubscribe = await api.subscribe(task => { if (!disposed) setTasks(old => mergeTasks(old, [task], removedIds.current)); }, id => { if (!disposed) { removedIds.current.add(id); setTasks(old => old.filter(task => task.id !== id)); setSelected(old => old === id ? null : old); } });
        if (disposed) { unsubscribe(); return; }
        const [snapshot, config] = await Promise.all([api.tasks(), api.settings()]);
        if (!disposed) { setTasks(old => mergeTasks(old, snapshot, removedIds.current)); setSettings(config); setConnected(true); }
      } catch { if (!disposed) setError('无法连接本地应用，请通过桌面程序启动映取。'); }
    })();
    const refresh = () => { api.tasks().then(snapshot => { if (!disposed) setTasks(old => mergeTasks(old, snapshot, removedIds.current)); }).catch(() => {}); };
    window.addEventListener('focus', refresh);
    return () => { disposed = true; unsubscribe?.(); window.removeEventListener('focus', refresh); };
  }, []);
  async function run(action: () => Promise<unknown>) {
    setError(''); setBusy(true);
    try { await action(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  async function parse(event: React.FormEvent) {
    event.preventDefault();
    if (!input.trim()) { setError('请输入视频分享链接'); inputRef.current?.focus(); return; }
    await run(async () => {
      if (preview?.status === 'ready') await api.cancel(preview.id);
      const task = await api.resolve(input);
      submittedInput.current = input;
      setTasks(old => mergeTasks(old, [task], removedIds.current)); setSelected(task.id);
    });
  }
  const nav = [{id:'download',label:'下载视频',Icon:Download},{id:'tasks',label:'下载任务',Icon:ListVideo},{id:'completed',label:'已完成',Icon:CircleCheck},{id:'settings',label:'设置',Icon:SettingsIcon}];
  const visible = tasks.filter(t => t.download_requested).filter(t => page === 'completed' ? t.status === 'completed' : t.status !== 'completed').filter(t => !search || ((t.video?.title ?? '') + (t.video?.author ?? '')).includes(search)).sort((a, b) => a.status === 'queued' && b.status === 'queued' ? (a.queue_order ?? 0) - (b.queue_order ?? 0) : a.status === 'queued' ? -1 : b.status === 'queued' ? 1 : b.created_at - a.created_at);
  const completed = tasks.filter(t => t.status === 'completed');
  function taskActions(task: Task) {
    return <div className="flex flex-wrap items-center justify-end gap-2">
      {task.status === 'ready' && <Button data-download-task={task.id} disabled={busy} onClick={() => run(() => download(task))}><Download data-icon="inline-start" />下载视频</Button>}
      {['parsing','ready','queued','downloading'].includes(task.status) && <Button size="sm" variant="outline" disabled={busy || task.canceling} onClick={event => { cancelTrigger.current = event.currentTarget; setConfirm({task, action: 'cancel'}); }}>{task.canceling ? '正在取消…' : '取消'}</Button>}
      {task.status === 'completed' && !task.file_missing && <Button size="sm" variant="outline" disabled={busy} onClick={() => run(() => api.reveal(task.id))}><FolderOpen data-icon="inline-start" />打开文件夹</Button>}
      {['failed', 'canceled'].includes(task.status) && <Button size="sm" variant="outline" disabled={busy} onClick={() => task.video ? run(async () => { const next = await api.retry(task.id); setTasks(old => mergeTasks(old, [next], removedIds.current)); setSelected(next.id); }) : (setPage('download'), setSelected(null), setInput(''), requestAnimationFrame(() => inputRef.current?.focus()))}><RotateCcw />{task.video ? '重试' : '重新解析'}</Button>}
      <DropdownMenu><DropdownMenuTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="更多操作" disabled={busy} onFocus={event => { cancelTrigger.current = event.currentTarget; }} onClick={event => { cancelTrigger.current = event.currentTarget; }}><MoreHorizontal /></Button></DropdownMenuTrigger><DropdownMenuContent align="end">
        {task.video && <DropdownMenuItem onSelect={() => void run(() => navigator.clipboard.writeText(task.video!.canonical_url))}><Copy />复制原链接</DropdownMenuItem>}
        {['completed', 'failed', 'canceled'].includes(task.status) && <><DropdownMenuSeparator /><DropdownMenuItem onSelect={() => setConfirm({task, action: 'remove'})}>从列表移除</DropdownMenuItem></>}
        {task.status === 'completed' && !task.file_missing && <DropdownMenuItem variant="destructive" onSelect={() => setConfirm({task, action: 'delete'})}><Trash2 />删除本地视频</DropdownMenuItem>}
      </DropdownMenuContent></DropdownMenu>
    </div>;
  }
  function emptyState(history = false) {
    return <Card size="sm" className="[--card-spacing:12px]"><CardContent><Empty className="min-h-[160px] gap-2 p-4"><EmptyHeader><EmptyMedia variant="icon">{history ? <CircleCheck /> : <Film />}</EmptyMedia><EmptyTitle>{history ? '还没有下载记录' : '粘贴链接开始解析'}</EmptyTitle><EmptyDescription>{history ? '下载完成的视频会保存在这里' : '作品信息将显示在这里'}</EmptyDescription></EmptyHeader><EmptyContent>{page === 'download' ? <div className="flex gap-2"><Badge variant="secondary">抖音</Badge><Badge variant="secondary">TikTok</Badge></div> : <Button size="sm" variant="outline" onClick={() => setPage('download')}>去下载视频</Button>}</EmptyContent></Empty></CardContent></Card>;
  }
  return <TooltipProvider><Toaster position="bottom-right" closeButton /><SidebarProvider className="h-dvh min-h-0 overflow-hidden" open={wide} onOpenChange={setWide} style={{'--sidebar-width':'176px','--sidebar-width-icon':'56px'} as React.CSSProperties}>
    <Sidebar collapsible="icon"><SidebarHeader className="h-12 justify-center px-2"><div data-slot="app-brand" className="flex h-8 items-center gap-2 px-2 group-data-[collapsible=icon]:px-0"><img src={brandIcon} alt="" className="size-8 shrink-0 object-contain" /><span className="flex flex-1 items-center justify-between group-data-[collapsible=icon]:hidden">映取<Badge variant="outline">{__APP_VERSION__}</Badge></span></div></SidebarHeader><SidebarSeparator /><SidebarContent><SidebarGroup className="px-2 py-2"><SidebarGroupContent><SidebarMenu className="gap-1">{nav.map(({id,label,Icon}) => <SidebarMenuItem key={id}><SidebarMenuButton className="h-8" isActive={page === id} aria-current={page === id ? 'page' : undefined} tooltip={label} onClick={() => { setPage(id); setSearch(''); }}><Icon /><span>{label}</span></SidebarMenuButton>{id === 'tasks' && activeCount > 0 && <SidebarMenuBadge>{activeCount}</SidebarMenuBadge>}</SidebarMenuItem>)}</SidebarMenu></SidebarGroupContent></SidebarGroup></SidebarContent><SidebarSeparator /><SidebarFooter className="p-2"><SidebarMenu><SidebarMenuItem><SidebarMenuButton tooltip={connected ? '本地运行' : '连接不可用'}><HardDrive /><span>{connected ? '本地运行' : '连接不可用'}</span></SidebarMenuButton></SidebarMenuItem></SidebarMenu></SidebarFooter></Sidebar>
    <SidebarInset className="min-h-0 min-w-0 overflow-hidden bg-background"><header className="mx-4 mt-4 flex min-h-10 shrink-0 items-center justify-between gap-4 border-b pb-3"><div className="flex flex-wrap items-baseline gap-x-3 gap-y-1"><h1 className="text-xl leading-[1.3] font-semibold tracking-tight">{nav.find(n => n.id === page)?.label}</h1><p className="text-muted-foreground">{page === 'download' ? '粘贴抖音或 TikTok 分享链接，保存视频到本地' : page === 'tasks' ? `${runningCount} 个下载中 · ${queued.length} 个等待 · 并发上限 ${settings?.max_downloads ?? '—'}` : page === 'completed' ? completed.length + ' 条已完成记录 · 视频保存在本机' : '管理下载位置与分平台访问配置'}</p></div>{page === 'completed' && <InputGroup className="max-w-[260px] h-7"><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput aria-label="搜索视频标题或作者" placeholder="搜索视频标题或作者" value={search} onChange={e => setSearch(e.target.value)} /></InputGroup>}</header>
    <div key={page} data-slot="page-content" className="min-h-0 flex-1 overflow-y-auto"><div className="flex w-full flex-col gap-4 p-4">
    {error && <Alert variant="destructive"><AlertCircle /><AlertTitle>操作未完成</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {page === 'download' && <div className="flex flex-col gap-3"><Card size="sm" className="[--card-spacing:12px]"><CardHeader><CardTitle>视频链接</CardTitle></CardHeader><CardContent><form onSubmit={parse}><FieldGroup className="gap-3"><Field orientation="horizontal"><FieldLabel className="sr-only" htmlFor="video-input">视频分享链接</FieldLabel><InputGroup className="min-h-8 flex-1"><InputGroupAddon><Link /></InputGroupAddon><InputGroupTextarea id="video-input" ref={inputRef} rows={1} className="min-h-8 py-1" value={input} onChange={e => setInput(e.target.value)} onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); e.currentTarget.form?.requestSubmit(); } }} placeholder="粘贴视频分享链接或分享口令…" /></InputGroup><Button className="min-w-[100px]" variant={current?.video ? 'secondary' : 'default'} disabled={!connected || busy || preview?.status === 'parsing'}>{preview?.status === 'parsing' ? <Spinner data-icon="inline-start" /> : <Link data-icon="inline-start" />}{preview?.status === 'parsing' ? '正在解析…' : current?.video ? '重新解析' : '解析链接'}</Button></Field><FieldDescription>支持抖音与 TikTok · 自动识别平台<span className="float-right">Ctrl / ⌘ + V 粘贴</span></FieldDescription></FieldGroup></form></CardContent></Card>
    {current ? <Card size="sm" className="[--card-spacing:12px]"><CardHeader><div className="flex items-center gap-2"><Badge variant="outline">{platformName(current.video?.platform)}</Badge><Badge variant={current.canceling ? 'warning' : statusVariant[current.status]}>{current.canceling ? '正在取消…' : labels[current.status]}</Badge></div></CardHeader><CardContent className="flex gap-4"><VideoCover task={current} /><div className="flex min-w-0 flex-1 flex-col gap-3"><CardTitle className="line-clamp-2" title={current.video?.title ?? ''}>{current.video?.title || (current.status === 'parsing' ? '正在获取作品信息' : '等待有效视频链接')}</CardTitle><CardDescription>{current.video?.author ? '@' + current.video.author : '作者 —'}</CardDescription><dl className="grid grid-cols-3 gap-4"><div><dt className="text-xs text-muted-foreground">时长</dt><dd className="mt-2 tabular-nums">{duration(current.video?.duration_ms)}</dd></div><div><dt className="text-xs text-muted-foreground">清晰度</dt><dd className="mt-2">—</dd></div><div><dt className="text-xs text-muted-foreground">文件大小</dt><dd className="mt-2 tabular-nums">{current.status === 'completed' ? bytes(current.bytes) : bytes(current.total_bytes)}</dd></div></dl>{current.status === 'downloading' && <DownloadProgress task={current} />}{current.status === 'queued' && <CardDescription>等待下载 · 第 {queued.findIndex(t => t.id === current.id) + 1} 位，空出名额后自动开始</CardDescription>}{current.error && <Alert variant="destructive"><AlertDescription>{errorText(current.error)}</AlertDescription></Alert>}{current.status === 'canceled' && <CardDescription>已取消，重试将重新解析并从头下载。</CardDescription>}{current.path && <p className="truncate text-xs text-muted-foreground" title={displayPath(current.path)}>{displayPath(current.path)}</p>}</div></CardContent><CardFooter className="justify-end py-2">{taskActions(current)}</CardFooter></Card> : emptyState()}
    <footer className="flex items-center gap-2 text-xs text-muted-foreground"><FolderOpen className="size-4 shrink-0" /><span className="min-w-0 truncate" title={displayPath(settings?.download_directory)}>下载目录：{displayPath(settings?.download_directory) || '—'}</span><Button variant="link" onClick={() => setPage('settings')}>修改设置</Button></footer></div>}
    {(page === 'tasks' || page === 'completed') && <div className="flex flex-col gap-2">{visible.length ? visible.map(task => <Card size="sm" key={task.id} className="[--card-spacing:12px]"><CardContent className="flex items-center gap-3"><VideoCover task={task} small /><div className="flex min-w-0 flex-1 flex-col gap-1.5"><div className="flex min-w-0 items-center gap-2"><CardTitle className="truncate" title={task.video?.title ?? ''}>{task.video?.title || '单条视频解析'}</CardTitle><Badge className="shrink-0" variant={task.canceling ? 'warning' : statusVariant[task.status]}>{task.canceling ? '正在取消…' : labels[task.status]}</Badge></div><div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground"><span>{platformName(task.video?.platform)}</span>{task.video?.author && <span>@{task.video.author}</span>}<span>{new Date((task.completed_at ?? task.created_at) * 1000).toLocaleString('zh-CN')}</span>{task.status === 'completed' && <span>{bytes(task.bytes)}</span>}</div>{task.status === 'downloading' && <DownloadProgress task={task} />}{task.status === 'queued' && <p className="text-xs text-muted-foreground">队列第 {queued.findIndex(t => t.id === task.id) + 1} 位 · 等待下载名额</p>}{task.error && <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive"><AlertCircle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" /><span>{errorText(task.error)}</span></p>}{task.cover_error && <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><AlertCircle className="size-3.5" />封面未保存</p>}{task.file_missing && <Alert className="py-2"><AlertCircle /><AlertDescription>文件已移动或删除</AlertDescription></Alert>}{task.path && <p className="truncate text-xs text-muted-foreground" title={displayPath(task.path)}>{displayPath(task.path)}</p>}</div><div className="max-w-[240px] shrink-0">{taskActions(task)}</div></CardContent></Card>) : emptyState(page === 'completed')}</div>}
    {page === 'settings' && settings && <div className="flex flex-col gap-4"><Card size="sm" className="[--card-spacing:12px]"><CardContent><FieldGroup className="gap-3"><Field orientation="horizontal" className="grid grid-cols-[64px_minmax(0,1fr)]"><FieldLabel className="w-16 shrink-0" htmlFor="download-directory">下载目录</FieldLabel><div className="flex min-w-0 flex-1 items-center gap-2"><Input id="download-directory" className="min-h-8" readOnly value={displayPath(settings.download_directory)} /><Button variant="outline" disabled={busy} onClick={() => run(async () => setSettings(await api.directory()))}><FolderOpen data-icon="inline-start" />选择文件夹</Button></div></Field><Field orientation="horizontal" className="grid grid-cols-[96px_minmax(0,1fr)]"><FieldLabel id="concurrency-label">同时下载数量</FieldLabel><div className="flex flex-wrap items-center gap-3"><Select value={String(settings.max_downloads)} disabled={busy} onValueChange={value => void run(async () => setSettings(await api.concurrency(Number(value))))}><SelectTrigger size="sm" aria-labelledby="concurrency-label" className="w-24"><SelectValue /></SelectTrigger><SelectContent>{[1, 2, 3].map(value => <SelectItem key={value} value={String(value)}>{value} 个</SelectItem>)}</SelectContent></Select><FieldDescription>默认 2 个；降低上限不打断已开始的下载。</FieldDescription></div></Field></FieldGroup></CardContent></Card><div className="flex items-center justify-between"><h2 className="text-sm font-semibold">平台账号与网络</h2><Badge variant="outline">仅在本机保存</Badge></div><div className="grid grid-cols-2 items-stretch gap-4 max-[1100px]:grid-cols-1">{settings.profiles.map(profile => <ProfileForm key={profile.platform} profile={profile} saved={setSettings} />)}</div><Alert><AlertCircle /><AlertDescription>公开作品解析成功只证明请求成功，不代表登录身份有效。</AlertDescription></Alert><Separator /><div className="flex flex-col gap-1 pb-2"><h2 className="font-semibold">关于映取</h2><p className="text-sm text-muted-foreground">{__APP_VERSION__} · yt-dlp 2026.08.19 · 引擎 0.2.1</p><p className="text-xs text-muted-foreground">开源许可与依赖来源见项目 vendor/UPSTREAM.md。</p></div></div>}
    </div></div></SidebarInset>
    <AlertDialog open={!!duplicate} onOpenChange={open => { if (!open) setDuplicate(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const button = document.querySelector<HTMLButtonElement>('[data-download-task="' + duplicateTaskId.current + '"]'); ((button && !button.disabled ? button : null) ?? inputRef.current ?? document.querySelector<HTMLButtonElement>('[aria-current="page"]'))?.focus(); }); }}><AlertDialogHeader><AlertDialogTitle>该视频已下载过</AlertDialogTitle><AlertDialogDescription>已找到这个作品的成功下载记录。重新下载会保存为新文件，不覆盖原视频。{duplicate?.previous.file_missing && <span className="mt-2 block">原视频文件已移动或删除。</span>}<span className="mt-2 block break-all">{displayPath(duplicate?.previous.path)}</span></AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">暂不下载</AlertDialogCancel><AlertDialogAction size="sm" onClick={() => { const task = duplicate?.task; setDuplicate(null); if (task) void run(() => api.download(task.id, true)); }}>重新下载</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <AlertDialog open={!!confirm} onOpenChange={open => { if (!open) setConfirm(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); cancelTrigger.current?.focus(); }}><AlertDialogHeader><AlertDialogTitle>{confirm?.action === 'delete' ? '删除本地视频？' : confirm?.action === 'remove' ? '从列表移除？' : '取消当前任务？'}</AlertDialogTitle><AlertDialogDescription>{confirm?.action === 'delete' ? <>将从磁盘永久删除此视频，保留历史记录。<span className="mt-2 block break-all">{displayPath(confirm.task.path)}</span></> : confirm?.action === 'remove' ? '仅移除这条记录，不会删除本地视频。' : '将取消等待，或停止当前解析 / 下载并清理暂存文件。其他任务不受影响，记录会保留。'}</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">返回</AlertDialogCancel><AlertDialogAction size="sm" variant={confirm?.action === 'delete' ? 'destructive' : 'default'} onClick={() => { const pending = confirm; setConfirm(null); if (pending) void run(() => pending.action === 'delete' ? api.deleteFile(pending.task.id) : pending.action === 'remove' ? api.remove(pending.task.id).then(() => requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('[aria-current="page"]')?.focus())) : api.cancel(pending.task.id)); }}>{confirm?.action === 'delete' ? '删除视频' : confirm?.action === 'remove' ? '移除记录' : '确认取消'}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </SidebarProvider></TooltipProvider>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
