import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Download, Play, ListVideo, ListX, CircleCheck, Settings as SettingsIcon, Link, FolderOpen, Film, Eye, EyeOff, Copy, ClipboardPaste, AlertCircle, HardDrive, Search, RotateCcw, Trash2, Info, KeyRound, PackageCheck, FileText } from 'lucide-react';
import { cn } from 'cn';
import { api, errorText, mergeTasks, type Task, type Settings, type Profile, type Resolver } from './api';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter, CardAction } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { InputGroup, InputGroupInput, InputGroupTextarea, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogAction, AlertDialogCancel } from '@/components/ui/alert-dialog';
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription, EmptyContent } from '@/components/ui/empty';
import { Field, FieldGroup, FieldLabel, FieldDescription, FieldError, FieldSet } from '@/components/ui/field';
import { Checkbox } from '@/components/ui/checkbox';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Spinner } from '@/components/ui/spinner';
import { Separator } from '@/components/ui/separator';
import { Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { SidebarProvider, Sidebar, SidebarHeader, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarMenu, SidebarMenuItem, SidebarMenuButton, SidebarMenuBadge, SidebarSeparator, SidebarInset } from '@/components/ui/sidebar';
import { Toaster } from '@/components/ui/sonner';
import { toast } from 'sonner';
import { displayPath } from '@/lib/path';
import { About, aboutVersionInfo } from '@/components/about';
import { RuntimeDependencies, RuntimeDependenciesToolbar, useRuntimeDependencies } from '@/components/runtime-dependencies';
import { TdlAccountCard, TdlDependencyCard, useTdlDependency } from '@/components/tdl-settings';
import { ShareLinkInput } from '@/components/share-link-input';
import { ContextMenu, ContextMenuTrigger, ContextMenuContent, ContextMenuGroup, ContextMenuItem } from '@/components/ui/context-menu';
import { TaskRemovalRow } from '@/components/task-removal-row';
import { VideoTranscriptDialog, useVideoTranscripts } from '@/components/video-transcript';
import { previousDownload, duplicatePromptKey, sameDownloadVariant } from '@/lib/download-identity';
import './style.css';

const brandIcon = new URL('../src-tauri/icons/64x64.png', import.meta.url).href;
const labels = { parsing: '解析中', ready: '已解析，可下载', queued: '等待中', downloading: '下载中', completed: '已完成', failed: '失败', canceled: '已取消' };
const statusVariant = { parsing: 'info', ready: 'info', queued: 'warning', downloading: 'info', completed: 'success', failed: 'destructive', canceled: 'muted' } as const;
const platformName = (platform?: string) => platform === 'douyin' ? '抖音' : platform === 'tiktok' ? 'TikTok' : platform === 'youtube' ? 'YouTube' : platform === 'telegram' ? 'Telegram' : '平台未知';
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
function GalleryImage({ task, index, className, onReadyChange }: { task: Task; index: number; className?: string; onReadyChange?: (source: string | undefined) => void }) {
  const [source, setSource] = useState<string>();
  const [failed, setFailed] = useState(false);
  const filename = task.files[index]?.filename ?? task.video?.images?.[index]?.preview_filename;
  const completed = task.status === 'completed';
  useEffect(() => {
    let disposed = false; let url: string | undefined;
    setSource(undefined); setFailed(false); onReadyChange?.(undefined);
    api.image(task.id, index).then(data => {
      if (disposed) return;
      const extension = filename?.split('.').pop() ?? '';
      const mime = ({jpeg: 'image/jpeg', png: 'image/png', webp: 'image/webp', gif: 'image/gif', avif: 'image/avif', heic: 'image/heic'} as Record<string, string>)[extension];
      url = URL.createObjectURL(new Blob([new Uint8Array(data)], {type: mime}));
      setSource(url);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; if (url) URL.revokeObjectURL(url); };
  }, [task.id, index, filename, completed, onReadyChange]);
  return source && !failed ? <img src={source} className={cn('size-full object-contain', className)} alt={`第 ${index + 1} 张图片`} onLoad={() => onReadyChange?.(source)} onError={() => { setFailed(true); onReadyChange?.(undefined); }} /> : failed ? <Film className="m-auto size-6 text-muted-foreground" aria-label="图片预览不可用" /> : <Spinner className="m-auto" aria-label="正在加载图片" />;
}
function AlbumGallery({ task, expanded = false }: { task: Task; expanded?: boolean }) {
  const [index, setIndex] = useState(0);
  const images = task.video?.images ?? [];
  const selected = images[index];
  return <div className={cn("flex min-w-0 shrink-0 flex-col gap-2", expanded ? "w-full" : "w-[40%]")}>
    <div className={cn(expanded ? "h-[380px]" : "h-[240px]", "flex items-center justify-center overflow-hidden rounded-lg border bg-muted")}><GalleryImage task={task} index={index} /></div>
    <div className="flex gap-1 overflow-x-auto pb-1" aria-label="图集图片列表">{images.map((_, i) => <Button key={i} size="icon-sm" variant="outline" className={cn('h-12 w-10 shrink-0 overflow-hidden p-0', i === index && 'ring-2 ring-ring')} aria-label={`查看第 ${i + 1} 张图片`} aria-pressed={i === index} onClick={() => setIndex(i)}><GalleryImage task={task} index={i} /></Button>)}</div>
    <p className="text-xs text-muted-foreground">{index + 1} / {images.length} · {selected?.quality_source === 'original' ? '原图' : '高清'}{selected?.width && selected.height ? ` · ${selected.width} × ${selected.height}` : ''}</p>
  </div>;
}
function VideoCover({ task, small = false }: { task: Task; small?: boolean }) {
  const [failed, setFailed] = useState(false);
  const [local, setLocal] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [copying, setCopying] = useState(false);
  const [albumSource, setAlbumSource] = useState<string>();
  const [previewOpen, setPreviewOpen] = useState(false);
  const coverRef = useRef<HTMLButtonElement>(null);
  const previewImageRef = useRef<HTMLImageElement>(null);
  useEffect(() => {
    setFailed(false); setLocal(undefined);
    let disposed = false; let objectUrl: string | undefined;
    if (task.cover_path || task.cover_cached) api.cover(task.id).then(data => {
      if (disposed) return;
      const bytes = new Uint8Array(data);
      objectUrl = URL.createObjectURL(new Blob([bytes], {type: bytes[0] === 0xff ? 'image/jpeg' : bytes[0] === 0x89 ? 'image/png' : 'image/webp'}));
      setLocal(objectUrl);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [task.id, task.cover_path, task.cover_cached, task.video?.cover_url]);
  const album = task.video?.kind === 'image_album';
  const source = album ? albumSource : task.cover_path || task.cover_cached ? local : task.video?.cover_url;
  const available = !!source && (album || !failed);
  useEffect(() => { if (!available) setPreviewOpen(false); }, [available]);
  async function saveCover() {
    if (saving || copying) return;
    setSaving(true);
    try { if (await api.saveCover(task.id)) toast.success('封面已保存'); }
    catch (error) { toast.error(errorText(error)); }
    finally { setSaving(false); }
  }
  async function copyCover(image = coverRef.current?.querySelector('img')) {
    if (copying || saving) return;
    setCopying(true);
    try {
      if (!image?.naturalWidth || !image.naturalHeight) throw Error('Cover unavailable');
      const canvas = document.createElement('canvas');
      canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
      const context = canvas.getContext('2d');
      if (!context) throw Error('Canvas unavailable');
      context.drawImage(image, 0, 0);
      await api.copyCover(context.getImageData(0, 0, canvas.width, canvas.height));
      toast.success('封面已复制到剪贴板');
    } catch { toast.error('无法复制封面，请稍后重试'); }
    finally { setCopying(false); }
  }
  const cover = <DialogTrigger asChild><Button ref={coverRef} type="button" variant="ghost"
    data-cover-context-menu={small ? '' : undefined} aria-label="查看封面" aria-disabled={!available}
    onClick={event => { if (!available) event.preventDefault(); }}
    className={cn('flex shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-muted p-0', small ? 'h-14 w-[42px]' : album ? 'h-[240px] w-[40%]' : 'h-auto max-w-[40%] self-start')}>
    {album ? <GalleryImage task={task} index={0} onReadyChange={setAlbumSource} /> : source && !failed ? <img className={small ? 'size-full object-contain' : 'h-auto w-auto max-h-[280px] max-w-full object-contain'} src={source} referrerPolicy="no-referrer" alt="作品封面" onError={() => setFailed(true)} /> : <Film className={cn('text-muted-foreground', !small && 'm-8')} aria-hidden="true" />}
  </Button></DialogTrigger>;
  return <Dialog open={previewOpen} onOpenChange={setPreviewOpen}>{small ? <ContextMenu><ContextMenuTrigger asChild>{cover}</ContextMenuTrigger>
    <ContextMenuContent aria-label="封面操作"><ContextMenuGroup>
      <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void saveCover()}><Download />{saving ? '保存中…' : '保存封面'}</ContextMenuItem>
      <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void copyCover()}><Copy />{copying ? '复制中…' : '复制封面'}</ContextMenuItem>
    </ContextMenuGroup></ContextMenuContent>
  </ContextMenu> : cover}
    <DialogContent className="max-h-[90vh] w-fit min-w-56 max-w-[90vw] overflow-y-auto sm:max-w-[90vw]">
      <DialogHeader className="w-0 min-w-full pr-8"><DialogTitle>封面预览</DialogTitle><DialogDescription className="line-clamp-2 text-sm">{task.video?.title || '作品封面'}</DialogDescription></DialogHeader>
      {source && <ContextMenu><ContextMenuTrigger asChild>
        <img ref={previewImageRef} src={source} alt="封面大图" data-cover-context-menu="" tabIndex={0}
          className="mx-auto h-auto max-h-[calc(90vh-7.5rem)] w-auto max-w-[calc(90vw-2rem)] object-contain outline-none focus-visible:ring-2 focus-visible:ring-ring" />
      </ContextMenuTrigger><ContextMenuContent aria-label="封面预览操作"><ContextMenuGroup>
        <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void saveCover()}><Download />{saving ? '保存中…' : '另存为…'}</ContextMenuItem>
        <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void copyCover(previewImageRef.current)}><Copy />{copying ? '复制中…' : '复制封面'}</ContextMenuItem>
      </ContextMenuGroup></ContextMenuContent></ContextMenu>}
    </DialogContent>
  </Dialog>;
}

function DownloadProgress({ task }: { task: Task }) {
  const total = task.total_bytes ?? task.total_bytes_estimate;
  const percent = total ? Math.max(0, Math.min(99, Math.floor(task.bytes / total * 100))) : null;
  return <div className="flex flex-col gap-2"><Progress aria-label="下载进度" value={percent} /><div className="flex flex-wrap items-center gap-4 text-xs tabular-nums text-muted-foreground">
    {task.merging || percent == null ? <span className="flex items-center gap-2"><Spinner />{task.merging ? '正在合并音视频…' : '正在下载'}</span> : <span>{percent}%</span>}{task.total_items != null && <span>{task.completed_items ?? 0} / {task.total_items} 张</span>}<span>{bytes(task.bytes)} / {task.total_bytes == null && task.total_bytes_estimate != null ? '约 ' : ''}{bytes(total ?? null)}</span><span>{task.speed_bps == null ? '速度 —' : bytes(task.speed_bps) + '/s'}</span>{task.eta_seconds != null && <span>预计剩余 {Math.ceil(task.eta_seconds)} 秒</span>}
  </div></div>;
}
type SettingsSection = 'download' | 'accounts' | 'dependencies';
const settingsSections: { id: SettingsSection; label: string; description: string; Icon: typeof Download }[] = [
  { id: 'download', label: '下载设置', description: '选择文件保存位置与同时下载数量。', Icon: Download },
  { id: 'accounts', label: '平台账号', description: '按平台管理账号认证、Cookie、网络代理与 User-Agent。', Icon: KeyRound },
  { id: 'dependencies', label: '运行依赖', description: '检查本机运行环境，补齐解析与音视频合并需要的工具。', Icon: PackageCheck },
];
function ProfileForm({ profile, saved }: { profile: Profile; saved: (settings: Settings) => void }) {
  const [cookie, setCookie] = useState('');
  const [savedCookie, setSavedCookie] = useState('');
  const [cookieLoaded, setCookieLoaded] = useState(false);
  const [useBrowserCookies, setUseBrowserCookies] = useState(!profile.cookie_configured);
  const [proxy, setProxy] = useState('');
  const [savedProxy, setSavedProxy] = useState('');
  const [proxyLoaded, setProxyLoaded] = useState(false);
  const [proxyError, setProxyError] = useState('');
  const [direct, setDirect] = useState(!profile.proxy);
  const [ua, setUa] = useState(profile.user_agent);
  const [chunkMode, setChunkMode] = useState(profile.http_chunk_size == null ? 'inherit' : profile.http_chunk_size === 0 ? 'disabled' : 'custom');
  const [chunkSize, setChunkSize] = useState(String(profile.http_chunk_size ? profile.http_chunk_size / 1048576 : 10));
  const [show, setShow] = useState(true);
  const [busy, setBusy] = useState(false);
  const [cookieError, setCookieError] = useState('');
  const [message, setMessage] = useState<{ text: string; failed: boolean } | null>(null);
  const proxyChanged = proxyLoaded && (direct !== (savedProxy === '') || (!direct && proxy !== savedProxy));
  const chunkInvalid = profile.platform === 'youtube' && chunkMode === 'custom' &&
    (!Number.isInteger(Number(chunkSize)) || Number(chunkSize) < 1 || Number(chunkSize) > 1024);
  const chunkBytes = chunkMode === 'inherit' ? null : chunkMode === 'disabled' ? 0 : Number(chunkSize) * 1048576;
  const chunkChanged = profile.platform === 'youtube' && chunkBytes !== (profile.http_chunk_size ?? null);
  const changed = (cookieLoaded && (useBrowserCookies ? '' : cookie) !== savedCookie) || proxyChanged || ua !== profile.user_agent || chunkChanged;
  useEffect(() => {
    let disposed = false;
    api.cookie(profile.platform).then(value => { if (!disposed) { setCookie(value); setSavedCookie(value); setUseBrowserCookies(value === ''); setCookieLoaded(true); } })
      .catch(error => { if (!disposed) setCookieError(errorText(error)); });
    api.proxy(profile.platform).then(value => { if (!disposed) { setProxy(value ?? ''); setSavedProxy(value ?? ''); setProxyLoaded(true); } })
      .catch(error => { if (!disposed) setProxyError(errorText(error)); });
    return () => { disposed = true; };
  }, [profile.platform]);
  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (busy || !changed || chunkInvalid) return;
    setBusy(true); setMessage(null);
    try {
      if (proxyChanged && !direct && !proxy.trim()) throw 'CONFIG_INVALID';
      const nextCookie = useBrowserCookies ? '' : cookie.trim();
      saved(await api.save({ platform: profile.platform,
        ...(profile.platform === 'youtube' ? { http_chunk_size: chunkBytes } : {}),
        user_agent: ua,
        cookie: cookieLoaded ? nextCookie : null,
        proxy: proxyChanged ? direct ? '' : proxy.trim() : null }));
      if (cookieLoaded) {
        setSavedCookie(nextCookie);
        setCookie(nextCookie);
        setUseBrowserCookies(nextCookie === '');
      }
      if (proxyChanged) {
        const nextProxy = direct ? '' : proxy.trim();
        setProxy(nextProxy); setSavedProxy(nextProxy);
      }
      setMessage({text: '平台配置已保存，仅影响新任务', failed: false});
    } catch (error) { setMessage({text: errorText(error), failed: true}); }
    finally { setBusy(false); }
  }
  return <Card size="sm" className="flex min-w-0 flex-col [--card-spacing:12px]"><CardHeader><CardTitle className="font-semibold">{platformName(profile.platform)}</CardTitle><CardDescription>此平台的凭据、代理和浏览器标识独立保存</CardDescription><CardAction><Badge variant="secondary">{profile.cookie_configured ? '手动 Cookie 已配置' : '使用默认浏览器'}</Badge></CardAction></CardHeader>
    <CardContent className="flex-1"><form id={'profile-' + profile.platform} onSubmit={save} aria-busy={busy}><FieldSet disabled={busy} className="min-w-0"><FieldGroup className="gap-3">
      <Field data-disabled={useBrowserCookies}><FieldLabel htmlFor={profile.platform + '-cookie'}>网页端 Cookie</FieldLabel><InputGroup><InputGroupTextarea id={profile.platform + '-cookie'} autoComplete="off" rows={4} className={cn('field-sizing-fixed h-24 min-h-24 max-h-48 flex-none resize-y break-all', !show && 'cookie-masked')} value={cookie} disabled={useBrowserCookies || !cookieLoaded} onChange={e => setCookie(e.target.value)} placeholder={cookieLoaded ? '粘贴浏览器中的 Cookie' : cookieError ? 'Cookie 读取失败' : '正在读取 Cookie…'} /><InputGroupAddon align="block-end" className="justify-end"><InputGroupButton className="size-7" aria-label={show ? '隐藏 Cookie' : '显示 Cookie'} onClick={() => setShow(!show)}>{show ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup><FieldDescription>手动 Cookie 保存在本机数据库；留空时尝试读取系统默认浏览器，不回写自动读取的 Cookie。</FieldDescription></Field>
      <Field orientation="horizontal"><Checkbox id={profile.platform + '-clear'} disabled={!cookieLoaded} checked={useBrowserCookies} onCheckedChange={value => setUseBrowserCookies(value === true)} /><FieldLabel htmlFor={profile.platform + '-clear'}>使用系统默认浏览器 Cookie（保存后生效，将清除手动 Cookie）</FieldLabel></Field>
      {cookieError && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{cookieError}；保存其他设置将保留已有 Cookie。</AlertDescription></Alert>}
      <Separator />
      <Field><FieldLabel id={profile.platform + '-mode'}>网络代理</FieldLabel><ToggleGroup size="sm" type="single" variant="outline" aria-labelledby={profile.platform + '-mode'} value={direct ? 'direct' : 'proxy'} disabled={!proxyLoaded} onValueChange={value => { if (value) setDirect(value === 'direct'); }}><ToggleGroupItem value="direct">直连</ToggleGroupItem><ToggleGroupItem value="proxy">自定义代理</ToggleGroupItem></ToggleGroup></Field>
      {!direct && <Field><FieldLabel htmlFor={profile.platform + '-proxy'}>HTTP / SOCKS5 地址</FieldLabel><Input id={profile.platform + '-proxy'} className="min-h-8" type="text" autoComplete="off" spellCheck={false} value={proxy} disabled={!proxyLoaded} onChange={e => setProxy(e.target.value)} placeholder={proxyLoaded ? 'socks5://127.0.0.1:7890' : proxyError ? '代理读取失败' : '正在读取代理…'} /><FieldDescription>请输入完整地址，例如 http://127.0.0.1:7890 或 socks5://127.0.0.1:7890；含密码的代理保存在系统凭据存储中。系统 TUN 已接管网络时可选择直连。</FieldDescription></Field>}
      {proxyError && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{proxyError}；保存其他设置将保留已有代理。</AlertDescription></Alert>}
      <Separator />
      {profile.platform === 'youtube' && <>
        <Field><FieldLabel id="youtube-chunk-mode">HTTP 分块下载</FieldLabel><ToggleGroup size="sm" type="single" variant="outline" aria-labelledby="youtube-chunk-mode" value={chunkMode} disabled={busy} onValueChange={value => { if (value) setChunkMode(value); }}><ToggleGroupItem value="inherit">继承上游默认</ToggleGroupItem><ToggleGroupItem value="disabled">关闭分块</ToggleGroupItem><ToggleGroupItem value="custom">自定义</ToggleGroupItem></ToggleGroup><FieldDescription>{chunkMode === 'inherit' ? '沿用 YouTube 解析引擎的默认分块策略，不等同于关闭分块。' : '仅影响新任务的 HTTP 媒体下载，不改变视频规格和音轨；速度取决于网络与 CDN。'}</FieldDescription></Field>
        {chunkMode === 'custom' && <Field data-invalid={chunkInvalid || undefined}><FieldLabel htmlFor="youtube-chunk-size">自定义分块大小（MiB）</FieldLabel><Input id="youtube-chunk-size" type="number" inputMode="numeric" min={1} max={1024} step={1} value={chunkSize} onChange={event => setChunkSize(event.target.value)} aria-invalid={chunkInvalid} aria-describedby="youtube-chunk-size-help" className="max-w-48" /><FieldDescription id="youtube-chunk-size-help">1–1024 MiB 的整数，10 MiB 可作为起始测试值。</FieldDescription>{chunkInvalid && <FieldError>请输入 1–1024 之间的整数。</FieldError>}</Field>}
        <Separator />
      </>}
      <Field><FieldLabel htmlFor={profile.platform + '-ua'}>User-Agent</FieldLabel><Textarea id={profile.platform + '-ua'} rows={3} value={ua} onChange={e => setUa(e.target.value)} /><FieldDescription>使用与 Cookie 来源一致的浏览器标识。</FieldDescription></Field>
      {message && (message.failed || !changed) && <Alert variant={message.failed ? 'destructive' : 'default'} role={message.failed ? 'alert' : 'status'} className={cn('border-0 bg-transparent p-0', !message.failed && 'text-success')}>{message.failed ? <AlertCircle aria-hidden="true" /> : <CircleCheck aria-hidden="true" />}<AlertDescription className="text-xs text-inherit">{message.text}</AlertDescription></Alert>}
    </FieldGroup></FieldSet></form></CardContent><CardFooter className="justify-end py-2"><Button type="submit" form={'profile-' + profile.platform} disabled={busy || !changed || chunkInvalid}>{busy && <Spinner aria-hidden="true" data-icon="inline-start" />}{busy ? '保存中…' : '保存平台配置'}</Button></CardFooter>
  </Card>;
}
function App() {
  useEffect(() => {
    const preventContextMenu = (event: MouseEvent) => {
      if (!(event.target instanceof Element && event.target.closest('#video-input, [data-cover-context-menu]'))) event.preventDefault();
    };
    document.addEventListener('contextmenu', preventContextMenu, true);
    return () => document.removeEventListener('contextmenu', preventContextMenu, true);
  }, []);
  const [page, setPage] = useState('download');
  const [settingsSection, setSettingsSection] = useState<SettingsSection>('download');
  const runtimeDependencies = useRuntimeDependencies();
  const videoTranscripts = useVideoTranscripts();
  const [settingsPlatform, setSettingsPlatform] = useState('douyin');
  const [tdlBusy, setTdlBusy] = useState(false);
  const settingsSectionInfo = settingsSections.find(section => section.id === settingsSection);
  const [tasks, setTasks] = useState<Task[]>([]);
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const [exitingTasks, setExitingTasks] = useState<Task[]>([]);
  const taskOrder = useRef(new Map<string, number>());
  const taskListRef = useRef<HTMLDivElement>(null);
  const finishRemoval = useCallback((id: string) => setExitingTasks(old => old.filter(task => task.id !== id)), []);
  const [settings, setSettings] = useState<Settings | null>(null);
  const tdlDependency = useTdlDependency(settings?.tdl.path, setTdlBusy);
  const [input, setInput] = useState('');
  const [pasting, setPasting] = useState(false);
  const [resolver, setResolver] = useState<Resolver>('auto');
  const [viewAlbum, setViewAlbum] = useState<string | null>(null);
  const viewedAlbum = tasks.find(task => task.id === viewAlbum);
  const [selected, setSelected] = useState<string | null>(null);
  const [previewReset, setPreviewReset] = useState(false);
  // UI handoff only: keep acknowledged background tasks out of preview selection,
  // even when their authoritative queued/downloading event has not arrived yet.
  const [backgroundedIds, setBackgroundedIds] = useState(() => new Set<string>());
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const [confirm, setConfirm] = useState<Task | null>(null);
  const [duplicate, setDuplicate] = useState<{task: Task; previous: Task; background?: boolean} | null>(null);
  const duplicateTrigger = useRef<{id: string; background: boolean} | null>(null);
  useEffect(() => { if (duplicate) duplicateTrigger.current = {id: duplicate.task.id, background: !!duplicate.background}; }, [duplicate]);
  const promptedDuplicates = useRef(new Set<string>());
  const [search, setSearch] = useState('');
  const [taskPlatform, setTaskPlatform] = useState('all');
  const [clearHistoryPlatform, setClearHistoryPlatform] = useState<string | null>(null);
  const clearHistoryTrigger = useRef<HTMLButtonElement>(null);
  const taskSearchRef = useRef<HTMLInputElement>(null);
  const [wide, setWide] = useState(window.innerWidth > 1100);
  const lastStatuses = useRef<Map<string, Task["status"]> | null>(null);
  const submittedInput = useRef('');
  const removedIds = useRef(new Set<string>());
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const cancelTrigger = useRef<HTMLButtonElement | null>(null);
  const preview = tasks.find(t => !backgroundedIds.has(t.id) && (t.status === 'ready' || (t.status === 'parsing' && !t.download_requested)));
  const runningCount = tasks.filter(t => t.status === 'downloading').length;
  const queued = tasks.filter(t => t.status === 'queued').sort((a, b) => (a.queue_order ?? 0) - (b.queue_order ?? 0));
  const activeCount = runningCount + queued.length + tasks.filter(t => t.download_requested && t.status === 'parsing').length;
  const selectedTask = tasks.find(t => t.id === selected && !backgroundedIds.has(t.id));
  const current = previewReset ? undefined : selectedTask?.status === 'completed' ? preview : selectedTask ?? preview;
  const resolution = current?.video?.width && current.video.height ? `${current.video.width} × ${current.video.height}` : '—';
  const previewBytes = current?.status === 'completed' ? current.bytes : current?.total_bytes ?? current?.video?.filesize;
  const previewSize = previewBytes != null ? bytes(previewBytes) : current?.video?.filesize_approx != null ? '约 ' + bytes(current.video.filesize_approx) : '—';
  useEffect(() => {
    if (!connected) return;
    if (lastStatuses.current) {
      for (const task of tasks) {
        if (task.status === 'completed' && lastStatuses.current.get(task.id) !== 'completed') {
          toast.success('下载完成', { id: task.id, description: task.video?.title || '视频已保存到本地', duration: 5000 });
        } else if (task.status === 'failed' && lastStatuses.current.get(task.id) !== 'failed') {
          toast.error(task.download_requested ? '下载失败' : '解析失败', { id: task.id, description: errorText(task.error), duration: 8000 });
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
    if (current?.status !== 'ready' || promptedDuplicates.current.has(duplicatePromptKey(current))) return;
    const previous = previousDownload(tasks, current);
    if (previous) { promptedDuplicates.current.add(duplicatePromptKey(current)); setDuplicate({task: current, previous}); }
  }, [current, tasks]);
  async function download(task: Task, background = false, confirmDuplicate = false) {
    const latest = tasksRef.current.find(t => t.id === task.id) ?? task;
    confirmDuplicate = confirmDuplicate && !!task.video && !!latest.video && sameDownloadVariant(task.video, latest.video);
    task = latest;
    const previous = task.video && !confirmDuplicate && previousDownload(tasksRef.current, task);
    if (previous) { promptedDuplicates.current.add(duplicatePromptKey(task)); setDuplicate({task, previous, background}); return; }
    try {
      await api.download(task.id, confirmDuplicate);
      if (background) {
        setBackgroundedIds(old => new Set(old).add(task.id));
        setSelected(null); setPreviewReset(true); setInput(''); submittedInput.current = '';
        requestAnimationFrame(() => inputRef.current?.focus());
        toast.success('已加入下载任务');
        // Fill the task list from Rust without waiting for its queued event.
        void api.tasks().then(snapshot => setTasks(old => mergeTasks(old, snapshot, removedIds.current))).catch(() => {});
      }
    }
    catch (error) {
      if (String(error) !== 'ALREADY_DOWNLOADED') throw error;
      const snapshot = await api.tasks();
      // A completion may have raced the last UI snapshot. Recheck against the refreshed list.
      const previous = previousDownload(snapshot, snapshot.find(t => t.id === task.id) ?? task);
      if (previous) promptedDuplicates.current.add(duplicatePromptKey(task));
      setTasks(old => mergeTasks(old, snapshot, removedIds.current));
      if (previous) setDuplicate({task, previous, background}); else throw error;
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
        unsubscribe = await api.subscribe(task => { if (!disposed) setTasks(old => mergeTasks(old, [task], removedIds.current)); }, id => { if (!disposed) removeTasks([id]); });
        if (disposed) { unsubscribe(); return; }
        const [snapshot, config] = await Promise.all([api.tasks(), api.settings()]);
        if (!disposed) {
          setTasks(old => mergeTasks(old, snapshot, removedIds.current)); setSettings(config); setConnected(true);
          const restored = snapshot.find(task => task.status === 'ready' && task.video);
          if (restored?.video) {
            promptedDuplicates.current.add(duplicatePromptKey(restored));
            setInput(restored.video.canonical_url);
          }
        }
      } catch { if (!disposed) setError('无法连接本地应用，请通过桌面程序启动映取。'); }
    })();
    const refresh = () => { api.tasks().then(snapshot => { if (!disposed) setTasks(old => mergeTasks(old, snapshot, removedIds.current)); }).catch(() => {}); };
    window.addEventListener('focus', refresh);
    return () => { disposed = true; unsubscribe?.(); window.removeEventListener('focus', refresh); };
  }, []);
  async function run(action: () => Promise<unknown>) {
    setError(''); setBusy(true);
    try { await action(); } catch (e) {
      const message = errorText(e);
      setError(message);
      toast.error('操作失败', { id: 'action-error', description: message, duration: 8000 });
    } finally { setBusy(false); }
  }
  async function pasteLink() {
    if (pasting || busy || preview?.status === 'parsing') return;
    setPasting(true);
    try {
      const text = await api.readClipboardText();
      if (!text.trim()) { toast.info('剪贴板中没有文本内容'); return; }
      setInput(text); setError(''); inputRef.current?.focus();
    } catch { toast.error('无法读取剪贴板，请使用 Ctrl / ⌘ + V 粘贴。'); }
    finally { setPasting(false); }
  }
  function removeTasks(ids: string[]) {
    videoTranscripts.remove(ids);
    const fresh = new Set(ids.filter(id => !removedIds.current.has(id)));
    if (!fresh.size) return;
    fresh.forEach(id => removedIds.current.add(id));
    const rendered = new Set(Array.from(taskListRef.current?.querySelectorAll<HTMLElement>('[data-task-row]') ?? [], row => row.dataset.taskRow));
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
      const departing = tasksRef.current.filter(task => fresh.has(task.id) && rendered.has(task.id));
      setExitingTasks(old => [...old, ...departing]);
    }
    setTasks(old => old.filter(task => !fresh.has(task.id)));
    setSelected(old => old && fresh.has(old) ? null : old);
    setViewAlbum(old => old && fresh.has(old) ? null : old);
  }
  async function removeTask(id: string, deleteFile = false) {
    if (deleteFile) await api.deleteFile(id); else await api.remove(id);
    removeTasks([id]);
  }
  async function parse(event: React.FormEvent) {
    event.preventDefault();
    if (pasting || busy) return;
    if (!input.trim()) { setError('请输入视频分享链接'); inputRef.current?.focus(); return; }
    await run(async () => {
      if (preview?.status === 'ready') await api.cancel(preview.id);
      const task = await api.resolve(input, resolver);
      submittedInput.current = input;
      setTasks(old => mergeTasks(old, [task], removedIds.current)); setSelected(task.id); setPreviewReset(false);
    });
  }
  const nav = [{id:'download',label:'下载作品',Icon:Download},{id:'tasks',label:'下载任务',Icon:ListVideo},{id:'settings',label:'系统设置',Icon:SettingsIcon},{id:'about',label:'关于软件',Icon:Info}];
  // Equal timestamps must retain row order while removed snapshots animate.
  for (const task of [...tasks, ...exitingTasks]) if (!taskOrder.current.has(task.id)) taskOrder.current.set(task.id, taskOrder.current.size);
  const visible = [...tasks, ...exitingTasks].filter(t => t.download_requested).filter(t => taskPlatform === 'all' || (t.video?.platform ?? (t.telegram_link ? 'telegram' : undefined)) === taskPlatform).filter(t => !search || ((t.video?.title ?? '') + (t.video?.author ?? '') + (t.telegram_link ?? '')).includes(search)).sort((a, b) => (a.status === 'queued' && b.status === 'queued' ? (a.queue_order ?? 0) - (b.queue_order ?? 0) : a.status === 'queued' ? -1 : b.status === 'queued' ? 1 : b.created_at - a.created_at) || taskOrder.current.get(a.id)! - taskOrder.current.get(b.id)!);
  const completed = tasks.filter(t => t.status === 'completed');
  const historyCount = tasks.filter(t => t.download_requested && (taskPlatform === 'all' || (t.video?.platform ?? (t.telegram_link ? 'telegram' : undefined)) === taskPlatform) && ['completed', 'failed', 'canceled'].includes(t.status)).length;
  function taskActions(task: Task) {
    return <div className="flex flex-wrap items-center justify-end gap-2">
      {page === 'download' && task.status === 'ready' && <Button size="sm" variant="outline" data-background-download-task={task.id} disabled={busy} onClick={() => run(() => download(task, true))}><ListVideo data-icon="inline-start" />后台下载</Button>}
      {task.status === 'failed' && task.error === 'TDL_CHANNEL_ID_REQUIRED' && task.telegram_link && <Button disabled={busy} onClick={() => run(() => download(task, true))}><ListVideo data-icon="inline-start" />加入待下载队列</Button>}
      {task.status === 'ready' && <Button data-download-task={task.id} disabled={busy} onClick={() => run(() => download(task))}><Download data-icon="inline-start" />{task.video?.kind === 'image_album' ? '下载图集' : '下载视频'}</Button>}
      {['parsing','ready','queued','downloading'].includes(task.status) && <Button size="sm" variant="outline" disabled={busy || task.canceling} onClick={event => { cancelTrigger.current = event.currentTarget; setConfirm(task); }}>{task.canceling ? '正在取消…' : '取消'}</Button>}
      {task.status === 'completed' && task.video?.kind === 'image_album' && !task.file_missing && <Button size="sm" variant="outline" onClick={() => setViewAlbum(task.id)}><Eye />查看图集</Button>}
      {task.status === 'completed' && task.video?.kind !== 'image_album' && !task.file_missing && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="播放视频" disabled={busy} onClick={() => run(() => api.play(task.id))}><Play aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>使用系统默认播放器播放</TooltipContent></Tooltip>}
      {task.status === 'completed' && task.video?.kind !== 'image_album' && (!task.file_missing || videoTranscripts.values[task.id]?.has_result) && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" aria-label={videoTranscripts.values[task.id]?.state === 'running' ? '正在提取文案' : videoTranscripts.values[task.id]?.has_result ? '查看文案' : '提取文案'} disabled={busy} onClick={() => videoTranscripts.open(task.id)}>{videoTranscripts.values[task.id]?.state === 'running' ? <Spinner aria-hidden="true" /> : <FileText aria-hidden="true" />}</Button></TooltipTrigger><TooltipContent>{videoTranscripts.values[task.id]?.has_result ? '查看已保存的视频文案' : '提取视频语音文案'}</TooltipContent></Tooltip>}
      {task.status === 'completed' && !task.file_missing && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="打开文件夹" disabled={busy} onClick={() => run(() => api.reveal(task.id))}><FolderOpen aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>打开文件夹</TooltipContent></Tooltip>}
      {['failed', 'canceled'].includes(task.status) && <Button size="sm" variant="outline" disabled={busy} onClick={() => task.video || (task.telegram_link && task.download_requested) ? run(async () => { const next = await api.retry(task.id); toast.dismiss(task.id); promptedDuplicates.current.delete(next.id); setBackgroundedIds(old => { const updated = new Set(old); updated.delete(next.id); return updated; }); setTasks(old => mergeTasks(old, [next], removedIds.current)); setSelected(next.id); setPreviewReset(false); }) : (setPage('download'), setSelected(null), setInput(''), requestAnimationFrame(() => inputRef.current?.focus()))}><RotateCcw />{task.video || (task.telegram_link && task.download_requested) ? '重试' : '重新解析'}</Button>}
      {['completed', 'failed', 'canceled'].includes(task.status) && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="从列表移除" disabled={busy} onClick={() => run(() => removeTask(task.id))}><ListX aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>从列表移除（保留本地文件）</TooltipContent></Tooltip>}
      {(task.video || task.telegram_link) && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="复制原链接" disabled={busy} onClick={() => run(() => navigator.clipboard.writeText(task.video?.canonical_url ?? task.telegram_link!))}><Copy aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>复制原链接</TooltipContent></Tooltip>}
      {task.status === 'completed' && !task.file_missing && <Tooltip><TooltipTrigger asChild><Button size="icon-sm" variant="ghost" className="text-destructive hover:text-destructive" aria-label={task.video?.kind === 'image_album' ? '删除本地图集' : '删除本地视频'} disabled={busy} onClick={() => run(() => removeTask(task.id, true))}><Trash2 aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>{task.video?.kind === 'image_album' ? '删除本地图集及下载记录' : '删除本地视频及下载记录'}</TooltipContent></Tooltip>}
    </div>;
  }
  function emptyState(history = false) {
    return <Card size="sm" className="[--card-spacing:12px]"><CardContent><Empty className="min-h-[160px] gap-2 p-4"><EmptyHeader><EmptyMedia variant="icon">{history ? <ListVideo /> : <Film />}</EmptyMedia><EmptyTitle>{history ? search ? '没有匹配的下载任务' : taskPlatform !== 'all' ? '该平台暂无下载任务' : '还没有下载记录' : '粘贴链接开始解析'}</EmptyTitle><EmptyDescription>{history ? search ? '试试其他作品标题或作者，或切换平台' : taskPlatform !== 'all' ? '切换“全部”查看其他平台的任务' : '下载任务和已完成记录会显示在这里' : '作品信息将显示在这里'}</EmptyDescription></EmptyHeader><EmptyContent>{page === 'download' ? <div className="flex gap-2"><Badge variant="secondary">抖音</Badge><Badge variant="secondary">TikTok</Badge><Badge variant="secondary">YouTube</Badge></div> : <Button size="sm" variant="outline" onClick={() => setPage('download')}>去下载作品</Button>}</EmptyContent></Empty></CardContent></Card>;
  }
  return <TooltipProvider><Toaster position="top-center" closeButton /><SidebarProvider className="h-dvh min-h-0 overflow-hidden" open={wide} onOpenChange={setWide} style={{'--sidebar-width':'176px','--sidebar-width-icon':'56px'} as React.CSSProperties}>
    <Sidebar collapsible="icon"><SidebarHeader className="h-16 justify-center px-2"><div data-slot="app-brand" className="flex h-10 items-center gap-2 px-2 group-data-[collapsible=icon]:px-0"><img src={brandIcon} alt="" className="size-10 shrink-0 object-contain" /><span className="grid min-w-0 flex-1 grid-cols-[auto_minmax(0,1fr)] items-center gap-2 group-data-[collapsible=icon]:hidden"><span className="whitespace-nowrap text-xl font-bold tracking-tight">映取</span><Badge variant="outline" className="min-w-0 max-w-full justify-self-end" title={__APP_VERSION__}><span className="truncate">{__APP_VERSION__}</span></Badge></span></div></SidebarHeader><SidebarSeparator /><SidebarContent><SidebarGroup className="px-2 py-2"><SidebarGroupContent><SidebarMenu className="gap-1">{nav.map(({id,label,Icon}) => <SidebarMenuItem key={id}><SidebarMenuButton className="h-8" isActive={page === id} aria-current={page === id ? 'page' : undefined} tooltip={label} onClick={() => { setPage(id); setSearch(''); }}><Icon /><span>{label}</span></SidebarMenuButton>{id === 'tasks' && activeCount > 0 && <SidebarMenuBadge>{activeCount}</SidebarMenuBadge>}</SidebarMenuItem>)}</SidebarMenu></SidebarGroupContent></SidebarGroup></SidebarContent><SidebarSeparator /><SidebarFooter className="p-2"><SidebarMenu><SidebarMenuItem><SidebarMenuButton tooltip={connected ? '本地运行' : '连接不可用'}><HardDrive /><span>{connected ? '本地运行' : '连接不可用'}</span></SidebarMenuButton></SidebarMenuItem></SidebarMenu></SidebarFooter></Sidebar>
    <SidebarInset className="min-h-0 min-w-0 overflow-hidden bg-background"><header className="mx-4 mt-4 flex min-h-10 flex-wrap shrink-0 items-center justify-between gap-4 border-b pb-3"><div className="flex flex-wrap items-baseline gap-x-3 gap-y-1"><h1 className="text-xl leading-[1.3] font-semibold tracking-tight">{nav.find(n => n.id === page)?.label}</h1><p className={cn("text-muted-foreground", page !== 'tasks' && "text-xs")}>{page === 'download' ? '粘贴作品链接，预览并保存到本地' : page === 'tasks' ? `${runningCount} 个下载中 · ${queued.length} 个等待 · ${completed.length} 个已完成 · 并发上限 ${settings?.max_downloads ?? '—'}` : page === 'about' ? '了解映取、获取支持与查看开源信息' : '管理下载、平台账号与运行环境'}</p></div>{page === 'tasks' && <div className="flex max-w-full items-center gap-2"><Tooltip><TooltipTrigger asChild><span className="inline-flex"><Button ref={clearHistoryTrigger} size="icon-sm" variant="ghost" aria-label="清理历史" disabled={busy || historyCount === 0} onClick={() => setClearHistoryPlatform(taskPlatform)}><ListX /></Button></span></TooltipTrigger><TooltipContent>{historyCount ? `清理${taskPlatform === 'all' ? '全部平台' : platformName(taskPlatform)}下载历史` : '当前平台暂无可清理的下载历史'}</TooltipContent></Tooltip><InputGroup className="h-7 w-[260px] max-w-full"><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput ref={taskSearchRef} aria-label="搜索作品标题或作者" placeholder="搜索作品标题或作者" value={search} onChange={e => setSearch(e.target.value)} /></InputGroup></div>}</header>
    <div key={page} data-slot="page-content" className={cn("min-h-0 flex-1", page === 'tasks' || page === 'settings' ? "overflow-hidden" : "overflow-y-auto", page === 'about' && "mr-1")}><div className={cn("flex w-full flex-col gap-4 p-4", (page === 'tasks' || page === 'settings') && "h-full min-h-0")}>
    {error && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{error}</AlertDescription></Alert>}
    {page === 'download' && <div className="flex flex-col gap-3"><Card size="sm" className="[--card-spacing:12px]"><CardHeader><CardTitle>作品链接</CardTitle></CardHeader><CardContent><form onSubmit={parse}><FieldGroup className="gap-3"><Field orientation="horizontal"><FieldLabel className="sr-only" htmlFor="video-input">作品分享链接</FieldLabel><InputGroup className="min-h-8 flex-1"><ShareLinkInput value={input} onChange={setInput} inputRef={inputRef} /></InputGroup><div className="flex shrink-0 flex-col gap-2"><Button type="button" variant="outline" className="min-w-[100px]" disabled={busy || pasting || preview?.status === 'parsing'} onClick={() => void pasteLink()}>{pasting ? <Spinner aria-hidden="true" data-icon="inline-start" /> : <ClipboardPaste data-icon="inline-start" />}{pasting ? '粘贴中…' : '粘贴链接'}</Button><Button className="min-w-[100px]" variant={current?.video ? 'secondary' : 'default'} disabled={!connected || busy || pasting || preview?.status === 'parsing'}>{preview?.status === 'parsing' ? <Spinner data-icon="inline-start" /> : <Link data-icon="inline-start" />}{preview?.status === 'parsing' ? '正在解析…' : current?.video ? '重新解析' : '解析链接'}</Button></div></Field><Field orientation="horizontal"><FieldLabel id="resolver-label">解析引擎</FieldLabel><Select value={resolver} onValueChange={value => setResolver(value as Resolver)} disabled={busy || preview?.status === 'parsing'}><SelectTrigger size="sm" aria-labelledby="resolver-label" className="w-44"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="auto">自动 · 按平台选择</SelectItem><SelectItem value="yt-dlp">yt-dlp · 视频</SelectItem><SelectItem value="dtk">DTK · 视频与图集</SelectItem><SelectItem value="tdl">tdl · Telegram 实验模式</SelectItem></SelectContent></Select></Field><FieldDescription>支持抖音、TikTok、YouTube 与 Telegram · 自动识别平台<span className="float-right">Ctrl / ⌘ + V 粘贴</span></FieldDescription></FieldGroup></form></CardContent></Card>
    {current ? <Card size="sm" className="[--card-spacing:12px]"><CardHeader><div className="flex items-center gap-2"><Badge variant="outline">{platformName(current.video?.platform ?? (current.telegram_link ? 'telegram' : undefined))}</Badge>{current.video?.resolver && <Badge variant="secondary">{current.video.resolver === 'dtk' ? 'DTK' : current.video.resolver === 'tdl' ? 'tdl' : 'yt-dlp'}</Badge>}<Badge variant={current.canceling ? 'warning' : statusVariant[current.status]}>{current.canceling ? '正在取消…' : current.status === 'queued' && !current.video && current.telegram_link ? '等待解析' : labels[current.status]}</Badge>{current.cache_hit && <Badge variant="muted">解析缓存</Badge>}</div></CardHeader><CardContent className="flex gap-4"><>{current.video?.kind === 'image_album' ? <AlbumGallery key={current.id} task={current} /> : <VideoCover task={current} />}</><div className="flex min-w-0 flex-1 flex-col gap-3"><CardTitle className="line-clamp-2" title={current.video?.title ?? ''}>{current.video?.title || (current.status === 'parsing' ? '正在获取作品信息' : current.telegram_link && (current.status === 'queued' || current.error === 'TDL_CHANNEL_ID_REQUIRED') ? '等待 Telegram 服务空闲' : '等待有效视频链接')}</CardTitle><CardDescription className="text-sm">{current.video?.author ? '@' + current.video.author : '作者 —'}</CardDescription><dl className="grid grid-cols-3 gap-4"><div><dt className="text-xs text-muted-foreground">{current.video?.kind === 'image_album' ? '图片数量' : '时长'}</dt><dd className="mt-2 tabular-nums">{current.video?.kind === 'image_album' ? `${current.video.images?.length ?? 0} 张` : duration(current.video?.duration_ms)}</dd></div><div><dt className="text-xs text-muted-foreground">{current.video?.kind === 'image_album' ? '图片质量' : '清晰度'}</dt><dd className="mt-2 tabular-nums">{current.video?.kind === 'image_album' ? '原图优先，高清次选' : resolution}</dd></div><div><dt className="text-xs text-muted-foreground">文件大小</dt><dd className="mt-2 tabular-nums">{previewSize}</dd></div></dl>{current.status === 'ready' && !!current.video?.formats?.length && <Field className="gap-2"><FieldLabel>下载规格 · {current.video.formats.length} 个</FieldLabel><ToggleGroup type="single" variant="outline" aria-label="下载视频规格" value={current.video.selected_format_id ?? ''} disabled={busy} onValueChange={value => { if (value) void run(async () => { const task = await api.chooseFormat(current.id, value); setTasks(old => mergeTasks(old, [task], removedIds.current)); }); }} className="grid max-h-[196px] w-full grid-cols-2 gap-2 overflow-y-auto p-1 pr-3 max-[1100px]:grid-cols-1">{current.video.formats.map(format => <ToggleGroupItem key={format.format_id} value={format.format_id} className="h-auto min-w-0 justify-between gap-2 px-3 py-2 text-left data-[state=on]:border-primary data-[state=on]:bg-accent data-[state=on]:text-primary"><span className="flex min-w-0 flex-col gap-0.5"><span className="font-medium">{format.format_id === 'original' ? '原始文件' : format.width && format.height ? `${format.width} × ${format.height}` : '清晰度未知'}</span><span className="text-[11px] text-muted-foreground">{format.container.toUpperCase()}{format.requires_merge ? ' · 需 FFmpeg' : ''} · {format.codec === 'h265' ? 'H.265' : format.codec === 'h264' ? 'H.264' : format.codec === 'av1' ? 'AV1' : format.codec || '编码未知'}{format.fps ? ` · ${format.fps} fps` : ''}{format.tbr ? ` · ${(format.tbr / 1000).toFixed(2)} Mbps` : ''}</span></span><span className="shrink-0 tabular-nums">{format.filesize != null ? bytes(format.filesize) : format.filesize_approx != null ? '约 ' + bytes(format.filesize_approx) : '大小未知'}</span></ToggleGroupItem>)}</ToggleGroup><FieldDescription>{current.video.platform === 'telegram' ? '仅下载真实原始文件，不转码；大小、类型或文件名变化时需要重新解析。tdl 服务可能被网络中的其他设备访问，请仅在可信网络中使用。' : current.video.platform === 'douyin' && current.video.resolver === 'yt-dlp' ? '仅展示已确认含视频和音轨的一体规格；默认最高分辨率，下载时重新获取所选规格。' : '默认最高分辨率；下载时重新获取所选规格。YouTube 分离音视频规格需本机安装 FFmpeg，合并不转码。'}</FieldDescription></Field>}{current.video?.cookie_warning && <Alert className="border-0 bg-transparent p-0 text-muted-foreground"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{errorText(current.video.cookie_warning)}</AlertDescription></Alert>}{current.video?.music?.title && <CardDescription className="text-sm">原声：{current.video.music.title}{current.video.music.author ? ` · ${current.video.music.author}` : ''}</CardDescription>}{current.status === 'downloading' && <DownloadProgress task={current} />}{current.status === 'queued' && <CardDescription className="text-sm">{!current.video && current.telegram_link ? '等待 Telegram 服务空闲后自动解析并下载' : '等待下载'} · 第 {queued.findIndex(t => t.id === current.id) + 1} 位</CardDescription>}{current.error && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{errorText(current.error)}</AlertDescription></Alert>}{current.status === 'canceled' && <CardDescription>已取消，重试将重新解析并从头下载。</CardDescription>}{current.path && <p className="truncate text-xs text-muted-foreground" title={displayPath(current.path)}>{displayPath(current.path)}</p>}</div></CardContent><CardFooter className="justify-end py-2">{taskActions(current)}</CardFooter></Card> : emptyState()}
    <footer className="flex items-center gap-2 text-xs text-muted-foreground"><FolderOpen className="size-4 shrink-0" /><span className="min-w-0 truncate" title={displayPath(settings?.download_directory)}>下载目录：{displayPath(settings?.download_directory) || '—'}</span><Button variant="link" onClick={() => setPage('settings')}>修改设置</Button></footer></div>}
    {page === 'tasks' && <Tabs value={taskPlatform} onValueChange={setTaskPlatform} className="min-h-0 flex-1 gap-3"><TabsList variant="line" aria-label="按平台筛选下载任务" className="w-full shrink-0 justify-start border-b px-0">{['all', 'douyin', 'tiktok', 'youtube', 'telegram'].map(platform => <TabsTrigger key={platform} value={platform} className="min-w-20 flex-none px-4 data-[state=active]:text-primary after:bg-primary">{platform === 'all' ? '全部' : platformName(platform)}</TabsTrigger>)}</TabsList><TabsContent ref={taskListRef} value={taskPlatform} className="flex min-h-0 flex-col overflow-y-auto p-px pr-3">{visible.length ? visible.map(task => <TaskRemovalRow key={task.id} id={task.id} removing={removedIds.current.has(task.id)} onExited={finishRemoval}><Card size="sm" className="[--card-spacing:12px]"><CardContent className="flex items-center gap-3"><VideoCover task={task} small /><div className="flex min-w-0 flex-1 flex-col gap-1.5"><div className="flex min-w-0 items-center gap-2"><CardTitle className="truncate" title={task.video?.title ?? ''}>{task.video?.title || task.telegram_link || '单条视频解析'}</CardTitle><Badge className="shrink-0" variant={task.canceling ? 'warning' : statusVariant[task.status]}>{task.canceling ? '正在取消…' : task.status === 'queued' && !task.video && task.telegram_link ? '等待解析' : labels[task.status]}</Badge></div><div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground"><span>{platformName(task.video?.platform ?? (task.telegram_link ? 'telegram' : undefined))}</span>{task.video?.kind === 'image_album' && <span>图集 · {task.files.length || task.video.images?.length || 0} 张</span>}{task.video?.author && <span>@{task.video.author}</span>}<span>{new Date((task.completed_at ?? task.created_at) * 1000).toLocaleString('zh-CN')}</span>{task.status === 'completed' && <span>{bytes(task.bytes)}</span>}</div>{task.status === 'downloading' && <DownloadProgress task={task} />}{task.status === 'queued' && <p className="text-xs text-muted-foreground">队列第 {queued.findIndex(t => t.id === task.id) + 1} 位 · {!task.video && task.telegram_link ? '等待 Telegram 服务空闲后自动解析并下载' : '等待下载名额'}</p>}{task.error && <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive"><AlertCircle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" /><span>{errorText(task.error)}</span></p>}{task.cover_error && <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><AlertCircle className="size-3.5" />封面未保存</p>}{task.file_missing && <Alert className="py-2"><AlertCircle /><AlertDescription>文件已移动或删除</AlertDescription></Alert>}{task.path && <p className="truncate text-xs text-muted-foreground" title={displayPath(task.path)}>{displayPath(task.path)}</p>}</div><div className="max-w-[240px] shrink-0">{taskActions(task)}</div></CardContent></Card></TaskRemovalRow>) : emptyState(true)}</TabsContent></Tabs>}
    {page === 'settings' && settings && <div className="grid min-h-0 min-w-0 flex-1 grid-cols-[184px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-6 max-[1100px]:grid-cols-[160px_minmax(0,1fr)]">
      <nav aria-label="设置分类" className="settings-navigation self-start border-r pr-4"><p className="px-3 pb-3 pt-1 text-xs font-medium text-muted-foreground">设置分类</p><SidebarMenu className="gap-2">{settingsSections.map(({ id, label, Icon }) => <SidebarMenuItem key={id}><SidebarMenuButton type="button" className="h-10 gap-3 px-3" isActive={settingsSection === id} aria-label={label} aria-current={settingsSection === id ? 'true' : undefined} aria-controls="settings-operation-area" onClick={() => setSettingsSection(id)}><Icon aria-hidden="true" /><span className="text-[13px] font-medium">{label}</span></SidebarMenuButton></SidebarMenuItem>)}</SidebarMenu></nav>
      <section id="settings-operation-area" aria-label={settingsSectionInfo?.label} className="flex min-h-0 min-w-0 flex-col gap-4"><header className="flex shrink-0 flex-col gap-1"><h2 className="text-base font-semibold">{settingsSectionInfo?.label}</h2><p className="text-xs text-muted-foreground">{settingsSectionInfo?.description}</p>{settingsSection === 'dependencies' && <RuntimeDependenciesToolbar runtime={runtimeDependencies} />}</header>
        <div hidden={settingsSection !== 'download'} className="min-h-0 overflow-y-auto p-px pr-3"><Card size="sm" className="[--card-spacing:12px]"><CardContent><FieldGroup className="gap-3"><Field orientation="horizontal" className="grid grid-cols-[64px_minmax(0,1fr)]"><FieldLabel className="w-16 shrink-0" htmlFor="download-directory">下载目录</FieldLabel><div className="flex min-w-0 flex-1 items-center gap-2"><Input id="download-directory" className="min-h-8" readOnly value={displayPath(settings.download_directory)} /><Button variant="outline" disabled={busy} onClick={() => run(async () => setSettings(await api.directory()))}><FolderOpen data-icon="inline-start" />选择文件夹</Button></div></Field><Field orientation="horizontal" className="grid grid-cols-[96px_minmax(0,1fr)]"><FieldLabel id="concurrency-label">同时下载数量</FieldLabel><div className="flex flex-wrap items-center gap-3"><Select value={String(settings.max_downloads)} disabled={busy} onValueChange={value => void run(async () => setSettings(await api.concurrency(Number(value))))}><SelectTrigger size="sm" aria-labelledby="concurrency-label" className="w-24"><SelectValue /></SelectTrigger><SelectContent>{[1, 2, 3].map(value => <SelectItem key={value} value={String(value)}>{value} 个</SelectItem>)}</SelectContent></Select><FieldDescription>默认 2 个；降低上限不打断已开始的下载。</FieldDescription></div></Field></FieldGroup></CardContent></Card></div>
        <div hidden={settingsSection !== 'accounts'} className="min-h-0 flex-1"><Tabs value={settingsPlatform} onValueChange={setSettingsPlatform} className="settings-platforms h-full min-h-0 gap-5"><TabsList variant="line" aria-label="配置平台" className="w-full shrink-0 justify-start border-b px-0">{settings.profiles.map(profile => <TabsTrigger key={profile.platform} value={profile.platform} className="min-w-24 flex-none px-4">{platformName(profile.platform)}</TabsTrigger>)}<TabsTrigger value="telegram" className="min-w-24 flex-none px-4">Telegram</TabsTrigger></TabsList>{settings.profiles.map(profile => <TabsContent key={profile.platform} value={profile.platform} forceMount className="min-h-0 overflow-y-auto p-px pr-3 data-[state=inactive]:hidden"><ProfileForm profile={profile} saved={setSettings} /><Alert className="mt-5"><AlertCircle /><AlertDescription>公开作品解析成功只证明请求成功，不代表登录身份有效。</AlertDescription></Alert></TabsContent>)}<TabsContent value="telegram" forceMount className="min-h-0 overflow-y-auto p-px pr-3 data-[state=inactive]:hidden"><TdlAccountCard settings={settings} saved={setSettings} busy={tdlBusy} setBusy={setTdlBusy} /></TabsContent></Tabs></div>
        <div hidden={settingsSection !== 'dependencies'} className="flex min-h-0 flex-col gap-4 overflow-y-auto p-px pr-3"><RuntimeDependencies runtime={runtimeDependencies}><TdlDependencyCard settings={settings} saved={setSettings} busy={tdlBusy} setBusy={setTdlBusy} dependency={tdlDependency} /></RuntimeDependencies></div>
      </section>
    </div>}
    {page === 'about' && <About busy={busy} openPage={(page, releaseTag) => { void run(() => api.openProjectPage(page, releaseTag)); }} copyVersion={() => { void run(async () => { await navigator.clipboard.writeText(aboutVersionInfo); toast.success('版本信息已复制'); }); }} />}
    </div></div></SidebarInset>
    <VideoTranscriptDialog client={videoTranscripts} tasks={tasks} runtime={runtimeDependencies} />
    <Dialog open={!!viewedAlbum} onOpenChange={open => { if (!open) setViewAlbum(null); }}><DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-[680px]"><DialogHeader><DialogTitle>{viewedAlbum?.video?.title || '图集'}</DialogTitle><DialogDescription className="text-sm">{viewedAlbum?.files.length ?? 0} 张图片 · 保留原格式</DialogDescription></DialogHeader>{viewedAlbum && <AlbumGallery key={viewedAlbum.id} task={viewedAlbum} expanded />}</DialogContent></Dialog>
    <AlertDialog open={!!duplicate} onOpenChange={open => { if (!open) setDuplicate(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const button = document.querySelector<HTMLButtonElement>('[' + (duplicateTrigger.current?.background ? 'data-background-download-task' : 'data-download-task') + '="' + duplicateTrigger.current?.id + '"]'); ((button && !button.disabled ? button : null) ?? inputRef.current ?? document.querySelector<HTMLButtonElement>('[aria-current="page"]'))?.focus(); }); }}><AlertDialogHeader><AlertDialogTitle>该规格已下载过</AlertDialogTitle><AlertDialogDescription>{duplicate?.previous.video?.selected_format_id == null ? '已找到规格未知的历史成功记录，本地文件仍存在。' : '已找到同一下载规格的成功记录，本地文件仍存在。'}重新下载会保存为新文件，不覆盖原作品。<span className="mt-2 block break-all">{displayPath(duplicate?.previous.path)}</span></AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">暂不下载</AlertDialogCancel><AlertDialogAction size="sm" disabled={busy} onClick={() => { const pending = duplicate; setDuplicate(null); if (pending) void run(() => download(pending.task, pending.background, true)); }}>重新下载</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <AlertDialog open={clearHistoryPlatform !== null} onOpenChange={open => { if (!open && !busy) setClearHistoryPlatform(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { (clearHistoryTrigger.current && !clearHistoryTrigger.current.disabled ? clearHistoryTrigger.current : taskListRef.current)?.focus({preventScroll: true}); }); }}><AlertDialogHeader><AlertDialogTitle>清理下载历史？</AlertDialogTitle><AlertDialogDescription>移除{(clearHistoryPlatform ?? taskPlatform) === 'all' ? '全部平台' : platformName(clearHistoryPlatform ?? taskPlatform)}中已完成、失败和已取消的下载记录，不受当前搜索条件影响。进行中、排队任务和未提交的预览会保留，不会删除任何本地文件。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm" disabled={busy}>返回</AlertDialogCancel><AlertDialogAction size="sm" disabled={busy || clearHistoryPlatform === null} onClick={event => { event.preventDefault(); void run(async () => {
      const ids = await api.clearHistory(clearHistoryPlatform === 'all' ? null : clearHistoryPlatform);
      removeTasks(ids);
      setClearHistoryPlatform(null);
      if (ids.length) toast.success(`已清理 ${ids.length} 条下载历史`); else toast.info('暂无可清理的下载历史');
    }); }}>{busy ? '清理中…' : '清理历史'}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <AlertDialog open={!!confirm} onOpenChange={open => { if (!open) setConfirm(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); cancelTrigger.current?.focus(); }}><AlertDialogHeader><AlertDialogTitle>取消当前任务？</AlertDialogTitle><AlertDialogDescription>将取消等待，或停止当前解析 / 下载并清理暂存文件。其他任务不受影响，记录会保留。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">返回</AlertDialogCancel><AlertDialogAction size="sm" onClick={() => { const pending = confirm; setConfirm(null); if (pending) void run(() => api.cancel(pending.id)); }}>确认取消</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </SidebarProvider></TooltipProvider>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
