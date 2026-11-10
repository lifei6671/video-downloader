import '@/lib/style-nonce';
import { RefreshCw } from 'lucide-react';
import { bytes } from '@/lib/format-bytes';
import { BackupSettingsForm } from '@/components/backup-settings';
import { backupAssetStatus, backupDeletion, BackupDeleteProgress, BackupRestoreProgress, BackupAssetCoverBadge, UnifiedTaskBadge, BackupCardAction, backupCardStatus, BackupTaskStatus, BackupTaskFeedback, BackupTransferMenuItems, BackupAssetRow, canPlayBackupAsset, useBackupTasks } from '@/components/backup-tasks';
import React, { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Download, Play, ListVideo, ListX, CircleCheck, CircleX, Settings as SettingsIcon, Link, FolderOpen, Film, Eye, EyeOff, Copy, ClipboardPaste, AlertCircle, HardDrive, Search, RotateCcw, Trash2, Info, KeyRound, PackageCheck, Monitor, CalendarClock, Images, Maximize2, Minimize2, MoreHorizontal, X } from 'lucide-react';
import { cn } from 'cn';
import { api, errorText, mergeTasks, compareTaskInsertion, type BackupAsset, type Task, type Settings, type Profile, type Resolver } from './api';
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
import { Field, FieldContent, FieldGroup, FieldLabel, FieldDescription, FieldError, FieldSet } from '@/components/ui/field';
import { Checkbox } from '@/components/ui/checkbox';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Tabs, TabsList, TabsTrigger, TabsContent, settingsTabsListClassName, settingsTabsTriggerClassName } from '@/components/ui/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Spinner } from '@/components/ui/spinner';
import { Separator } from '@/components/ui/separator';
import { Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogClose } from '@/components/ui/dialog';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { SidebarProvider, Sidebar, SidebarHeader, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarMenu, SidebarMenuItem, SidebarMenuButton, SidebarMenuBadge, SidebarSeparator, SidebarInset } from '@/components/ui/sidebar';
import { Toaster } from '@/components/ui/sonner';
import { toast } from 'sonner';
import { displayPath } from '@/lib/path';
import { About, aboutVersionInfo } from '@/components/about';
import { RuntimeDependencies, RuntimeDependenciesToolbar, useRuntimeDependencies } from '@/components/runtime-dependencies';
import { TelegramStorageSettings } from '@/components/telegram-storage-settings';
import { ShareLinkInput } from '@/components/share-link-input';
import { ContextMenu, ContextMenuTrigger, ContextMenuContent, ContextMenuGroup, ContextMenuItem } from '@/components/ui/context-menu';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from '@/components/ui/dropdown-menu';
import { RetainedTaskPage, TaskList, taskListRows, type TaskView } from '@/components/task-list';
import { TaskRow, labels, statusVariant } from '@/components/task-row';
import { TaskStatusFilters, type TaskStatusFilter } from '@/components/task-filters';
import { VideoTranscriptDialog, useVideoTranscripts } from '@/components/video-transcript';
import { previousDownload, duplicatePromptKey, sameDownloadVariant, sameReplacementVariant } from '@/lib/download-identity';
import { FilenameSettings, type FilenamePreviewCache } from '@/components/filename-settings';
import { DownloadConcurrency } from '@/components/download-concurrency';
import { ResolveProgressCard } from '@/components/resolve-progress';
import { AuthorInfo } from '@/components/author-info';
import { AuthorDownload } from '@/components/author-download';
import { TaskMoreMenu, type CoverMenuActions } from '@/components/task-more-menu';
import { PlaybackSession, type MediaSize, type PlaybackSubject } from '@/components/video-player';
import { forgetPlaybackPositions } from '@/lib/playback-position';
import { authorSearch, matchesAuthor, type AuthorSearch } from '@/lib/author-search';
import './style.css';
import { useTaskImage, forgetTaskImages, readTaskImageSize } from '@/lib/task-image-cache';

const taskViewStorageKey = 'yingqu.task-view';
const brandIcon = new URL('../src-tauri/icons/64x64.png', import.meta.url).href;
const platformName = (platform?: string) => platform === 'douyin' ? '抖音' : platform === 'xiaohongshu' ? '小红书' : platform === 'tiktok' ? 'TikTok' : platform === 'youtube' ? 'YouTube' : platform === 'telegram' ? 'Telegram' : '平台未知';
function duration(ms?: number) {
  if (ms == null) return '—';
  const seconds = Math.floor(ms / 1000);
  return Math.floor(seconds / 60) + ':' + String(seconds % 60).padStart(2, '0');
}
function remainingTime(seconds: number) {
  const total = Math.ceil(seconds);
  if (seconds <= 300) return `${total} 秒`;
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total % 3600 / 60);
  const remainder = total % 60;
  const parts = [];
  if (hours) parts.push(`${hours} 小时`);
  if (minutes) parts.push(`${minutes} 分`);
  if (remainder || !parts.length) parts.push(`${remainder} 秒`);
  return parts.join(' ');
}
function resolutionLabel(width?: number | null, height?: number | null) {
  return width && height ? `${Math.min(width, height)}P` : '清晰度未知';
}
function downloadTime(timestamp: number | null) {
  if (timestamp == null) return '—';
  const date = new Date(timestamp * 1000);
  return `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日 ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}
function albumImageCount(task: Task) {
  return task.files.length || task.video?.images?.length || 0;
}
function TaskMetadataItem({ label, children, ...props }: React.ComponentProps<'span'> & { label: string }) {
  return <Tooltip><TooltipTrigger asChild><span tabIndex={0} {...props}>{children}</span></TooltipTrigger><TooltipContent collisionPadding={8}>{label}</TooltipContent></Tooltip>;
}
function TaskMetadata({ task, details = false, compact = false, hideMissing = false }: { task: Task; details?: boolean; compact?: boolean; hideMissing?: boolean }) {
  const video = task.video;
  const selected = video?.formats?.find(format => format.format_id === video.selected_format_id);
  const width = selected?.width ?? video?.width;
  const height = selected?.height ?? video?.height;
  const resolution = resolutionLabel(width, height);
  const imageCount = albumImageCount(task);
  const exactSize = selected?.filesize ?? video?.filesize;
  const approximateSize = selected?.filesize_approx ?? video?.filesize_approx;
  const size = task.status === 'completed' ? bytes(task.bytes) : exactSize != null ? bytes(exactSize) : approximateSize != null ? `约 ${bytes(approximateSize)}` : '大小未知';
  const downloadedAt = downloadTime(task.completed_at);
  return <div role="group" aria-label="下载信息" className={cn("flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground", compact && "task-row-metadata")} >
    {task.file_missing && !hideMissing && <Alert variant="destructive" className="inline-flex w-auto shrink-0 items-center gap-1.5 border-0 bg-transparent p-0 text-xs [&>svg]:size-3.5! [&>svg]:translate-y-0!"><AlertCircle aria-hidden="true" /><AlertDescription>文件不存在</AlertDescription></Alert>}
    {video?.kind === 'image_album' ? <TaskMetadataItem label="图片数量" className="inline-flex items-center gap-1.5"><Images aria-hidden="true" className="size-3.5 shrink-0" />{imageCount ? `${imageCount} 张图片` : '图片数量未知'}</TaskMetadataItem>
      : <TaskMetadataItem label={width && height ? `清晰度：${width} × ${height}` : '清晰度未知'} aria-label={`清晰度：${resolution}`} className="inline-flex items-center gap-1.5"><Monitor aria-hidden="true" className="size-3.5 shrink-0" />{resolution}</TaskMetadataItem>}
    <TaskMetadataItem label="文件大小" aria-label={`文件大小：${size}`} className="inline-flex items-center gap-1.5"><HardDrive aria-hidden="true" className="size-3.5 shrink-0" />{size}</TaskMetadataItem>
    {(details || (compact && task.completed_at != null)) && <TaskMetadataItem label={`下载时间：${downloadedAt}`} aria-label={`下载时间：${downloadedAt}`} className="inline-flex items-center gap-1.5"><CalendarClock aria-hidden="true" className="size-3.5 shrink-0" />{downloadedAt}</TaskMetadataItem>}
  </div>;
}
function taskTitle(task: Task) {
  const video = task.video;
  const platform = platformName(video?.platform ?? (task.telegram_link ? 'telegram' : undefined));
  return video?.title?.trim() || (video ? `${platform} ${video.kind === 'image_album' ? '图集' : '视频'} ${video.video_id}` : task.telegram_link ? `Telegram ${task.telegram_link}` : `下载任务 ${task.id}`);
}
function TaskDetails({ task, coverTools, regionRef }: { task: Task; coverTools?: React.ReactNode; regionRef: React.Ref<HTMLDivElement> }) {
  const video = task.video;
  const platform = platformName(video?.platform ?? (task.telegram_link ? 'telegram' : undefined));
  const authorId = video?.author_profile?.id ?? (video?.platform === 'telegram' ? video.video_id.split('_')[0] : undefined);
  return <div ref={regionRef} className="task-details-content" role="region" aria-label="任务详细信息" tabIndex={0}>
        <div className="flex min-w-0 items-center justify-between gap-2"><AuthorInfo task={task} showId={false} />{coverTools}</div>
        <h2 className="task-details-title text-sm font-semibold">{taskTitle(task)}</h2>
        <TaskMetadata task={task} details />
        <dl className="task-details-fields">
          <dt>来源</dt><dd>{platform}</dd>
          {authorId && <><dt>作者 ID</dt><dd>{authorId}</dd></>}
          {video && <><dt>作品 ID</dt><dd>{video.video_id}</dd></>}
          <dt>任务 ID</dt><dd>{task.id}</dd>
          <dt>创建时间</dt><dd>{downloadTime(task.created_at)}</dd>
          <dt>文件路径</dt><dd>{task.path ? displayPath(task.path) : '尚无本地文件'}</dd>
        </dl>
      </div>;
}
type AlbumImage = { source: string };
function GalleryImage({ task, index, onReadyChange }: { task: Task; index: number; onReadyChange?: (image: AlbumImage | undefined) => void }) {
  const { source, failed: readFailed } = useTaskImage(task, index);
  const [failed, setFailed] = useState(false);
  useEffect(() => { setFailed(false); }, [source]);
  useEffect(() => { if (readFailed) onReadyChange?.(undefined); }, [readFailed, onReadyChange]);
  return source && !failed && !readFailed ? <img src={source} className="size-full object-contain" alt={`第 ${index + 1} 张图片`} onLoad={() => onReadyChange?.({ source })} onError={() => { setFailed(true); onReadyChange?.(undefined); }} /> : failed || readFailed ? <Film className="m-auto size-6 text-muted-foreground" aria-label="图片预览不可用" /> : <Spinner className="m-auto" aria-label="正在加载图片" />;
}
function GalleryVideo({ task, index, open, fadeIn, onMediaReady }: { task: Task; index: number; open: boolean; fadeIn?: boolean; onMediaReady?: (ready: boolean) => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const mediaReady = useRef(false);
  const [source, setSource] = useState<string>();
  const [failure, setFailure] = useState('');
  useLayoutEffect(() => {
    if (!open) return;
    mediaReady.current = false;
    let active = true;
    const element = video.current;
    api.albumVideoSource(task.id, index).then(source => {
      if (active) setSource(source);
    }).catch(() => { if (active) setFailure('无法读取实况视频'); });
    return () => {
      active = false;
      if (element) { element.pause(); element.removeAttribute('src'); element.load(); }
    };
  }, [task.id, index, open]);
  useLayoutEffect(() => { if (failure) onMediaReady?.(false); }, [failure, onMediaReady]);
  const reportPlaying = () => {
    if (!open || mediaReady.current) return;
    mediaReady.current = true;
    onMediaReady?.(true);
  };
  return <>
    <video ref={video} src={source} autoPlay muted loop playsInline controls className={cn('relative size-full object-contain', fadeIn && 'album-slide-fade-in')} aria-label={`第 ${index + 1} 张实况视频`}
      onPlaying={reportPlaying}
      onTimeUpdate={event => {
        const element = event.currentTarget;
        if (!element.paused && !element.seeking && element.currentTime > 0) reportPlaying();
      }}
      onLoadedMetadata={event => {
        const { videoWidth, videoHeight } = event.currentTarget;
        if (!videoWidth || !videoHeight) setFailure('实况视频无法播放');
      }}
      onError={() => setFailure('实况视频无法播放')} />
    {!source && !failure && <div className="absolute inset-0 flex items-center justify-center"><Spinner aria-label="正在加载实况视频" /></div>}
    {failure && <div className="absolute inset-0 flex items-center justify-center bg-muted text-xs text-muted-foreground" role="alert">{failure}</div>}
  </>;
}
type AlbumStill = { index: number; image: AlbumImage };
type AlbumSelection = { index: number; image?: AlbumImage; previous?: AlbumStill; frame?: HTMLCanvasElement; nextIndex?: number; settled?: boolean; fadeIn?: boolean };
type MediaDirection = -1 | 1;
type AlbumNavigation = { step: (direction: MediaDirection) => boolean };
function albumMedia(task: Task) {
  if (task.video?.kind !== 'image_album' || task.file_missing) return;
  return task.video.images?.length ? task.video.images : task.files;
}
function AlbumGallery({ task, expanded = false, open = true, slideshow = false, startAtLast, navigation, onMediaReady }: {
  task: Task; expanded?: boolean; open?: boolean; slideshow?: boolean; startAtLast?: boolean; navigation?: React.Ref<AlbumNavigation>; onMediaReady?: (ready: boolean) => void;
}) {
  const images = task.video?.images?.length ? task.video.images : task.files;
  const [{ index, image, previous, frame, nextIndex, settled, fadeIn }, setSelection] = useState<AlbumSelection>(() => ({
    index: startAtLast ? images.length - 1 : 0,
  }));
  const activeDot = useRef<HTMLButtonElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const selected = images[index];
  const isLive = useCallback((index: number) => task.status === 'completed' && !!task.files[index]?.video, [task.status, task.files]);
  const live = isLive(index);
  const imageReady = useCallback((image: AlbumImage | undefined) => {
    onMediaReady?.(!!image);
    setSelection(current => current.index === index
      ? { ...current, image, settled: true, previous: image ? current.previous : undefined }
      : current);
  }, [index, onMediaReady]);
  const liveReady = useCallback((ready: boolean) => {
    onMediaReady?.(ready);
    setSelection(current => current.index === index && !current.settled ? { ...current, settled: true } : current);
  }, [index, onMediaReady]);
  const transitionTo = useCallback((current: AlbumSelection, next: number, animate: boolean): AlbumSelection => {
    if (next === (current.nextIndex ?? current.index)) return current;
    // Let the current fade finish before showing another slide.
    if (animate && current.previous && current.image) return { ...current, nextIndex: next };
    if (current.previous?.index === next) return { index: next, image: current.previous.image, settled: true };
    const visible = current.image && !isLive(current.index)
      ? { index: current.index, image: current.image }
      : current.previous;
    const outgoing = animate && !isLive(next) ? visible : undefined;
    return { index: next, previous: outgoing, fadeIn: animate && !outgoing };
  }, [isLive]);
  const selectImage = useCallback((next: number) => {
    const animate = expanded && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (slideshow && animate && next !== (nextIndex ?? index) && (frame || live || isLive(next))) {
      const retained = frame ?? captureWorkFrame(stage.current);
      if (retained) {
        setSelection({ index: next, frame: retained });
        return;
      }
    }
    setSelection(current => transitionTo(current, next, animate));
  }, [expanded, slideshow, nextIndex, index, frame, live, isLive, transitionTo]);
  useEffect(() => {
    if (!slideshow || !open || images.length < 2 || !settled || previous || frame) return;
    const timer = window.setTimeout(() => selectImage((index + 1) % images.length), 3000);
    return () => window.clearTimeout(timer);
  }, [slideshow, open, images.length, settled, previous, frame, index, selectImage]);
  const finishFade = () => {
    setSelection(current => {
      if (current.index !== index || current.previous !== previous) return current;
      const next = current.nextIndex;
      if (next === undefined || next === index) return { index, image: current.image, settled: current.settled };
      return {
        index: next,
        image: previous?.index === next ? previous.image : undefined,
        settled: previous?.index === next,
        fadeIn: isLive(next) || !current.image,
        previous: current.image && !isLive(next) ? { index, image: current.image } : undefined,
      };
    });
  };
  useEffect(() => {
    const dot = activeDot.current;
    const strip = dot?.parentElement;
    if (!dot || !strip) return;
    const item = dot.getBoundingClientRect();
    const bounds = strip.getBoundingClientRect();
    if (item.left < bounds.left) strip.scrollLeft += item.left - bounds.left;
    if (item.right > bounds.right) strip.scrollLeft += item.right - bounds.right;
  }, [index]);
  useImperativeHandle(navigation, () => ({
    step(direction) {
      const next = (nextIndex ?? index) + direction;
      if (next >= 0 && next < images.length) {
        selectImage(next);
        return true;
      }
      // Let a queued boundary slide become active before leaving the album.
      return nextIndex !== undefined;
    },
  }));
  const stills = previous ? [previous, { index, image }] : [{ index, image }];
  return <div className={cn('album-gallery', expanded && 'is-expanded')}>
    <div ref={stage} className="album-stage flex items-center justify-center">
      {expanded && live ? <GalleryVideo key={index} task={task} index={index} open={open} fadeIn={slideshow && fadeIn && settled} onMediaReady={liveReady} /> : stills.map(still => {
        const outgoing = still.index !== index;
        return <div key={still.index} aria-hidden={outgoing || undefined}
          className={cn('album-still', outgoing && 'album-still-outgoing', outgoing && image && 'is-fading', !outgoing && slideshow && fadeIn && image && 'album-slide-fade-in')}
          onAnimationEnd={outgoing ? event => { if (event.target === event.currentTarget) finishFade(); } : undefined}>
          {still.image && !live && <img className="media-atmosphere" src={still.image.source} alt="" aria-hidden="true" />}
          <GalleryImage task={task} index={still.index} onReadyChange={outgoing ? undefined : imageReady} />
        </div>;
      })}
      {frame && <div className={cn('album-still album-still-outgoing', settled && 'is-fading')} aria-hidden="true"
        ref={node => { if (node && frame.parentElement !== node) node.append(frame); }}
        onAnimationEnd={event => {
          if (event.target === event.currentTarget) setSelection(current => current.frame === frame ? { ...current, frame: undefined } : current);
        }} />}
      {expanded && images.length > 1 && <div className={cn('album-dot-overlay', live && 'is-live')}>
        <div className="album-dots" role="group" aria-label="切换图集媒体">
          {images.map((_, i) => <Button key={i} ref={i === index ? activeDot : undefined} size="icon-sm" variant="ghost" className="album-dot" aria-label={`查看第 ${i + 1} 张图片`} aria-pressed={i === index} onClick={() => selectImage(i)}><span aria-hidden="true" /></Button>)}
        </div>
      </div>}
    </div>
    {!expanded && <div className="flex gap-1 overflow-x-auto pb-1" aria-label="图集图片列表">
      {images.map((_, i) => <Button key={i} size="icon-sm" variant="outline" className="album-thumbnail shrink-0 overflow-hidden p-0" aria-label={`查看第 ${i + 1} 张图片`} aria-pressed={i === index} onClick={() => selectImage(i)}><GalleryImage task={task} index={i} />{i === index && <CircleCheck className="album-selection" aria-hidden="true" />}</Button>)}
    </div>}
    <p className={expanded ? 'sr-only' : 'text-xs text-muted-foreground'} role="status">{index + 1} / {images.length}，{live && '实况，'}{selected?.quality_source === 'original' ? '原图' : '高清'}{selected?.width && selected.height ? `，${selected.width} × ${selected.height}` : ''}</p>
  </div>;
}
type CoverOperation = 'save' | 'copy';
type CoverOperations = {
  pending: ReadonlyMap<string, CoverOperation>;
  run: (id: string, operation: CoverOperation, work: () => Promise<void>) => Promise<void>;
};

function useCoverOperations(): CoverOperations {
  const active = useRef(new Map<string, CoverOperation>());
  const [pending, setPending] = useState<ReadonlyMap<string, CoverOperation>>(new Map());
  const run = useCallback(async (id: string, operation: CoverOperation, work: () => Promise<void>) => {
    if (active.current.has(id)) return;
    active.current.set(id, operation);
    setPending(new Map(active.current));
    try { await work(); }
    finally {
      active.current.delete(id);
      setPending(new Map(active.current));
    }
  }, []);
  return { pending, run };
}

function VideoCover({ task, mode = 'preview', feed = false, operations, onOpen, renderTask, actionTooltipSide }: {
  task: Task;
  mode?: 'preview' | 'thumbnail' | 'details' | 'actions';
  feed?: boolean;
  operations: CoverOperations;
  onOpen?: (trigger: HTMLElement) => void;
  renderTask?: (cover: React.ReactNode, actions: CoverMenuActions) => React.ReactNode;
  actionTooltipSide?: 'bottom';
}) {
  const small = mode === 'thumbnail';
  const inDetails = mode === 'details';
  const album = task.video?.kind === 'image_album';
  const [failed, setFailed] = useState(false);
  const { source: local, failed: readFailed } = useTaskImage(task, album ? 0 : undefined);
  const pending = operations.pending.get(task.id);
  const saving = pending === 'save';
  const copying = pending === 'copy';
  const [previewOpen, setPreviewOpen] = useState(false);
  const [actionImage, setActionImage] = useState<{ source: string; image: HTMLImageElement }>();
  const coverRef = useRef<HTMLButtonElement>(null);
  const previewImageRef = useRef<HTMLImageElement>(null);
  useEffect(() => { setFailed(false); }, [local, task.video?.cover_url]);
  const imageCount = albumImageCount(task);
  const coverCaption = album ? imageCount ? `${imageCount} 张图片` : null : task.video?.duration_ms != null ? duration(task.video.duration_ms) : null;
  const source = album || task.cover_path || task.cover_cached ? local : task.video?.cover_url;
  const available = !!source && !failed && !readFailed && (mode !== 'actions' || actionImage?.source === source);
  const coverDisabled = feed && task.status === 'downloading';
  useEffect(() => { if (!small && !available) setPreviewOpen(false); }, [available, small]);
  useEffect(() => {
    if (mode !== 'actions' || !source) return;
    let active = true;
    const image = new Image();
    image.referrerPolicy = 'no-referrer';
    image.src = source;
    void image.decode().then(() => {
      if (active) setActionImage({ source, image });
    }).catch(() => { if (active) setActionImage(undefined); });
    return () => { active = false; };
  }, [mode, source]);
  function saveCover() {
    return operations.run(task.id, 'save', async () => {
      try { if (await api.saveCover(task.id)) toast.success('封面已保存'); }
      catch (error) { toast.error(errorText(error)); }
    });
  }
  function copyCover(image = mode === 'actions' ? actionImage?.image : coverRef.current?.querySelector('img')) {
    return operations.run(task.id, 'copy', async () => {
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
    });
  }
  if (mode === 'actions') return available ? <DropdownMenu>
    <Tooltip><TooltipTrigger asChild><DropdownMenuTrigger asChild><Button size="icon-sm" variant="ghost" aria-label="封面操作"><MoreHorizontal aria-hidden="true" /></Button></DropdownMenuTrigger></TooltipTrigger><TooltipContent side={actionTooltipSide}>封面操作</TooltipContent></Tooltip>
    <DropdownMenuContent align="end" aria-label="封面操作">
      <DropdownMenuItem disabled={!!pending} onSelect={() => void saveCover()}><Download />{saving ? '保存中…' : '另存为…'}</DropdownMenuItem>
      <DropdownMenuItem disabled={!!pending} onSelect={() => void copyCover()}><Copy />{copying ? '复制中…' : '复制封面'}</DropdownMenuItem>
    </DropdownMenuContent>
  </DropdownMenu> : null;
  const thumbnail = <Button ref={coverRef} type="button" variant="ghost"
    data-media-cover={small ? "thumbnail" : "preview"} data-cover-context-menu={small ? '' : undefined} aria-label="查看封面" disabled={coverDisabled} aria-disabled={coverDisabled || !small && !available}
    onClick={event => {
      if (coverDisabled || !small && !available) event.preventDefault();
      else if (small) onOpen?.(event.currentTarget);
    }}
    className={cn('flex shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-muted p-0', small ? 'media-thumbnail' : 'media-stage')}>
    {!small && available && <img className="media-atmosphere" src={source} alt="" aria-hidden="true" />}{available ? <img className="media-cover-image" src={source} referrerPolicy="no-referrer" alt={album ? '第 1 张图片' : '作品封面'} onError={() => setFailed(true)} /> : <Film className={cn('text-muted-foreground', !small && 'm-8')} aria-hidden="true" />}
    {small && feed && coverCaption && <span className="task-feed-caption">{coverCaption}</span>}
  </Button>;
  if (small) return renderTask ? renderTask(thumbnail, { available, saving, copying, save: () => void saveCover(), copy: () => void copyCover() }) : thumbnail;
  const preview = available ? <ContextMenu><ContextMenuTrigger asChild>
        <img ref={previewImageRef} src={source} alt="封面大图" data-cover-context-menu="" tabIndex={0} onError={() => setFailed(true)}
          className={cn('object-contain outline-none focus-visible:ring-2 focus-visible:ring-ring', inDetails ? 'max-h-full max-w-full h-auto w-auto' : 'mx-auto h-auto max-h-[calc(90dvh-128px)] w-auto max-w-[calc(90vw-42px)]')} />
      </ContextMenuTrigger><ContextMenuContent aria-label="封面预览操作"><ContextMenuGroup>
        <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void saveCover()}><Download />{saving ? '保存中…' : '另存为…'}</ContextMenuItem>
        <ContextMenuItem disabled={saving || copying || !available} onSelect={() => void copyCover(previewImageRef.current)}><Copy />{copying ? '复制中…' : '复制封面'}</ContextMenuItem>
      </ContextMenuGroup></ContextMenuContent></ContextMenu> : <Film className="size-10 text-muted-foreground" aria-label="封面不可用" />;
  if (inDetails) return <div className="task-cover-image">{available && <img className="media-atmosphere" src={source} alt="" aria-hidden="true" />}{preview}</div>;
  return <Dialog open={previewOpen} onOpenChange={setPreviewOpen}>
    <DialogTrigger asChild>{thumbnail}</DialogTrigger>
    <DialogContent className="cover-preview-dialog max-h-[90vh] w-fit min-w-56 max-w-[90vw] overflow-y-auto sm:max-w-[90vw]">
      <DialogHeader className="w-0 min-w-full pr-8"><DialogTitle>封面预览</DialogTitle><DialogDescription data-slot="media-description" className="line-clamp-2 text-sm">{task.video?.title || '作品封面'}</DialogDescription></DialogHeader>
      {preview}
    </DialogContent>
  </Dialog>;
}

type TaskMediaEntry = {
  id: string;
  returnFocus: HTMLElement | null;
  streamStep?: number;
} & ({ kind: 'task'; playVideo?: true; asset?: PlaybackSubject; startAtLast?: boolean }
  | { kind: 'asset'; asset: PlaybackSubject });

function playbackKey(entry: TaskMediaEntry) {
  return `${entry.kind}:${entry.id}:${entry.streamStep ?? 'details'}`;
}

const wheelIdleMs = 420;
const workSwitchPauseMs = 700;
const wheelPixelThreshold = 80;
const streamWheelPixelThreshold = 48;
const workSlideDurationMs = 420;
const workStageSelector = '.album-stage, .video-player-stage:not(.stream-video-slot[hidden] *)';
const workImageSelector = '.album-still:not([aria-hidden]) img:not(.media-atmosphere)';
const streamWorkSelector = '.stream-playback-work:not(.stream-playback-outgoing)';

type WorkSwitch = {
  next: TaskMediaEntry;
  direction: MediaDirection;
  page?: { element: HTMLDivElement; detailsScrollTop: number; coveredSidebar?: HTMLElement; coveredControls?: HTMLElement };
  animations?: Animation[];
  finishTimer?: number;
};

function captureWorkFrame(stage: Element | null): HTMLCanvasElement | undefined {
  const video = stage?.querySelector('video');
  const currentImage = stage?.querySelector<HTMLImageElement>(workImageSelector);
  const image = currentImage?.naturalWidth ? currentImage
    : stage?.querySelector<HTMLImageElement>('.album-still-outgoing img:not(.media-atmosphere)');
  const media = video && video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA ? video
    : image?.naturalWidth ? image : stage?.querySelector<HTMLCanvasElement>('canvas.media-work-frame');
  if (!media) return;
  const width = media instanceof HTMLVideoElement ? media.videoWidth : media instanceof HTMLImageElement ? media.naturalWidth : media.width;
  const height = media instanceof HTMLVideoElement ? media.videoHeight : media instanceof HTMLImageElement ? media.naturalHeight : media.height;
  if (!width || !height) return;
  const frame = document.createElement('canvas');
  const scale = Math.min(1, 1280 / Math.max(width, height));
  frame.width = Math.max(1, Math.round(width * scale));
  frame.height = Math.max(1, Math.round(height * scale));
  const context = frame.getContext('2d');
  if (!context) return;
  try {
    context.drawImage(media, 0, 0, frame.width, frame.height);
  } catch {
    return;
  }
  frame.className = 'media-work-frame';
  frame.style.objectFit = getComputedStyle(media).objectFit;
  frame.setAttribute('aria-hidden', 'true');
  return frame;
}

function clearWorkFrame(switching: WorkSwitch) {
  window.clearTimeout(switching.finishTimer);
  switching.animations?.forEach(animation => animation.cancel());
  if (switching.page?.coveredSidebar) switching.page.coveredSidebar.inert = false;
  if (switching.page?.coveredControls) switching.page.coveredControls.inert = false;
  switching.page?.element.remove();
}

function captureStreamWork(dialog: Element, frame?: HTMLCanvasElement): WorkSwitch['page'] {
  const sidebar = dialog.querySelector(`${streamWorkSelector} > .task-media-sidebar`);
  if (!sidebar) return;
  const element = document.createElement('div');
  element.className = 'stream-playback-work stream-playback-outgoing';
  element.inert = true;
  element.setAttribute('aria-hidden', 'true');
  const media = document.createElement('div');
  media.className = 'task-media-panel';
  if (frame) media.append(frame);
  const details = sidebar.cloneNode(true) as HTMLElement;
  details.querySelectorAll('[id]').forEach(node => node.removeAttribute('id'));
  element.append(media, details);
  return { element, detailsScrollTop: sidebar.querySelector('.task-details-content')?.scrollTop ?? 0 };
}

function TaskMediaDialog({ task, entry, playable, preload, coverOperations, removedIds, adjacent, navigate, close, fallbackFocus }: {
  task?: Task;
  entry: TaskMediaEntry | null;
  playable: boolean;
  preload?: { entry: TaskMediaEntry; subject: PlaybackSubject };
  coverOperations: CoverOperations;
  removedIds: ReadonlySet<string>;
  adjacent: (direction: MediaDirection) => TaskMediaEntry | undefined;
  navigate: (previous: TaskMediaEntry, next: TaskMediaEntry) => void;
  close: () => void;
  fallbackFocus: () => HTMLElement | null;
}) {
  const last = useRef<{ task?: Task; entry: TaskMediaEntry; playable: boolean } | null>(null);
  const [dialog, setDialog] = useState<HTMLDivElement | null>(null);
  const [streamMaximized, setStreamMaximized] = useState(false);
  const streamCoverControls = useRef<HTMLSpanElement>(null);
  const details = useRef<HTMLDivElement>(null);
  const albumNavigation = useRef<AlbumNavigation>(null);
  const wheelGesture = useRef({ lastAt: -Infinity, lockedUntil: -Infinity, delta: 0, consumed: false });
  const workSwitch = useRef<WorkSwitch | null>(null);
  const focusedEntry = useRef<TaskMediaEntry | null>(null);
  const [measured, setMeasured] = useState<{ entry: TaskMediaEntry; size: MediaSize }>();
  const open = !!entry && (entry.kind === 'asset' || !!task);
  if (open) last.current = { task, entry, playable };
  const current = last.current;
  const currentTask = current?.task;
  const streaming = current?.entry.streamStep !== undefined;
  const albumImages = currentTask && albumMedia(currentTask);
  const album = !!albumImages?.length;
  const standalone = current?.entry.kind === 'asset';
  const video = standalone ? current.entry.asset : current?.playable ? current.entry.asset ?? currentTask : undefined;
  const mediaEntry = video || album ? current?.entry : undefined;
  const taskId = currentTask?.id;
  const onMediaReady = useCallback((ready: boolean) => {
    const switching = workSwitch.current;
    if (!switching || switching.next !== entry || switching.animations) return;
    const finish = () => {
      if (workSwitch.current !== switching) return;
      workSwitch.current = null;
      clearWorkFrame(switching);
      wheelGesture.current.lockedUntil = performance.now() + workSwitchPauseMs;
      if (switching.page && document.activeElement === dialog) details.current?.focus({ preventScroll: true });
    };
    const outgoing = switching.page?.element;
    const incoming = dialog?.querySelector(streamWorkSelector);
    if (!ready || !outgoing?.isConnected || !incoming || matchMedia('(prefers-reduced-motion: reduce)').matches) {
      finish();
      return;
    }
    const offset = switching.direction * 100;
    const timing = { duration: workSlideDurationMs, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'both' } as const;
    const leaving = outgoing.animate([{ transform: 'translateY(0)' }, { transform: `translateY(${-offset}%)` }], timing);
    const entering = incoming.animate([{ transform: `translateY(${offset}%)` }, { transform: 'translateY(0)' }], timing);
    switching.animations = [leaving, entering];
    entering.onfinish = finish;
    entering.oncancel = finish;
    switching.finishTimer = window.setTimeout(finish, workSlideDurationMs + 480);
  }, [dialog, entry]);
  useLayoutEffect(() => {
    const previous = focusedEntry.current;
    focusedEntry.current = open ? entry : null;
    if (open && !previous) setStreamMaximized(false);
    const switching = workSwitch.current;
    if (switching) {
      if (!open || switching.next !== mediaEntry) {
        workSwitch.current = null;
        clearWorkFrame(switching);
      } else if (switching.page && dialog && !switching.page.element.isConnected) {
        const sidebar = dialog.querySelector<HTMLElement>(`${streamWorkSelector} > .task-media-sidebar`);
        if (sidebar) {
          sidebar.inert = true;
          switching.page.coveredSidebar = sidebar;
        }
        if (streamCoverControls.current) {
          streamCoverControls.current.inert = true;
          switching.page.coveredControls = streamCoverControls.current;
        }
        dialog.append(switching.page.element);
        const oldDetails = switching.page.element.querySelector('.task-details-content');
        if (oldDetails) oldDetails.scrollTop = switching.page.detailsScrollTop;
      }
    }
    if (open && previous && previous !== entry) {
      const focus = workSwitch.current?.page?.coveredSidebar ? dialog : details.current ?? dialog;
      focus?.focus({ preventScroll: true });
    }
  }, [dialog, entry, mediaEntry, open]);
  useEffect(() => () => {
    const switching = workSwitch.current;
    workSwitch.current = null;
    if (switching) clearWorkFrame(switching);
  }, []);
  useEffect(() => {
    const gesture = wheelGesture.current;
    if (!open) Object.assign(gesture, { lastAt: -Infinity, lockedUntil: -Infinity, delta: 0, consumed: false });
    if (!open || !mediaEntry || !dialog || (!streaming && !album)) return;
    const wheel = (event: WheelEvent) => {
      const mode = event.deltaMode;
      if (!(event.target instanceof Element) || !event.target.closest('.task-media-panel, .video-player-stage')
        || event.ctrlKey || event.shiftKey || Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      event.preventDefault();
      if (streaming && mode === WheelEvent.DOM_DELTA_PIXEL && Math.abs(event.deltaY) < 4) return;
      if (event.timeStamp - gesture.lastAt >= wheelIdleMs) {
        gesture.delta = 0;
        gesture.consumed = false;
      }
      gesture.lastAt = event.timeStamp;
      if (event.timeStamp < gesture.lockedUntil || workSwitch.current) gesture.consumed = true;
      if (gesture.consumed) return;
      const direction: MediaDirection = event.deltaY > 0 ? 1 : -1;
      if (mode === WheelEvent.DOM_DELTA_PIXEL) {
        if (gesture.delta * direction < 0) gesture.delta = 0;
        gesture.delta += event.deltaY;
        if (Math.abs(gesture.delta) < (streaming ? streamWheelPixelThreshold : wheelPixelThreshold)) return;
      }
      gesture.consumed = true;
      if (album && (albumNavigation.current?.step(direction) ?? true)) return;
      if (!streaming) return;
      const next = adjacent(direction);
      if (!next) return;
      const frame = captureWorkFrame(dialog.querySelector(workStageSelector));
      workSwitch.current = { next, direction, page: captureStreamWork(dialog, frame) };
      navigate(mediaEntry, next);
    };
    dialog.addEventListener('wheel', wheel, { passive: false });
    return () => dialog.removeEventListener('wheel', wheel);
  }, [dialog, open, mediaEntry, album, adjacent, navigate, streaming]);
  const albumBounds = useMemo(() => {
    if (streaming || !albumImages) return;
    const size = { width: 0, height: 0 };
    const missing: number[] = [];
    albumImages.forEach(({ width, height }, index) => {
      if (width && height) {
        size.width = Math.max(size.width, width);
        size.height = Math.max(size.height, height);
      } else {
        missing.push(index);
      }
    });
    return { size, missing };
  }, [albumImages, streaming]);
  const waitingForAlbumSize = !!(albumBounds?.missing.length && measured?.entry !== mediaEntry);
  useEffect(() => {
    if (!open || !waitingForAlbumSize || !mediaEntry || !taskId || !albumBounds) return;
    let active = true;
    void (async () => {
      const size = { ...albumBounds.size };
      // Older records may omit dimensions. Read those images once before showing the album.
      for (const index of albumBounds.missing) {
        if (!active) return;
        try {
          const image = await readTaskImageSize(taskId, index);
          size.width = Math.max(size.width, image.width);
          size.height = Math.max(size.height, image.height);
        } catch {
          // An unreadable slide keeps the normal image error state.
        }
      }
      if (active) setMeasured({ entry: mediaEntry, size });
    })();
    return () => { active = false; };
  }, [open, waitingForAlbumSize, mediaEntry, taskId, albumBounds]);
  const mediaSize = measured?.entry === mediaEntry ? measured?.size : albumBounds?.size;
  const onMediaSize = useCallback((size: MediaSize) => {
    if (!streaming && mediaEntry && size.width > 0 && size.height > 0) {
      setMeasured({ entry: mediaEntry, size });
    }
  }, [mediaEntry, streaming]);
  const mediaStyle = mediaSize?.width && mediaSize.height ? {
    '--task-media-intrinsic-width': `${mediaSize.width}px`,
    '--task-media-aspect': mediaSize.width / mediaSize.height,
  } as React.CSSProperties : undefined;
  const size = mediaSize ?? video?.video;
  const ratio = size?.width && size.height ? size.width / size.height : 0;
  const playerStyle = { '--video-player-ratio': Number.isFinite(ratio) && ratio > 0 ? ratio : 16 / 9 } as React.CSSProperties;
  const albumGallery = currentTask && current?.entry.kind === 'task' && album && (waitingForAlbumSize
    ? <Spinner className="m-auto" aria-label="正在加载图集" />
    : <AlbumGallery key={currentTask.id} task={currentTask} expanded open={open} slideshow startAtLast={current.entry.startAtLast} navigation={albumNavigation} onMediaReady={streaming ? onMediaReady : undefined} />);
  const taskDetails = currentTask && <TaskDetails key={currentTask.id} task={currentTask} regionRef={details}
    coverTools={!streaming && (video || album) && <VideoCover task={currentTask} mode="actions" operations={coverOperations} />} />;
  const renderLayout = (media: React.ReactNode, tools?: React.ReactNode) => <>
    <div className="task-media-panel">{media}</div>
    {currentTask && <div className="task-media-sidebar">
      {taskDetails}
      {tools}
    </div>}
  </>;
  const streamVideos = [
    ...(video && mediaEntry ? [{ entry: mediaEntry, subject: video, preloading: false }] : []),
    ...(open && preload ? [{ ...preload, preloading: true }] : []),
  ];
  const streamSizeAction = streamMaximized ? '还原大小' : '最大化';

  return <Dialog open={open} onOpenChange={next => { if (!next) close(); }}>
    <DialogContent ref={setDialog} showCloseButton={!streaming} className={cn(streaming ? 'stream-playback-dialog' : !standalone && 'task-media-dialog', streaming && streamMaximized && 'is-maximized', album && 'album-dialog', !streaming && video && 'video-player-dialog', !streaming && !standalone && mediaStyle && 'has-media-size')} style={streaming ? undefined : standalone ? playerStyle : mediaStyle} onOpenAutoFocus={event => {
      if (!details.current) return;
      event.preventDefault();
      details.current.focus({ preventScroll: true });
    }} onCloseAutoFocus={event => {
      event.preventDefault();
      const target = last.current?.entry.returnFocus;
      const available = target?.isConnected && !target.matches(':disabled') && !target.closest('[inert], [hidden]');
      (available ? target : fallbackFocus())?.focus({ preventScroll: true });
    }}>
      {streaming ? <><DialogTitle className="sr-only">{currentTask ? taskTitle(currentTask) : video?.video?.title?.trim() || '流式播放'}</DialogTitle><DialogDescription className="sr-only">流式播放</DialogDescription></>
        : standalone ? <DialogHeader className="pr-8"><DialogTitle className="line-clamp-2">{video?.video?.title?.trim() || '视频播放'}</DialogTitle><DialogDescription className="sr-only">本地视频播放器</DialogDescription></DialogHeader>
        : <><DialogTitle className="sr-only">任务详情</DialogTitle><DialogDescription className="sr-only">作品媒体与任务信息</DialogDescription></>}
      {streaming && <div className="stream-playback-controls">
        <span ref={streamCoverControls} className="inline-flex empty:hidden">
          {currentTask && (video || album) && <VideoCover key={currentTask.id} task={currentTask} mode="actions" operations={coverOperations} actionTooltipSide="bottom" />}
        </span>
        <Tooltip><TooltipTrigger asChild>
          <Button size="icon-sm" variant="ghost" aria-label={streamSizeAction} onClick={() => setStreamMaximized(value => !value)}>
            {streamMaximized ? <Minimize2 aria-hidden="true" /> : <Maximize2 aria-hidden="true" />}
          </Button>
        </TooltipTrigger><TooltipContent side="bottom">{streamSizeAction}</TooltipContent></Tooltip>
        <Tooltip><TooltipTrigger asChild><DialogClose asChild>
          <Button size="icon-sm" variant="ghost" aria-label="关闭"><X aria-hidden="true" /></Button>
        </DialogClose></TooltipTrigger><TooltipContent side="bottom">关闭</TooltipContent></Tooltip>
      </div>}
      {streaming ? <div className="stream-playback-work">
        <div className="task-media-panel">
          {streamVideos.map(slot => <div key={playbackKey(slot.entry)} className="stream-video-slot" hidden={slot.preloading}>
            <PlaybackSession task={slot.subject} open={open} close={close} removedIds={removedIds} loop preloading={slot.preloading}
              onMediaReady={slot.preloading ? undefined : onMediaReady} renderLayout={media => media} />
          </div>)}
          {albumGallery}
        </div>
        <div className="task-media-sidebar">
          {taskDetails ?? <div ref={details} className="task-details-content" role="region" aria-label="任务详细信息" tabIndex={0}>
            <h2 className="task-details-title text-sm font-semibold">{video?.video?.title?.trim() || '视频播放'}</h2>
            {video?.video?.width && video.video.height && <dl className="task-details-fields"><dt>分辨率</dt><dd>{video.video.width} × {video.video.height}</dd></dl>}
          </div>}
        </div>
      </div> : video && mediaEntry ? <PlaybackSession key={playbackKey(mediaEntry)} task={video} open={open} close={close} removedIds={removedIds} onMediaSize={onMediaSize} renderLayout={standalone ? undefined : renderLayout} />
        : currentTask && renderLayout(albumGallery || <VideoCover key={currentTask.id} task={currentTask} mode="details" operations={coverOperations} />)}
    </DialogContent>
  </Dialog>;
}

function downloadPercent(task: Task) {
  const total = task.total_bytes ?? task.total_bytes_estimate;
  return total ? Math.max(0, Math.min(99, Math.floor(task.bytes / total * 100))) : null;
}
function DownloadProgress({ task, hideIndicator = false }: { task: Task; hideIndicator?: boolean }) {
  if (task.merging) return <div className={cn("download-progress flex flex-wrap items-center gap-4 text-xs tabular-nums text-muted-foreground", hideIndicator && "task-feed-telemetry")} role="status"><span className="flex items-center gap-2"><Spinner aria-hidden="true" />正在合并音视频…</span><span>已下载 {bytes(task.bytes)}</span></div>;
  const total = task.total_bytes ?? task.total_bytes_estimate;
  const percent = downloadPercent(task);
  if (hideIndicator) {
    const transferred = `${bytes(task.bytes)} / ${task.total_bytes == null && task.total_bytes_estimate != null ? '约 ' : ''}${bytes(total ?? null)}`;
    const speed = task.speed_bps == null ? '速度 —' : bytes(task.speed_bps) + '/s';
    const eta = task.eta_seconds == null ? '剩余时间 —' : `预计剩余 ${remainingTime(task.eta_seconds)}`;
    return <div className="download-progress task-feed-telemetry text-xs tabular-nums text-muted-foreground"><TaskMetadataItem label={transferred} className="truncate">{transferred}</TaskMetadataItem>{task.total_items != null && <TaskMetadataItem label="图片数量" className="shrink-0">{task.completed_items ?? 0} / {task.total_items} 张</TaskMetadataItem>}<TaskMetadataItem label={speed} className="shrink-0">{speed}</TaskMetadataItem><TaskMetadataItem label={eta} className="ml-auto truncate">{eta}</TaskMetadataItem></div>;
  }
  return <div className="download-progress flex flex-col gap-2">{!hideIndicator && <Progress aria-label="下载进度" value={percent} updateIntervalMs={1000} />}<div className="flex flex-wrap items-center gap-4 text-xs tabular-nums text-muted-foreground">
    {percent == null ? <span className="flex items-center gap-2"><Spinner />正在下载</span> : !hideIndicator && <span>{percent}%</span>}{task.total_items != null && <span>{task.completed_items ?? 0} / {task.total_items} 张</span>}<span>{bytes(task.bytes)} / {task.total_bytes == null && task.total_bytes_estimate != null ? '约 ' : ''}{bytes(total ?? null)}</span><span>{task.speed_bps == null ? '速度 —' : bytes(task.speed_bps) + '/s'}</span>{task.eta_seconds != null && <span>预计剩余 {remainingTime(task.eta_seconds)}</span>}
  </div></div>;
}
type SettingsSection = 'download' | 'accounts' | 'dependencies' | 'backup';
const settingsSections: { id: SettingsSection; label: string; description: string; Icon: typeof Download }[] = [
  { id: 'download', label: '下载设置', description: '选择文件保存位置与同时下载数量。', Icon: Download },
  { id: 'accounts', label: '平台账号', description: '按平台管理账号认证、Cookie、网络代理与 User-Agent。', Icon: KeyRound },
  { id: 'backup', label: '远端备份', description: '备份视频与文案，视频按需下载。', Icon: HardDrive },
  { id: 'dependencies', label: '运行依赖', description: '检查本机运行环境，补齐解析与音视频合并需要的工具。', Icon: PackageCheck },
];
function ProfileForm({ profile, saved }: { profile: Profile; saved: (settings: Settings) => void }) {
  const anonymousFirst = profile.platform === 'xiaohongshu';
  const [cookie, setCookie] = useState('');
  const [savedCookie, setSavedCookie] = useState('');
  const [cookieLoaded, setCookieLoaded] = useState(false);
  const [useBrowserCookies, setUseBrowserCookies] = useState(!profile.cookie_configured);
  const [proxy, setProxy] = useState('');
  const [savedProxy, setSavedProxy] = useState('');
  const [proxyLoaded, setProxyLoaded] = useState(false);
  const [proxyError, setProxyError] = useState('');
  const [direct, setDirect] = useState(!profile.proxy);
  const [bypassProxy, setBypassProxy] = useState(!!profile.bypass_proxy);
  const [ua, setUa] = useState(profile.user_agent);
  const [chunkMode, setChunkMode] = useState(profile.http_chunk_size == null ? 'inherit' : profile.http_chunk_size === 0 ? 'disabled' : 'custom');
  const [chunkSize, setChunkSize] = useState(String(profile.http_chunk_size ? profile.http_chunk_size / 1048576 : 10));
  const [show, setShow] = useState(true);
  const [busy, setBusy] = useState(false);
  const [cookieError, setCookieError] = useState('');
  const proxyChanged = !bypassProxy && proxyLoaded && (direct !== (savedProxy === '') || (!direct && proxy !== savedProxy));
  const chunkInvalid = profile.platform === 'youtube' && chunkMode === 'custom' &&
    (!Number.isInteger(Number(chunkSize)) || Number(chunkSize) < 1 || Number(chunkSize) > 1024);
  const chunkBytes = chunkMode === 'inherit' ? null : chunkMode === 'disabled' ? 0 : Number(chunkSize) * 1048576;
  const chunkChanged = profile.platform === 'youtube' && chunkBytes !== (profile.http_chunk_size ?? null);
  const changed = (cookieLoaded && (useBrowserCookies ? '' : cookie) !== savedCookie) || proxyChanged || bypassProxy !== !!profile.bypass_proxy || ua !== profile.user_agent || chunkChanged;
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
    setBusy(true);
    try {
      if (proxyChanged && !direct && !proxy.trim()) throw 'CONFIG_INVALID';
      const nextCookie = useBrowserCookies ? '' : cookie.trim();
      saved(await api.save({ platform: profile.platform,
        bypass_proxy: bypassProxy,
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
      toast.success('平台配置已保存，仅影响新任务', { description: platformName(profile.platform) });
    } catch (error) { toast.error(errorText(error), { description: platformName(profile.platform), duration: 8000 }); }
    finally { setBusy(false); }
  }
  return <Card size="sm" className="flex min-w-0 flex-col [--card-spacing:12px]"><CardHeader><CardTitle className="font-semibold">{platformName(profile.platform)}</CardTitle><CardDescription>此平台的凭据、代理和浏览器标识独立保存</CardDescription><CardAction><Badge variant="secondary">{profile.cookie_configured ? '手动 Cookie 已配置' : anonymousFirst ? '默认匿名' : '使用默认浏览器'}</Badge></CardAction></CardHeader>
    <CardContent className="flex-1"><form id={'profile-' + profile.platform} onSubmit={save} aria-busy={busy}><FieldSet disabled={busy} className="min-w-0"><FieldGroup className="gap-3">
      <Field data-disabled={useBrowserCookies}><FieldLabel htmlFor={profile.platform + '-cookie'}>网页端 Cookie</FieldLabel><InputGroup><InputGroupTextarea id={profile.platform + '-cookie'} autoComplete="off" rows={4} className={cn('field-sizing-fixed h-24 min-h-24 max-h-48 flex-none resize-y break-all', !show && 'cookie-masked')} value={cookie} disabled={useBrowserCookies || !cookieLoaded} onChange={e => setCookie(e.target.value)} placeholder={cookieLoaded ? '粘贴浏览器中的 Cookie' : cookieError ? 'Cookie 读取失败' : '正在读取 Cookie…'} /><InputGroupAddon align="block-end" className="justify-end"><InputGroupButton className="size-7" aria-label={show ? '隐藏 Cookie' : '显示 Cookie'} onClick={() => setShow(!show)}>{show ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup><FieldDescription>{anonymousFirst ? '留空时先匿名访问，遇到访问限制时读取系统默认浏览器的 Cookie。' : '手动 Cookie 保存在本机数据库；留空时尝试读取系统默认浏览器，不回写自动读取的 Cookie。'}</FieldDescription></Field>
      <Field orientation="horizontal"><Checkbox id={profile.platform + '-clear'} disabled={!cookieLoaded} checked={useBrowserCookies} onCheckedChange={value => setUseBrowserCookies(value === true)} /><FieldLabel htmlFor={profile.platform + '-clear'}>{anonymousFirst ? '按需使用系统默认浏览器 Cookie（保存后将清除手动 Cookie）' : '使用系统默认浏览器 Cookie（保存后生效，将清除手动 Cookie）'}</FieldLabel></Field>
      {cookieError && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{cookieError}；保存其他设置将保留已有 Cookie。</AlertDescription></Alert>}
      <Separator />
      <Field orientation="horizontal"><Checkbox id={profile.platform + '-bypass-proxy'} checked={bypassProxy} onCheckedChange={value => setBypassProxy(value === true)} /><FieldContent><FieldLabel htmlFor={profile.platform + '-bypass-proxy'}>解析与下载不走代理</FieldLabel><FieldDescription>开启 TUN／透明代理时启用，保留已保存的代理地址。</FieldDescription></FieldContent></Field>
      <Field><FieldLabel id={profile.platform + '-mode'}>网络代理</FieldLabel><ToggleGroup size="sm" type="single" variant="outline" aria-labelledby={profile.platform + '-mode'} value={direct ? 'direct' : 'proxy'} disabled={!proxyLoaded || bypassProxy} onValueChange={value => { if (value) setDirect(value === 'direct'); }}><ToggleGroupItem value="direct">直连</ToggleGroupItem><ToggleGroupItem value="proxy">自定义代理</ToggleGroupItem></ToggleGroup></Field>
      {!direct && <Field><FieldLabel htmlFor={profile.platform + '-proxy'}>HTTP / SOCKS5 地址</FieldLabel><Input id={profile.platform + '-proxy'} className="min-h-8" type="text" autoComplete="off" spellCheck={false} value={proxy} disabled={!proxyLoaded || bypassProxy} onChange={e => setProxy(e.target.value)} placeholder={proxyLoaded ? 'socks5://127.0.0.1:7890' : proxyError ? '代理读取失败' : '正在读取代理…'} /><FieldDescription>请输入完整地址，例如 http://127.0.0.1:7890 或 socks5://127.0.0.1:7890；含密码的代理保存在系统凭据存储中。</FieldDescription></Field>}
      {proxyError && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{proxyError}；保存其他设置将保留已有代理。</AlertDescription></Alert>}
      <Separator />
      {profile.platform === 'youtube' && <>
        <Field><FieldLabel id="youtube-chunk-mode">HTTP 分块下载</FieldLabel><ToggleGroup size="sm" type="single" variant="outline" aria-labelledby="youtube-chunk-mode" value={chunkMode} disabled={busy} onValueChange={value => { if (value) setChunkMode(value); }}><ToggleGroupItem value="inherit">继承上游默认</ToggleGroupItem><ToggleGroupItem value="disabled">关闭分块</ToggleGroupItem><ToggleGroupItem value="custom">自定义</ToggleGroupItem></ToggleGroup><FieldDescription>{chunkMode === 'inherit' ? '沿用 YouTube 解析引擎的默认分块策略，不等同于关闭分块。' : '仅影响新任务的 HTTP 媒体下载，不改变视频规格和音轨；速度取决于网络与 CDN。'}</FieldDescription></Field>
        {chunkMode === 'custom' && <Field data-invalid={chunkInvalid || undefined}><FieldLabel htmlFor="youtube-chunk-size">自定义分块大小（MiB）</FieldLabel><Input id="youtube-chunk-size" type="number" inputMode="numeric" min={1} max={1024} step={1} value={chunkSize} onChange={event => setChunkSize(event.target.value)} aria-invalid={chunkInvalid} aria-describedby="youtube-chunk-size-help" className="max-w-48" /><FieldDescription id="youtube-chunk-size-help">1–1024 MiB 的整数，10 MiB 可作为起始测试值。</FieldDescription>{chunkInvalid && <FieldError>请输入 1–1024 之间的整数。</FieldError>}</Field>}
        <Separator />
      </>}
      <Field><FieldLabel htmlFor={profile.platform + '-ua'}>User-Agent</FieldLabel><Textarea id={profile.platform + '-ua'} rows={3} value={ua} onChange={e => setUa(e.target.value)} /><FieldDescription>使用与 Cookie 来源一致的浏览器标识。</FieldDescription></Field>
    </FieldGroup></FieldSet></form></CardContent><CardFooter className="justify-end py-2"><Button type="submit" form={'profile-' + profile.platform} disabled={busy || !changed || chunkInvalid}>{busy && <Spinner aria-hidden="true" data-icon="inline-start" />}{busy ? '保存中…' : '保存平台配置'}</Button></CardFooter>
  </Card>;
}
function App() {
  useEffect(() => {
    const preventContextMenu = (event: MouseEvent) => {
      if (!(event.target instanceof Element && event.target.closest('#video-input, [data-cover-context-menu], .task-context-target'))) event.preventDefault();
    };
    document.addEventListener('contextmenu', preventContextMenu, true);
    return () => document.removeEventListener('contextmenu', preventContextMenu, true);
  }, []);
  const [page, setPage] = useState('download');
  const [resolveSubmitting, setResolveSubmitting] = useState(false);
  const [resolveRequestError, setResolveRequestError] = useState('');
  const [resolveSource, setResolveSource] = useState('');
  const pageContentRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => { pageContentRef.current?.scrollTo(0, 0); }, [page]);
  const backup = useBackupTasks();
  const backupSettings = backup.settings;
  const [settingsSection, setSettingsSection] = useState<SettingsSection>('download');
  const settingsActive = page === 'settings';
  const backupSettingsActive = settingsActive && settingsSection === 'backup';
  const [settingsVisited, setSettingsVisited] = useState(false);
  const [backupSettingsVisited, setBackupSettingsVisited] = useState(false);
  useEffect(() => { if (settingsActive) setSettingsVisited(true); }, [settingsActive]);
  useEffect(() => { if (backupSettingsActive) setBackupSettingsVisited(true); }, [backupSettingsActive]);
  const runtimeDependencies = useRuntimeDependencies();
  const videoTranscripts = useVideoTranscripts();
  const [settingsPlatform, setSettingsPlatform] = useState('douyin');
  const settingsSectionInfo = settingsSections.find(section => section.id === settingsSection);
  const [tasks, setTasks] = useState<Task[]>([]);
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const [exitingTasks, setExitingTasks] = useState<Task[]>([]);
  const taskOrder = useRef(new Map<string, number>());
  const taskListRef = useRef<HTMLDivElement>(null);
  const finishRemoval = useCallback((id: string) => setExitingTasks(old => old.filter(task => task.id !== id)), []);
  const [settings, setSettings] = useState<Settings | null>(null);
  const filenamePreviewCache = useRef<FilenamePreviewCache | null>(null);
  const [input, setInput] = useState('');
  const [downloadMode, setDownloadMode] = useState<'post' | 'author'>('post');
  const [pasting, setPasting] = useState(false);
  const [resolver, setResolver] = useState<Resolver>('auto');
  const [selected, setSelected] = useState<string | null>(null);
  const [previewReset, setPreviewReset] = useState(false);
  // Hide previews after acknowledged handoff/cancellation, even before the
  // corresponding task event arrives. Rust still owns their task state.
  const [dismissedPreviewIds, setDismissedPreviewIds] = useState(() => new Set<string>());
  const [busy, setBusy] = useState(false);
  const pendingTaskActions = useRef(new Set<string>());
  const [waitingTaskActions, setWaitingTaskActions] = useState(() => new Set<string>());
  const [dismissedTaskErrorRevisions, setDismissedTaskErrorRevisions] = useState(() => new Map<string, number>());
  const [connected, setConnected] = useState(false);
  const [confirm, setConfirm] = useState<{task: Task; resetPreview: boolean; input: string} | null>(null);
  const [duplicate, setDuplicate] = useState<{task: Task; previous: Task; background?: boolean} | null>(null);
  const duplicateCompleted = duplicate?.previous.status === 'completed';
  const duplicateCanReplace = duplicateCompleted && !!duplicate?.task.video && !!duplicate.previous.video
    && sameReplacementVariant(duplicate.previous.video, duplicate.task.video);
  const duplicateTrigger = useRef<{id: string; background: boolean} | null>(null);
  useEffect(() => { if (duplicate) duplicateTrigger.current = {id: duplicate.task.id, background: !!duplicate.background}; }, [duplicate]);
  const promptedDuplicates = useRef(new Set<string>());
  const [search, setSearch] = useState('');
  const [authorQuery, setAuthorQuery] = useState<AuthorSearch | null>(null);
  const taskListPage = page === 'tasks' || page === 'search';
  const [taskPlatform, setTaskPlatform] = useState('all');
  const [taskView, setTaskView] = useState<TaskView>(() => localStorage.getItem(taskViewStorageKey) === 'feed' ? 'feed' : 'list');
  const coverOperations = useCoverOperations();
  const [mediaEntry, setMediaEntry] = useState<TaskMediaEntry | null>(null);
  const viewedTask = mediaEntry?.kind === 'task' ? tasks.find(task => task.id === mediaEntry.id) : undefined;
  const viewPlaysVideo = mediaEntry?.kind === 'task' && !!mediaEntry.playVideo && !!viewedTask && !waitingTaskActions.has(viewedTask.id) && canPlayTask(viewedTask);
  const [taskStatus, setTaskStatus] = useState('all');
  const [selectedRow, setSelectedRow] = useState<string | null>(null);
  const [clearHistoryPlatform, setClearHistoryPlatform] = useState<string | null>(null);
  const clearHistoryTrigger = useRef<HTMLButtonElement>(null);
  const taskSearchRef = useRef<HTMLInputElement>(null);
  const [wide, setWide] = useState(window.innerWidth > 1100);
  const lastStatuses = useRef<Map<string, Pick<Task, 'status' | 'error' | 'download_requested_at'>> | null>(null);
  const submittedInput = useRef<{taskId: string; value: string; afterRevision: number; replacedRequestAt?: number | null} | null>(null);
  const removedIds = useRef(new Set<string>());
  const removedAssetIds = useRef(new Set<string>());
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const cancelTrigger = useRef<HTMLElement | null>(null);
  const latestTask = tasks[0];
  const preview = latestTask && !latestTask.download_requested && !dismissedPreviewIds.has(latestTask.id)
    && (latestTask.status === 'ready' || latestTask.status === 'parsing') ? latestTask : undefined;
  const runningCount = tasks.filter(t => t.status === 'downloading').length;
  const queued = tasks.filter(t => t.status === 'queued').sort((a, b) => (a.queue_order ?? 0) - (b.queue_order ?? 0));
  const activeCount = runningCount + queued.length + tasks.filter(t => t.download_requested && t.status === 'parsing').length;
  const selectedTask = tasks.find(t => t.id === selected && !dismissedPreviewIds.has(t.id));
  const current = previewReset ? undefined : selectedTask?.status === 'completed' ? preview : selectedTask ?? preview;
  const parsing = preview?.status === 'parsing' || current?.status === 'parsing';
  const resolvingTask = preview?.status === 'parsing' ? preview : current?.status === 'parsing' ? current : undefined;
  const progressTask = resolvingTask ?? (!resolveSubmitting && !resolveRequestError && current?.resolve_progress ? current : undefined);
  const showResolveProgress = resolveSubmitting || parsing || !!resolveRequestError || !!(current?.resolve_progress && !current.download_requested && (current.status === 'failed' || current.status === 'canceled'));
  useEffect(() => { if (parsing) setResolveRequestError(''); }, [parsing]);
  const resolution = resolutionLabel(current?.video?.width, current?.video?.height);
  const previewImageCount = current ? albumImageCount(current) : 0;
  const previewBytes = current?.status === 'completed' ? current.bytes : current?.total_bytes ?? current?.video?.filesize;
  const previewSize = previewBytes != null ? bytes(previewBytes) : current?.video?.filesize_approx != null ? '约 ' + bytes(current.video.filesize_approx) : '—';
  useEffect(() => {
    if (!connected) return;
    if (lastStatuses.current) {
      for (const task of tasks) {
        const previous = lastStatuses.current.get(task.id);
        if (task.status === 'completed') {
          if (!task.error && (previous?.status !== 'completed' || previous.download_requested_at !== task.download_requested_at)) {
            toast.success('下载完成', { id: task.id, description: task.video?.title || '视频已保存到本地', duration: 5000 });
          } else if (previous && task.error && task.error !== 'CANCELED'
            && (previous.status !== 'completed' || previous.error !== task.error)) {
            toast.error('重新下载失败', { id: task.id, description: errorText(task.error), duration: 8000 });
          }
        } else if (task.status === 'failed' && previous?.status !== 'failed') {
          toast.error(task.download_requested ? '下载失败' : '解析失败', { id: task.id, description: errorText(task.error), duration: 8000 });
        }
      }
    }
    lastStatuses.current = new Map(tasks.map(task => [task.id, task]));
    if (selectedTask?.status === 'completed') {
      const submitted = submittedInput.current;
      if (submitted?.taskId === selectedTask.id) {
        // A replacement can acknowledge the old ID before its queued event arrives.
        if (selectedTask.revision <= submitted.afterRevision || (submitted.replacedRequestAt !== undefined
          && submitted.replacedRequestAt === (selectedTask.download_requested_at ?? null))) return;
        submittedInput.current = null;
        if (!selectedTask.error) setInput(value => value === submitted.value ? '' : value);
      }
      setSelected(null);
    }
  }, [tasks, connected, selectedTask]);
  useEffect(() => {
    if (downloadMode !== 'post' || current?.status !== 'ready' || promptedDuplicates.current.has(duplicatePromptKey(current))) return;
    const previous = previousDownload(tasks, current);
    if (previous) { promptedDuplicates.current.add(duplicatePromptKey(current)); setDuplicate({task: current, previous}); }
  }, [current, downloadMode, tasks]);
  async function download(task: Task, background = false, consent?: Task) {
    const latest = tasksRef.current.find(t => t.id === task.id) ?? task;
    const confirmDuplicate = !!consent && !!task.video && !!latest.video && sameDownloadVariant(task.video, latest.video);
    task = latest;
    const previous = task.video && !confirmDuplicate && previousDownload(tasksRef.current, task);
    if (previous) { promptedDuplicates.current.add(duplicatePromptKey(task)); setDuplicate({task, previous, background}); return; }
    try {
      const replaceTaskId = confirmDuplicate && consent?.status === 'completed' && consent.video && task.video
        && sameReplacementVariant(consent.video, task.video) ? consent.id : undefined;
      const replaced = replaceTaskId ? tasksRef.current.find(t => t.id === replaceTaskId) ?? consent : undefined;
      const submitted = submittedInput.current;
      const source = submitted?.taskId === task.id ? submitted.value : task.video?.canonical_url ?? task.telegram_link ?? '';
      const field = inputRef.current;
      const queuedId = await api.download(task.id, confirmDuplicate, replaceTaskId);
      if (background) {
        setDismissedPreviewIds(old => new Set(old).add(task.id));
        setSelected(null); setPreviewReset(true);
        setInput(value => value === source ? '' : value);
        submittedInput.current = null;
        requestAnimationFrame(() => { if (inputRef.current === field) field?.focus(); });
        toast.success('已加入下载任务');
        // Fill the task list from Rust without waiting for its queued event.
        void api.tasks().then(snapshot => setTasks(old => mergeTasks(old, snapshot, removedIds.current))).catch(() => {});
      } else {
        const taskId = queuedId || task.id;
        submittedInput.current = {taskId, value: source, afterRevision: replaced?.revision ?? task.revision,
          replacedRequestAt: replaced ? replaced.download_requested_at ?? null : undefined};
        setSelected(taskId);
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
  async function cancelTask(intent: NonNullable<typeof confirm>) {
    await api.cancel(intent.task.id);
    if (intent.resetPreview) {
      setDismissedPreviewIds(old => new Set(old).add(intent.task.id));
      setSelected(null); setPreviewReset(true);
      setInput(value => value === intent.input ? '' : value);
      submittedInput.current = null;
      requestAnimationFrame(() => inputRef.current?.focus());
    } else {
      requestAnimationFrame(() => {
        if (document.activeElement === document.body || cancelTrigger.current?.contains(document.activeElement)) {
          taskListRef.current?.focus({preventScroll: true});
        }
      });
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
        unsubscribe = await api.subscribe(task => { if (!disposed) setTasks(old => mergeTasks(old, [task], removedIds.current)); }, (id, spaceId) => { if (!disposed) removeTasks([id], spaceId); });
        if (disposed) { unsubscribe(); return; }
        const [snapshot, config] = await Promise.all([api.tasks(), api.settings()]);
        if (!disposed) {
          setTasks(old => mergeTasks(old, snapshot, removedIds.current)); setSettings(config); setConnected(true);
          // Restored previews must not open unsolicited duplicate dialogs when
          // a different preview is handed to the background queue.
          for (const restored of snapshot) {
            if (restored.status === 'ready' && restored.video) promptedDuplicates.current.add(duplicatePromptKey(restored));
          }
          const restored = [...snapshot].sort(compareTaskInsertion)[0];
          if (restored?.status === 'ready' && !restored.download_requested && restored.video) {
            const source = restored.video.canonical_url;
            setInput(value => value || source);
          }
        }
      } catch { if (!disposed) toast.error('无法连接本地应用，请通过桌面程序启动映取。', { id: 'connection-error', duration: 8000 }); }
    })();
    const refresh = () => { api.tasks().then(snapshot => { if (!disposed) setTasks(old => mergeTasks(old, snapshot, removedIds.current)); }).catch(() => {}); };
    window.addEventListener('focus', refresh);
    return () => { disposed = true; unsubscribe?.(); window.removeEventListener('focus', refresh); };
  }, []);
  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    try { await action(); } catch (e) { reportActionError(e); } finally { setBusy(false); }
  }
  async function runTaskAction(id: string, action: () => Promise<unknown>) {
    if (pendingTaskActions.current.has(id)) return;
    pendingTaskActions.current.add(id);
    setWaitingTaskActions(new Set(pendingTaskActions.current));
    try { await action(); }
    catch (error) { if (String(error) !== 'CANCELED') reportActionError(error); }
    finally {
      pendingTaskActions.current.delete(id);
      setWaitingTaskActions(new Set(pendingTaskActions.current));
    }
  }
  function waitingTaskAction(task: Task) {
    return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className="task-feed-play" aria-label="取消等待" onClick={() => void api.cancel(task.id).catch(reportActionError)}><X aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>等待资源释放，点击取消</TooltipContent></Tooltip>;
  }
  function reportActionError(error: unknown) {
    const message = errorText(error);
    toast.error('操作失败', { id: 'action-error', description: message, duration: 8000 });
  }
  function assetPlayback(asset: BackupAsset): PlaybackSubject {
    return { id: asset.asset_id, path: asset.path, completed_at: null, video: asset.metadata, asset: true };
  }
  function playTaskEntry(task: Task, returnFocus: HTMLElement | null, streamStep?: number): TaskMediaEntry {
    const asset = task.file_missing ? backup.assetsById.get(task.asset_id ?? '') : undefined;
    return { kind: 'task', id: task.id, returnFocus, streamStep, playVideo: true, asset: asset && assetPlayback(asset) };
  }
  function playTask(task: Task, trigger?: HTMLElement) {
    if (busy || waitingTaskActions.has(task.id) || !canPlayTask(task)) return false;
    if (mediaEntry?.kind === 'task' && mediaEntry.id === task.id && mediaEntry.playVideo) return true;
    const row = Array.from(taskListRef.current?.querySelectorAll<HTMLElement>('[data-task-row]') ?? []).find(node => node.dataset.taskRow === task.id);
    setMediaEntry(playTaskEntry(task, trigger ?? row?.querySelector<HTMLElement>('.task-context-target') ?? null));
    return true;
  }
  function openTaskCover(task: Task, returnFocus: HTMLElement) {
    if (!playTask(task, returnFocus)) setMediaEntry({ kind: 'task', id: task.id, returnFocus });
  }
  async function pasteLink() {
    const field = inputRef.current;
    if (pasting || !field || field.disabled || (downloadMode === 'post' && (busy || parsing))) return;
    const before = field.value;
    setPasting(true);
    try {
      const text = await api.readClipboardText();
      if (inputRef.current !== field || field.disabled || field.value !== before) return;
      if (!text.trim()) { toast.info('剪贴板中没有文本内容'); return; }
      setInput(text); inputRef.current?.focus();
    } catch { toast.error('无法读取剪贴板，请使用 Ctrl / ⌘ + V 粘贴。'); }
    finally { setPasting(false); }
  }
  function removeTasks(ids: string[], spaceId?: string) {
    const fresh = new Set(ids.filter(id => !removedIds.current.has(id)));
    if (!fresh.size) return;
    setDismissedTaskErrorRevisions(old => {
      if (![...fresh].some(id => old.has(id))) return old;
      const next = new Map(old);
      fresh.forEach(id => next.delete(id));
      return next;
    });
    const list = taskListRef.current;
    const rows = Array.from(list?.querySelectorAll<HTMLElement>('[data-task-row]') ?? []);
    const focusedRow = rows.find(row => fresh.has(row.dataset.taskRow!) && row.contains(document.activeElement));
    const index = focusedRow ? rows.indexOf(focusedRow) : -1;
    const neighbor = [...rows.slice(index + 1), ...rows.slice(0,index).reverse()].find(row => !fresh.has(row.dataset.taskRow!));
    const nextFocus = neighbor?.querySelector<HTMLElement>('button:not(:disabled):not([aria-disabled="true"]), [tabindex="0"]');
    for (const task of tasksRef.current) if (fresh.has(task.id) && task.asset_id) removedAssetIds.current.add(`${spaceId ?? '*'}:${task.asset_id}`);
    videoTranscripts.remove(ids);
    forgetTaskImages(ids);
    fresh.forEach(id => removedIds.current.add(id));
    try { forgetPlaybackPositions([...fresh]); } catch (reason) { toast.error('无法清理播放进度', { description: errorText(reason) }); }
    const rendered = new Set(rows.map(row => row.dataset.taskRow));
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
      const departing = tasksRef.current.filter(task => fresh.has(task.id) && rendered.has(task.id));
      setExitingTasks(old => [...old, ...departing]);
    }
    setTasks(old => old.filter(task => !fresh.has(task.id)));
    setSelected(old => old && fresh.has(old) ? null : old);
    setMediaEntry(old => old?.kind === 'task' && fresh.has(old.id) ? null : old);
    if (focusedRow) requestAnimationFrame(() => {
      if (list && taskListRef.current === list && (document.activeElement === document.body || focusedRow.contains(document.activeElement))) {
        (nextFocus?.isConnected ? nextFocus : list).focus({preventScroll:true});
      }
    });
  }

  async function removeTask(id: string, deleteFile = false) {
    const list = taskListRef.current;
    const row = Array.from(list?.querySelectorAll<HTMLElement>('[data-task-row]') ?? []).find(node => node.dataset.taskRow === id);
    const neighbor = row?.nextElementSibling ?? row?.previousElementSibling;
    const nextFocus = neighbor?.querySelector<HTMLElement>('button:not(:disabled):not([aria-disabled="true"])');
    if (deleteFile) {
      await api.deleteFile(id);
      const snapshot = await api.tasks();
      if (snapshot.some(task => task.id === id)) {
        setTasks(old => mergeTasks(old, snapshot, removedIds.current));
        await backup.reload();
        return;
      }
    } else await api.remove(id);
    removeTasks([id]);
    requestAnimationFrame(() => {
      if (list && taskListRef.current === list && (document.activeElement === document.body || row?.contains(document.activeElement))) {
        (nextFocus?.isConnected ? nextFocus : list).focus({preventScroll: true});
      }
    });
  }
  async function parse(event: React.FormEvent) {
    event.preventDefault();
    if (pasting || busy || parsing) return;
    if (!input.trim()) { toast.info('请输入视频分享链接', { id: 'empty-share-link' }); inputRef.current?.focus(); return; }
    setResolveSource(input); setResolveSubmitting(true); setResolveRequestError(''); setBusy(true);
    try {
      if (preview?.status === 'ready') await api.cancel(preview.id);
      const task = await api.resolve(input, resolver);
      submittedInput.current = {taskId: task.id, value: input, afterRevision: task.revision};
      setTasks(old => mergeTasks(old, [task], removedIds.current)); setSelected(task.id); setPreviewReset(false);
    } catch (error) {
      setResolveRequestError(errorText(error)); reportActionError(error);
    } finally { setBusy(false); setResolveSubmitting(false); }
  }
  const nav = [{id:'download',label:'下载作品',Icon:Download},{id:'tasks',label:'下载任务',Icon:ListVideo},{id:'settings',label:'系统设置',Icon:SettingsIcon},{id:'about',label:'关于软件',Icon:Info}];
  // Retain row order for legacy snapshots and removal animations.
  for (const task of [...tasks, ...exitingTasks]) if (!taskOrder.current.has(task.id)) taskOrder.current.set(task.id, taskOrder.current.size);
  const filteredTasks = [...tasks, ...exitingTasks].filter(t => t.status !== 'canceled').filter(t => authorQuery || t.download_requested).filter(t => taskPlatform === 'all' || (t.video?.platform ?? (t.telegram_link ? 'telegram' : undefined)) === taskPlatform).filter(t => authorQuery ? matchesAuthor(t.video, authorQuery) : !search || ((t.video?.title ?? '') + (t.video?.author ?? '') + (t.video?.author_profile?.nickname ?? '') + (t.video?.author_profile?.id ?? '') + (t.telegram_link ?? '')).includes(search));
  const statusCounts: Record<TaskStatusFilter, number> = {all: 0, downloading: 0, queued: 0, completed: 0, failed: 0};
  for (const task of filteredTasks) {
    if (removedIds.current.has(task.id)) continue;
    statusCounts.all++;
    if (task.status === 'parsing' || task.status === 'downloading') statusCounts.downloading++;
    else if (task.status === 'queued' || task.status === 'completed' || task.status === 'failed') statusCounts[task.status]++;
  }
  const visible = filteredTasks.filter(t => page !== 'tasks' || taskStatus === 'all' || (taskStatus === 'downloading' ? t.status === 'downloading' || t.status === 'parsing' : t.status === taskStatus)).sort((a, b) => compareTaskInsertion(a, b) || taskOrder.current.get(a.id)! - taskOrder.current.get(b.id)!);
  const taskAssetIds = new Set([...tasks, ...exitingTasks].map(task => task.asset_id));
  const remoteAssets = [...backup.assets, ...backup.exitingAssets].filter(a => !removedAssetIds.current.has(`*:${a.asset_id}`) && !removedAssetIds.current.has(`${a.space_id}:${a.asset_id}`) && !taskAssetIds.has(a.asset_id) && (a.asset_state === 'active' || !!a.path || a.remote_state === 'pending_delete'));
  for (const asset of remoteAssets) if (!taskOrder.current.has('asset-' + asset.asset_id)) taskOrder.current.set('asset-' + asset.asset_id, taskOrder.current.size);
  remoteAssets.sort((a,b) => taskOrder.current.get('asset-' + a.asset_id)! - taskOrder.current.get('asset-' + b.asset_id)!);
  const filteredAssets = remoteAssets.filter(a => (taskPlatform === 'all' || a.metadata.platform === taskPlatform) && (!search || `${a.metadata.title ?? ''} ${a.metadata.author ?? ''} ${a.original_filename}`.includes(search)));
  statusCounts.all += filteredAssets.length;
  for (const asset of filteredAssets) {
    const status = backupAssetStatus(backup, asset);
    if (status) statusCounts[status]++;
  }
  const visibleAssets = filteredAssets.filter(a => page !== 'tasks' || taskStatus === 'all' || backupAssetStatus(backup, a) === taskStatus);
  function adjacentMedia(direction: MediaDirection): TaskMediaEntry | undefined {
    if (!taskListPage || mediaEntry?.streamStep === undefined || busy) return;
    const rows = taskListRows(visible, visibleAssets);
    const current = rows.findIndex(row => row.kind === mediaEntry.kind && (row.kind === 'task' ? row.task.id : row.asset.asset_id) === mediaEntry.id);
    if (current < 0) return;
    const { returnFocus } = mediaEntry;
    const streamStep = mediaEntry.streamStep + 1;
    for (let i = current + direction; i >= 0 && i < rows.length; i += direction) {
      const row = rows[i];
      if (row.kind === 'task') {
        const { task } = row;
        if (removedIds.current.has(task.id) || waitingTaskActions.has(task.id)) continue;
        if (canPlayTask(task)) {
          return playTaskEntry(task, returnFocus, streamStep);
        }
        if (albumMedia(task)?.length && !(task.asset_id && backupDeletion(backup, task.asset_id))) {
          return { kind: 'task', id: task.id, returnFocus, streamStep, startAtLast: direction === -1 };
        }
      } else {
        const { asset } = row;
        if (backup.exitingAssets.some(item => item.asset_id === asset.asset_id)) continue;
        const status = backupAssetStatus(backup, asset);
        if (canPlayBackupAsset(asset, !!backupDeletion(backup, asset.asset_id), taskView === 'feed' && (status === 'queued' || status === 'downloading'))) {
          return { kind: 'asset', id: asset.asset_id, returnFocus, streamStep, asset: assetPlayback(asset) };
        }
      }
    }
  }
  const nextStreamEntry = adjacentMedia(1);
  // Preparing a backup asset can restore it from the remote library. Warm local videos only.
  const preloadSubject = nextStreamEntry?.kind === 'task' && nextStreamEntry.playVideo && !nextStreamEntry.asset
    ? tasks.find(task => task.id === nextStreamEntry.id) : undefined;
  const preload = nextStreamEntry && preloadSubject ? { entry: nextStreamEntry, subject: preloadSubject } : undefined;
  function showAuthorWorks(task: Task) {
    const query = authorSearch(task.video);
    if (!query) return;
    setAuthorQuery(query); setSearch(query.name); setTaskPlatform('all'); setPage('search');
    requestAnimationFrame(() => taskSearchRef.current?.focus());
  }
  const historyCount = tasks.filter(t => t.download_requested && (taskPlatform === 'all' || (t.video?.platform ?? (t.telegram_link ? 'telegram' : undefined)) === taskPlatform) && ['completed', 'failed'].includes(t.status)).length;
  function requestCancel(task: Task, returnFocus: HTMLElement | null) {
    cancelTrigger.current = returnFocus;
    setConfirm({task, resetPreview: page === 'download' && !task.download_requested, input});
  }
  function taskMenu(task: Task, children?: React.ReactNode, coverActions?: CoverMenuActions) {
    const asset = backup.assetsById.get(task.asset_id ?? '');
    const member = asset?.in_current_space !== false ? asset : undefined;
    const deleting = task.asset_id ? backupDeletion(backup, task.asset_id) : null;
    const deleteFromBackup = !!task.asset_id && !!backupSettings?.enabled && (backupSettings.provider_type !== 'telegram' || backupSettings.can_delete_owned_media) && !!member && !member.remote_deleted;
    const deleteUnavailable = !!member && backupSettings?.enabled && backupSettings.provider_type === 'telegram' && !backupSettings.can_delete_owned_media;
    const stream = taskListPage && children !== undefined && !removedIds.current.has(task.id)
      && (canPlayTask(task) || !!albumMedia(task)?.length)
      ? (returnFocus: HTMLElement | null) => setMediaEntry(canPlayTask(task)
        ? playTaskEntry(task, returnFocus, 0)
        : { kind: 'task', id: task.id, returnFocus, streamStep: 0 })
      : undefined;
    return <TaskMoreMenu stream={stream} children={children} backupActions={task.asset_id ? <BackupTransferMenuItems backup={backup} assetId={task.asset_id} context={children !== undefined} /> : undefined} coverActions={coverActions} onOpenChange={open => { if (open) React.startTransition(() => setSelectedRow(task.id)); }} task={task} play={canPlayTask(task) ? () => playTask(task) : undefined} deleteBackup={deleteFromBackup} release={member?.remote_state === 'backed_up' && !member.remote_deleted ? () => { void run(async () => { await api.releaseAsset(task.asset_id!); setTasks(await api.tasks()); toast.success('本机副本已释放'); }); } : undefined} busy={busy || !!deleting || waitingTaskActions.has(task.id)} waitingCancel={waitingTaskActions.has(task.id) ? () => { void api.cancel(task.id).catch(reportActionError); } : undefined} cancel={taskListPage && ['parsing','ready','queued','downloading'].includes(task.status) ? returnFocus => requestCancel(task, returnFocus) : undefined} transcript={task.status === 'completed' && task.video?.kind !== 'image_album' && (!task.file_missing || videoTranscripts.values[task.id]?.has_result || !!asset && (asset.has_transcript || asset.in_current_space !== false && !asset.remote_deleted)) ? { label: videoTranscripts.values[task.id]?.state === 'running' ? '正在提取文案' : videoTranscripts.values[task.id]?.has_result || asset?.has_transcript ? '查看文案' : '提取文案', open: returnFocus => videoTranscripts.open(task.id, returnFocus) } : undefined} reveal={() => { void run(() => api.reveal(task.id)); }} copy={() => { void run(() => navigator.clipboard.writeText(task.video?.canonical_url ?? task.telegram_link!)); }} copyPath={() => { void run(async () => { await navigator.clipboard.writeText(displayPath(task.path)); toast.success('本地地址已复制'); }); }} remove={() => { void runTaskAction(task.id, () => removeTask(task.id)); }} deleteFile={deleteUnavailable ? undefined : deleteFromBackup ? () => { void backup.deleteAsset(task.asset_id!); } : () => { void runTaskAction(task.id, () => removeTask(task.id, true)); }} />;
  }
  function canPlayTask(task: Task) {
    if (task.status !== 'completed' || !task.video || task.video.kind === 'image_album') return false;
    if (task.asset_id && backupDeletion(backup, task.asset_id)) return false;
    if (!task.file_missing) return !!task.path;
    const asset = backup.assetsById.get(task.asset_id ?? '');
    return !!backupSettings?.enabled && !!asset && !asset.remote_deleted && asset.remote_state === 'backed_up';
  }
  function playAction(task: Task, overlay = false) {
    return <Tooltip><TooltipTrigger asChild><Button size={taskListPage ? 'icon' : 'sm'} variant={taskListPage ? 'ghost' : 'outline'} className={cn('task-play', overlay && 'task-feed-play cursor-pointer')} aria-label="播放视频" disabled={busy || mediaEntry?.kind === 'task' && mediaEntry.id === task.id} onClick={event => playTask(task, event.currentTarget)}><Play aria-hidden="true" />{!taskListPage && '播放'}</Button></TooltipTrigger><TooltipContent>在应用内播放</TooltipContent></Tooltip>;
  }
  function cancelAction(task: Task, overlay = false) {
    return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className={overlay ? 'task-feed-play' : undefined} aria-label={task.canceling ? '正在取消…' : task.status === 'parsing' ? '取消解析' : '取消'} disabled={busy || task.canceling} onClick={event => requestCancel(task, event.currentTarget)}>{task.canceling ? <Spinner aria-hidden="true" /> : <X aria-hidden="true" />}</Button></TooltipTrigger><TooltipContent>{task.canceling ? '正在取消…' : task.status === 'parsing' ? '取消解析' : '取消下载'}</TooltipContent></Tooltip>;
  }
  function feedAction(task: Task) {
    if (waitingTaskActions.has(task.id)) return waitingTaskAction(task);
    if (task.status === 'parsing') return cancelAction(task, true);
    if (task.status === 'ready') return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className="task-feed-play" aria-label={task.video?.kind === 'image_album' ? '下载图集' : '下载视频'} disabled={busy} onClick={() => void run(() => download(task))}><Download aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>点击下载</TooltipContent></Tooltip>;
    if (task.status === 'canceled' || task.status === 'failed') return retryAction(task);
    const asset = backup.assetsById.get(task.asset_id ?? '');
    const restore = asset ? backupAssetStatus(backup, asset) : null;
    if (task.status === 'completed' && asset && !backupDeletion(backup, asset.asset_id) && ['queued','downloading','failed'].includes(restore ?? '')) {
      const restoring = restore !== 'failed';
      return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className="task-feed-play" aria-label={restoring ? '正在下载' : '重试下载'} disabled={busy || restoring} onClick={event => void playTask(task, event.currentTarget)}>{restoring ? <Spinner aria-hidden="true" /> : <RotateCcw aria-hidden="true" />}</Button></TooltipTrigger><TooltipContent>{restoring ? '正在下载' : '下载失败，点击重试下载'}</TooltipContent></Tooltip>;
    }
    if (task.status === 'completed' && task.asset_id && !backupDeletion(backup, task.asset_id)) {
      const state = backupCardStatus(backup, task.asset_id, task);
      if (state.action || ['清理中','待确认','正在取消'].includes(state.label)) return <BackupCardAction backup={backup} assetId={task.asset_id} />;
    }
    if (canPlayTask(task)) return playAction(task, true);
    if (task.status === 'completed' && task.video?.kind === 'image_album' && !task.file_missing) return albumAction(task, true);
    return null;
  }
  function cancelProgress(task: Task) {
    const queued = task.status === 'queued';
    return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className="task-feed-progress-cancel" aria-label={task.canceling ? '正在取消…' : '取消下载'} disabled={busy || task.canceling} onClick={event => requestCancel(task, event.currentTarget)}><Progress variant="circular" updateIntervalMs={1000} aria-label={queued ? '等待下载' : task.merging ? '合并进度' : '下载进度'} aria-valuetext={task.canceling ? '正在取消' : queued ? '等待下载' : task.merging ? '正在合并音视频' : downloadPercent(task) == null ? '正在下载，大小未知' : undefined} value={queued || task.merging ? null : downloadPercent(task)}>{task.canceling ? <Spinner aria-hidden="true" /> : queued ? '等待' : task.merging ? '合并' : undefined}</Progress></Button></TooltipTrigger><TooltipContent>{task.canceling ? '正在取消…' : '点击取消下载'}</TooltipContent></Tooltip>;
  }
  function albumAction(task: Task, overlay = false) {
    return <Tooltip><TooltipTrigger asChild><Button size={taskListPage ? 'icon' : 'sm'} variant={taskListPage ? 'ghost' : 'outline'} className={overlay ? 'task-feed-play cursor-pointer' : undefined} aria-label="查看图集" onClick={event => setMediaEntry({ kind: 'task', id: task.id, returnFocus: event.currentTarget })}>{taskListPage ? <Images aria-hidden="true" /> : <Eye aria-hidden="true" />}{!taskListPage && '查看图集'}</Button></TooltipTrigger><TooltipContent>查看图集</TooltipContent></Tooltip>;
  }
  function retryTask(task: Task) {
    if (task.video || (task.telegram_link && task.download_requested)) {
      setDismissedTaskErrorRevisions(old => new Map(old).set(task.id, task.revision));
      return runTaskAction(task.id, async () => {
        const next = await api.retry(task.id);
        toast.dismiss(task.id);
        promptedDuplicates.current.delete(duplicatePromptKey(next));
        setDismissedPreviewIds(old => { const updated = new Set(old); updated.delete(next.id); return updated; });
        setTasks(old => mergeTasks(old, [next], removedIds.current));
        setSelected(next.id);
        setPreviewReset(false);
      });
    }
    setPage('download'); setSelected(null); setInput('');
    requestAnimationFrame(() => inputRef.current?.focus());
  }
  function retryAction(task: Task) {
    const label = task.video || (task.telegram_link && task.download_requested) ? '重试下载' : '重新解析';
    return <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" className="task-feed-play" aria-label={label} disabled={busy} onClick={() => void retryTask(task)}>{task.status === 'canceled' ? <CircleX aria-hidden="true" /> : <RotateCcw aria-hidden="true" />}</Button></TooltipTrigger><TooltipContent collisionPadding={8}>{task.status === 'canceled' ? '已取消' : task.error === 'INTERRUPTED' ? '任务已中断' : '下载失败'}，点击{label}</TooltipContent></Tooltip>;
  }
  function taskActions(task: Task) {
    if (waitingTaskActions.has(task.id)) return <div className="task-actions flex justify-end">{waitingTaskAction(task)}</div>;
    return <div className="task-actions flex flex-wrap items-center justify-end gap-2">
      {page === 'download' && task.status === 'ready' && <Button size="sm" data-background-download-task={task.id} disabled={busy} onClick={() => run(() => download(task, true))}><ListVideo data-icon="inline-start" />后台下载</Button>}
      {task.status === 'ready' && <Button variant={page === 'download' ? 'outline' : 'default'} data-download-task={task.id} disabled={busy} onClick={() => run(() => download(task))}><Download data-icon="inline-start" />{page === 'download' ? '立即下载' : task.video?.kind === 'image_album' ? '下载图集' : '下载视频'}</Button>}
      {['parsing','ready','queued','downloading'].includes(task.status) && (taskListPage && task.status === 'downloading'
        ? cancelAction(task)
        : <Button size="sm" variant="outline" disabled={busy || task.canceling} onClick={event => requestCancel(task, event.currentTarget)}>{task.canceling ? '正在取消…' : '取消'}</Button>)}
      {task.status === 'completed' && task.video?.kind === 'image_album' && !task.file_missing && albumAction(task)}
      {canPlayTask(task) && playAction(task)}

      {['failed', 'canceled'].includes(task.status) && <Button size="sm" variant="outline" disabled={busy} onClick={() => void retryTask(task)}><RotateCcw />{task.video || (task.telegram_link && task.download_requested) ? '重试' : '重新解析'}</Button>}
      {page === 'download' && taskMenu(task)}
    </div>;
  }
  function emptyState(history = false) {
    return <Card size="sm" className="[--card-spacing:12px]"><CardContent><Empty className="min-h-[160px] gap-2 p-4"><EmptyHeader><EmptyMedia variant="icon">{history ? <ListVideo /> : <Film />}</EmptyMedia><EmptyTitle>{history ? search || (page === 'tasks' && taskStatus !== 'all') ? '没有匹配的下载任务' : taskPlatform !== 'all' ? '该平台暂无下载任务' : '还没有下载记录' : '粘贴链接开始解析'}</EmptyTitle><EmptyDescription>{history ? search || (page === 'tasks' && taskStatus !== 'all') ? '试试其他关键词，或切换状态与平台' : taskPlatform !== 'all' ? '切换“全部”查看其他平台的任务' : '下载任务和已完成记录会显示在这里' : '作品信息将显示在这里'}</EmptyDescription></EmptyHeader>{page === 'download' && <EmptyContent><div className="flex gap-2"><Badge variant="secondary">抖音</Badge><Badge variant="secondary">TikTok</Badge><Badge variant="secondary">YouTube</Badge><Badge variant="secondary">Telegram</Badge></div></EmptyContent>}</Empty></CardContent></Card>;
  }
  return <TooltipProvider><Toaster position="top-center" closeButton /><SidebarProvider className="h-dvh min-h-0 overflow-hidden" open={wide} onOpenChange={setWide} style={{'--sidebar-width':'192px','--sidebar-width-icon':'56px'} as React.CSSProperties}>
    <Sidebar collapsible="icon"><SidebarHeader className="app-brand-header h-16 justify-center px-2"><div data-slot="app-brand" className="flex h-10 items-center gap-2 px-2 group-data-[collapsible=icon]:px-0"><img src={brandIcon} alt="" className="size-8 shrink-0 object-contain" /><span className="grid min-w-0 flex-1 grid-cols-[auto_minmax(0,1fr)] items-center gap-2 group-data-[collapsible=icon]:hidden"><span className="whitespace-nowrap text-xl font-semibold">映取</span><Badge variant="outline" className="min-w-0 max-w-full justify-self-end" title={__APP_VERSION__}><span className="truncate">{__APP_VERSION__}</span></Badge></span></div></SidebarHeader><SidebarSeparator /><SidebarContent><SidebarGroup className="px-2 py-2"><SidebarGroupContent><SidebarMenu className="gap-1">{nav.map(({id,label,Icon}) => <SidebarMenuItem key={id}><SidebarMenuButton className="app-navigation-item" isActive={page === id || id === 'tasks' && page === 'search'} aria-current={page === id || id === 'tasks' && page === 'search' ? 'page' : undefined} tooltip={label} onClick={() => { setPage(id); setSearch(''); setAuthorQuery(null); }}><Icon /><span>{label}</span></SidebarMenuButton>{id === 'tasks' && activeCount > 0 && <SidebarMenuBadge>{activeCount}</SidebarMenuBadge>}</SidebarMenuItem>)}</SidebarMenu></SidebarGroupContent></SidebarGroup></SidebarContent><SidebarSeparator /><SidebarFooter className="p-2"><SidebarMenu><SidebarMenuItem><SidebarMenuButton asChild tooltip={connected ? '本地运行' : '连接不可用'}><div role="status"><HardDrive /><span>{connected ? '本地运行' : '连接不可用'}</span></div></SidebarMenuButton></SidebarMenuItem></SidebarMenu></SidebarFooter></Sidebar>
    <SidebarInset className={cn("min-h-0 min-w-0 overflow-hidden bg-background", taskListPage && "task-workspace")}><header className={cn("app-page-header", taskListPage && "task-page-header")}>
      <h1>{page === 'search' ? '搜索结果' : nav.find(n => n.id === page)?.label}</h1>
      {taskListPage && <div className="flex max-w-full items-center gap-2">{page === 'tasks' && <Tooltip><TooltipTrigger asChild><span className="inline-flex"><Button ref={clearHistoryTrigger} size="icon-sm" variant="ghost" aria-label="清理历史" disabled={busy || historyCount === 0} onClick={() => setClearHistoryPlatform(taskPlatform)}><ListX /></Button></span></TooltipTrigger><TooltipContent>{historyCount ? `清理${taskPlatform === 'all' ? '全部平台' : platformName(taskPlatform)}下载历史` : '当前平台暂无可清理的下载历史'}</TooltipContent></Tooltip>}<InputGroup className="h-7 w-[260px] max-w-full"><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput ref={taskSearchRef} aria-label="搜索作品标题或作者" placeholder="搜索作品标题或作者" value={search} onChange={e => { setSearch(e.target.value); setAuthorQuery(null); }} />{search.length > 0 && <InputGroupAddon align="inline-end"><InputGroupButton size="icon-xs" aria-label="清空搜索" title="清空搜索" onClick={() => { setSearch(''); setAuthorQuery(null); taskSearchRef.current?.focus(); }}><X aria-hidden="true" /></InputGroupButton></InputGroupAddon>}</InputGroup></div>}</header>
    <div ref={pageContentRef} data-slot="page-content" className={cn("min-h-0 flex-1", taskListPage || page === 'settings' ? "overflow-hidden" : "overflow-y-auto", page === 'about' && "mr-1")}><div className={cn("page-body flex w-full flex-col gap-4", taskListPage && "task-page-body", (taskListPage || page === 'settings') && "h-full min-h-0")}>
    {page === 'download' && <div className="flex flex-col gap-3">
      <ToggleGroup type="single" value={downloadMode} variant="outline" aria-label="下载范围" className="self-start"
        onValueChange={value => { if (value === 'post' || value === 'author') setDownloadMode(value); }}>
        <ToggleGroupItem value="post">单个作品</ToggleGroupItem>
        <ToggleGroupItem value="author">抖音主页</ToggleGroupItem>
      </ToggleGroup>
      {downloadMode === 'author' ? <AuthorDownload input={input} onChange={setInput} inputRef={inputRef}
        paste={() => void pasteLink()} pasting={pasting} connected={connected}
        openTasks={() => { setPage('tasks'); setSearch(''); setAuthorQuery(null); }} /> : <>
      <Card size="sm" className="share-card"><CardHeader><CardTitle>作品链接</CardTitle></CardHeader><CardContent><form onSubmit={parse}><FieldGroup className="gap-3"><Field orientation="horizontal"><FieldLabel className="sr-only" htmlFor="video-input">作品分享链接</FieldLabel><InputGroup className="min-h-8 flex-1"><ShareLinkInput value={input} onChange={setInput} inputRef={inputRef} disabled={busy || parsing} /></InputGroup><div className="flex shrink-0 flex-col gap-2"><Button type="button" variant="outline" className="min-w-[100px]" disabled={busy || pasting || parsing} onClick={() => void pasteLink()}>{pasting ? <Spinner aria-hidden="true" data-icon="inline-start" /> : <ClipboardPaste data-icon="inline-start" />}{pasting ? '粘贴中…' : '粘贴链接'}</Button><Button className="min-w-[100px]" variant={current?.video ? 'secondary' : 'default'} disabled={!connected || busy || pasting || parsing}>{parsing ? <Spinner data-icon="inline-start" /> : <Link data-icon="inline-start" />}{parsing ? '正在解析…' : current?.video ? '重新解析' : '解析链接'}</Button></div></Field><Field orientation="horizontal"><FieldLabel id="resolver-label">解析引擎</FieldLabel><Select value={resolver} onValueChange={value => setResolver(value as Resolver)} disabled={busy || parsing}><SelectTrigger size="sm" aria-labelledby="resolver-label" className="w-44"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="auto">自动，按平台选择</SelectItem><SelectItem value="yt-dlp">yt-dlp，视频与小红书图集</SelectItem><SelectItem value="dtk">DTK，视频与图集</SelectItem><SelectItem value="grammers">Telegram</SelectItem></SelectContent></Select></Field><FieldDescription>支持抖音、小红书、TikTok、YouTube、Telegram<span className="float-right">Ctrl / ⌘ + V 粘贴</span></FieldDescription></FieldGroup></form></CardContent></Card>
    {showResolveProgress ? <ResolveProgressCard task={progressTask} source={resolveSource} error={resolveRequestError || undefined} actions={progressTask ? progressTask.status === 'parsing' ? cancelAction(progressTask) : taskActions(progressTask) : undefined} /> : current ? <Card size="sm" className="media-preview"><CardHeader><div className="flex items-center gap-2"><Badge variant="outline">{platformName(current.video?.platform ?? (current.telegram_link ? 'telegram' : undefined))}</Badge>{current.video?.resolver && current.video.platform !== 'telegram' && <Badge variant="secondary">{current.video.resolver === 'dtk' ? 'DTK' : 'yt-dlp'}</Badge>}<Badge variant={current.canceling ? 'warning' : statusVariant[current.status]}>{current.canceling ? '正在取消…' : current.status === 'queued' && !current.video && current.telegram_link ? '等待解析' : labels[current.status]}</Badge>{current.cache_hit && <Badge variant="muted">解析缓存</Badge>}</div></CardHeader><CardContent className="media-preview-content"><>{current.video?.kind === 'image_album' && previewImageCount ? <AlbumGallery key={current.id} task={current} /> : <VideoCover task={current} operations={coverOperations} />}</><div className="media-preview-info flex min-w-0 flex-1 flex-col gap-3"><CardTitle className="line-clamp-2" title={current.video?.title ?? ''}>{current.video?.title || (current.status === 'parsing' ? '正在获取作品信息' : current.telegram_link && current.status === 'queued' ? '等待解析 Telegram 链接' : '等待有效视频链接')}</CardTitle><AuthorInfo task={current} /><dl className="grid grid-cols-3 gap-4"><div><dt className="text-xs text-muted-foreground">{current.video?.kind === 'image_album' ? '图片数量' : '时长'}</dt><dd className="mt-2 tabular-nums">{current.video?.kind === 'image_album' ? (previewImageCount ? `${previewImageCount} 张` : '未知') : duration(current.video?.duration_ms)}</dd></div><div><dt className="text-xs text-muted-foreground">{current.video?.kind === 'image_album' ? '图片质量' : '清晰度'}</dt><dd className="mt-2 tabular-nums">{current.video?.kind === 'image_album' ? (current.video.platform === 'xiaohongshu' ? '高清图片' : '原图优先，高清次选') : resolution}</dd></div><div><dt className="text-xs text-muted-foreground">文件大小</dt><dd className="mt-2 tabular-nums">{previewSize}</dd></div></dl>{current.status === 'ready' && !!current.video?.formats?.length && <Field className="gap-2"><FieldLabel>下载规格，{current.video.formats.length} 个</FieldLabel><ToggleGroup type="single" variant="outline" data-media-formats="" aria-label="下载视频规格" value={current.video.selected_format_id ?? ''} disabled={busy} onValueChange={value => { if (value) void run(async () => { const task = await api.chooseFormat(current.id, value); setTasks(old => mergeTasks(old, [task], removedIds.current)); }); }} className="grid max-h-[196px] w-full grid-cols-2 gap-2 overflow-y-auto p-1 pr-3 max-[1100px]:grid-cols-1">{current.video.formats.map(format => <ToggleGroupItem key={format.format_id} value={format.format_id} className="h-auto min-w-0 justify-between gap-2 px-3 py-2 text-left data-[state=on]:border-primary data-[state=on]:bg-accent data-[state=on]:text-primary"><span className="flex min-w-0 flex-col gap-0.5"><span className="font-medium">{format.format_id === 'original' ? '原始文件' : format.width && format.height ? `${format.width} × ${format.height}` : '清晰度未知'}</span><span className="text-xs text-muted-foreground">{format.container.toUpperCase()}{format.requires_merge ? '，需 FFmpeg' : ''}，{format.codec === 'h265' ? 'H.265' : format.codec === 'h264' ? 'H.264' : format.codec === 'av1' ? 'AV1' : format.codec || '编码未知'}{format.fps ? `，${format.fps} fps` : ''}{format.tbr ? `，${(format.tbr / 1000).toFixed(2)} Mbps` : ''}</span></span><span className="shrink-0 tabular-nums">{format.filesize != null ? bytes(format.filesize) : format.filesize_approx != null ? '约 ' + bytes(format.filesize_approx) : '大小未知'}</span></ToggleGroupItem>)}</ToggleGroup><FieldDescription>{current.video.platform === 'telegram' ? '下载原始文件，不转码。' : current.video.platform === 'douyin' && current.video.resolver === 'yt-dlp' ? '仅展示已确认含视频和音轨的一体规格；默认最高分辨率，下载时重新获取所选规格。' : '默认最高分辨率；下载时重新获取所选规格。YouTube 分离音视频规格需本机安装 FFmpeg，合并不转码。'}</FieldDescription></Field>}{current.video?.cookie_warning && <Alert className="border-0 bg-transparent p-0 text-muted-foreground"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{errorText(current.video.cookie_warning)}</AlertDescription></Alert>}{current.video?.music?.title && <p className="text-sm text-muted-foreground">原声：{current.video.music.title}{current.video.music.author ? `，${current.video.music.author}` : ''}</p>}{current.status === 'downloading' && <DownloadProgress task={current} />}{current.status === 'queued' && <p className="text-sm text-muted-foreground">{!current.video && current.telegram_link ? '等待解析并下载' : '等待下载'}，第 {queued.findIndex(t => t.id === current.id) + 1} 位</p>}{current.error && <Alert variant="destructive" className="border-0 bg-transparent p-0"><AlertCircle aria-hidden="true" /><AlertDescription className="text-xs text-inherit">{errorText(current.error)}</AlertDescription></Alert>}{current.status === 'canceled' && <CardDescription>已取消，重试将重新解析并从头下载。</CardDescription>}{current.path && <p className="truncate text-xs text-muted-foreground" title={displayPath(current.path)}>{displayPath(current.path)}</p>}</div></CardContent><CardFooter className="justify-end py-2">{taskActions(current)}</CardFooter></Card> : emptyState()}
    </>}
    <footer className="flex items-center gap-2 text-xs text-muted-foreground"><FolderOpen className="size-4 shrink-0" /><span className="min-w-0 truncate" title={displayPath(settings?.download_directory)}>下载目录：{displayPath(settings?.download_directory) || '—'}</span><Button variant="link" onClick={() => setPage('settings')}>修改设置</Button></footer></div>}
    <RetainedTaskPage active={taskListPage}>
    {page === 'tasks' && <TaskStatusFilters value={taskStatus} counts={statusCounts} onChange={value => { setTaskStatus(value); taskListRef.current?.scrollTo(0, 0); }} />}
    <TaskList tasks={visible} assets={visibleAssets} removingAssetIds={new Set(backup.exitingAssets.map(asset => asset.asset_id))} onAssetExited={backup.finishAssetRemoval} renderAsset={asset => <BackupAssetRow asset={asset} backup={backup} play={(returnFocus, streaming) => setMediaEntry({ kind: 'asset', id: asset.asset_id, returnFocus, streamStep: streaming ? 0 : undefined, asset: assetPlayback(asset) })} openTranscript={returnFocus => videoTranscripts.open(asset.asset_id, returnFocus)} feed={taskView === 'feed'} />} toolbar={backupSettings?.enabled && <Tooltip><TooltipTrigger asChild><Button size="icon" variant="ghost" aria-label="刷新备份" disabled={backup.refreshing} onClick={() => void backup.refresh()}><RefreshCw aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>刷新备份</TooltipContent></Tooltip>} paginationKey={JSON.stringify([page, taskPlatform, taskStatus, search, authorQuery])} platform={taskPlatform} onPlatformChange={setTaskPlatform} view={taskView} onViewChange={value => { try { localStorage.setItem(taskViewStorageKey, value); setTaskView(value); taskListRef.current?.scrollTo(0, 0); } catch (error) { reportActionError(error); } }} listRef={taskListRef} removingIds={removedIds.current} onExited={finishRemoval} empty={emptyState(true)} renderRow={task => <VideoCover task={task} mode="thumbnail" operations={coverOperations} feed={taskView === 'feed'} onOpen={returnFocus => openTaskCover(task, returnFocus)} renderTask={(cover, coverActions) => taskMenu(task, <TaskRow
      task={task} hidePreviousError={task.revision <= (dismissedTaskErrorRevisions.get(task.id) ?? -1)} waitingForResources={waitingTaskActions.has(task.id)} feed={taskView === 'feed'} selected={selectedRow === task.id} onSelect={() => setSelectedRow(task.id)}
      onPlay={!busy && canPlayTask(task) ? () => { void playTask(task); } : undefined}
      cover={cover} title={<>{task.asset_id && <span className="task-title-backup"><BackupTaskStatus backup={backup} assetId={task.asset_id} /></span>}<Tooltip><TooltipTrigger asChild><span className="task-row-title" tabIndex={0}>{taskTitle(task)}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{taskTitle(task)}</TooltipContent></Tooltip></>}
      metadata={<div className="task-row-byline"><AuthorInfo task={task} compact showId={false} onAuthorClick={authorSearch(task.video) ? () => showAuthorWorks(task) : undefined} /><span className="task-row-platform">{platformName(task.video?.platform ?? (task.telegram_link ? 'telegram' : undefined))}</span>{!(taskView === 'feed' && task.status === 'downloading') && <TaskMetadata task={task} compact hideMissing />}</div>}
      coverBadge={<UnifiedTaskBadge backup={backup} task={task} waiting={waitingTaskActions.has(task.id)} />}
      coverActions={taskView === 'feed' ? feedAction(task) : null}
      coverProgress={taskView === 'feed' && task.asset_id && backupDeletion(backup, task.asset_id) ? <BackupDeleteProgress backup={backup} assetId={task.asset_id} cover /> : taskView === 'feed' && (task.status === 'queued' || task.status === 'downloading') ? cancelProgress(task) : taskView === 'feed' && task.asset_id ? <BackupRestoreProgress backup={backup} assetId={task.asset_id} /> : null}
      feedback={task.asset_id && <BackupTaskFeedback backup={backup} assetId={task.asset_id} />}
      progress={<>{task.status === 'downloading' && <DownloadProgress task={task} hideIndicator={taskView === 'feed'} />}{taskView !== 'feed' && task.asset_id && <><BackupDeleteProgress backup={backup} assetId={task.asset_id} /></>}</>}
      queuePosition={queued.findIndex(t => t.id === task.id) + 1} actions={taskView === 'list' ? taskActions(task) : null}
    />, coverActions)} />} />
    </RetainedTaskPage>
    {(settingsActive || settingsVisited) && settings && <div hidden={!settingsActive} className={`${settingsActive ? 'grid' : 'hidden'} min-h-0 min-w-0 flex-1 grid-cols-[184px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-6 max-[1100px]:grid-cols-[160px_minmax(0,1fr)]`}>
      <nav aria-label="设置分类" className="settings-navigation self-stretch border-r pr-4"><SidebarMenu className="gap-2">{settingsSections.map(({ id, label, Icon }) => <SidebarMenuItem key={id}><SidebarMenuButton type="button" className="h-10 gap-3 px-3" isActive={settingsSection === id} aria-label={label} aria-current={settingsSection === id ? 'true' : undefined} aria-controls="settings-operation-area" onClick={() => setSettingsSection(id)}><Icon aria-hidden="true" /><span className="text-sm font-medium">{label}</span></SidebarMenuButton></SidebarMenuItem>)}</SidebarMenu></nav>
      <section id="settings-operation-area" aria-label={settingsSectionInfo?.label} className="flex min-h-0 min-w-0 flex-col gap-4">{settingsSection !== 'backup' && <header className="flex shrink-0 flex-col gap-1"><h2 className="text-sm font-semibold">{settingsSectionInfo?.label}</h2><p className="text-xs text-muted-foreground">{settingsSectionInfo?.description}</p>{settingsSection === 'dependencies' && <RuntimeDependenciesToolbar runtime={runtimeDependencies} />}</header>}
        <div hidden={settingsSection !== 'download'} className="flex min-h-0 flex-col gap-4 overflow-y-auto p-px pr-3"><Card size="sm" className="shrink-0 [--card-spacing:12px]"><CardContent><FieldGroup className="gap-3"><Field orientation="horizontal" className="grid grid-cols-[64px_minmax(0,1fr)]"><FieldLabel className="w-16 shrink-0" htmlFor="download-directory">下载目录</FieldLabel><div className="flex min-w-0 flex-1 items-center gap-2"><Input id="download-directory" className="min-h-8" readOnly value={displayPath(settings.download_directory)} /><Button variant="outline" disabled={busy} onClick={() => run(async () => setSettings(await api.directory()))}><FolderOpen data-icon="inline-start" />选择文件夹</Button></div></Field><DownloadConcurrency value={settings.max_downloads} disabled={busy} saved={setSettings} /></FieldGroup></CardContent></Card><FilenameSettings settings={settings} saved={setSettings} previewCache={filenamePreviewCache} /></div>
        <div hidden={settingsSection !== 'accounts'} className="min-h-0 flex-1"><Tabs value={settingsPlatform} onValueChange={setSettingsPlatform} className="settings-platforms h-full min-h-0 gap-5"><TabsList variant="line" aria-label="配置平台" className={settingsTabsListClassName}>{settings.profiles.filter(profile => profile.platform !== 'telegram').map(profile => <TabsTrigger key={profile.platform} value={profile.platform} className={settingsTabsTriggerClassName}>{platformName(profile.platform)}</TabsTrigger>)}<TabsTrigger value="telegram" className={settingsTabsTriggerClassName}>Telegram</TabsTrigger></TabsList>{settings.profiles.filter(profile => profile.platform !== 'telegram').map(profile => <TabsContent key={profile.platform} value={profile.platform} forceMount className="min-h-0 overflow-y-auto p-px pr-3 data-[state=inactive]:hidden"><ProfileForm profile={profile} saved={setSettings} /><Alert className="mt-5"><AlertCircle /><AlertDescription>公开作品解析成功只证明请求成功，不代表登录身份有效。</AlertDescription></Alert></TabsContent>)}<TabsContent value="telegram" className="min-h-0 overflow-y-auto p-px pr-3 data-[state=inactive]:hidden"><TelegramStorageSettings accountOnly active={settingsActive && settingsSection === 'accounts'} network={settings.profiles.find(profile => profile.platform === 'telegram')} networkSaved={setSettings} /></TabsContent></Tabs></div>
        {(backupSettingsActive || backupSettingsVisited) && <div hidden={settingsSection !== 'backup'} className={settingsSection === 'backup' ? 'flex min-h-0 flex-1' : 'hidden'}><BackupSettingsForm active={backupSettingsActive} saved={backup.setSettings} /></div>}
        <div hidden={settingsSection !== 'dependencies'} className="flex min-h-0 flex-col gap-4 overflow-y-auto p-px pr-3"><RuntimeDependencies runtime={runtimeDependencies} /></div>
      </section>
    </div>}
    {page === 'about' && <About busy={busy} openPage={(page, releaseTag) => { void run(() => api.openProjectPage(page, releaseTag)); }} copyVersion={() => { void run(async () => { await navigator.clipboard.writeText(aboutVersionInfo); toast.success('版本信息已复制'); }); }} />}
    </div></div></SidebarInset>
    <VideoTranscriptDialog client={videoTranscripts} tasks={tasks} assets={backup.assets} runtime={runtimeDependencies} />
    <TaskMediaDialog task={viewedTask} entry={mediaEntry} playable={viewPlaysVideo} preload={preload} coverOperations={coverOperations} removedIds={removedIds.current} adjacent={adjacentMedia} navigate={(previous, next) => setMediaEntry(current => current === previous ? next : current)} close={() => setMediaEntry(null)} fallbackFocus={() => taskListPage ? taskListRef.current : inputRef.current} />
    <AlertDialog open={!!duplicate} onOpenChange={open => { if (!open) setDuplicate(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const button = document.querySelector<HTMLButtonElement>('[' + (duplicateTrigger.current?.background ? 'data-background-download-task' : 'data-download-task') + '="' + duplicateTrigger.current?.id + '"]'); ((button && !button.disabled ? button : null) ?? inputRef.current ?? document.querySelector<HTMLButtonElement>('[aria-current="page"]'))?.focus(); }); }}><AlertDialogHeader><AlertDialogTitle>{duplicateCompleted ? '该规格已下载过' : '该规格已有下载任务'}</AlertDialogTitle><AlertDialogDescription>{duplicateCanReplace ? '新文件校验通过后会替换原下载，沿用原任务。' : duplicateCompleted ? '历史记录的规格未知，继续下载会另存一份。' : '已有同规格的下载任务，再次下载会创建另一条任务。'}{duplicate?.previous.path && <span className="mt-2 block break-all">{displayPath(duplicate.previous.path)}</span>}</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">暂不下载</AlertDialogCancel><AlertDialogAction size="sm" disabled={busy} onClick={() => { const pending = duplicate; setDuplicate(null); if (pending) void run(() => download(pending.task, pending.background, pending.previous)); }}>{duplicateCompleted && !duplicateCanReplace ? '另存一份' : '重新下载'}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <AlertDialog open={clearHistoryPlatform !== null} onOpenChange={open => { if (!open && !busy) setClearHistoryPlatform(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { (clearHistoryTrigger.current && !clearHistoryTrigger.current.disabled ? clearHistoryTrigger.current : taskListRef.current)?.focus({preventScroll: true}); }); }}><AlertDialogHeader><AlertDialogTitle>清理下载历史？</AlertDialogTitle><AlertDialogDescription>移除{(clearHistoryPlatform ?? taskPlatform) === 'all' ? '全部平台' : platformName(clearHistoryPlatform ?? taskPlatform)}中已完成、失败和已取消的下载记录，不受当前搜索条件影响。进行中、排队任务和未提交的预览会保留，不会删除任何本地文件。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm" disabled={busy}>返回</AlertDialogCancel><AlertDialogAction size="sm" disabled={busy || clearHistoryPlatform === null} onClick={event => { event.preventDefault(); void run(async () => {
      const ids = await api.clearHistory(clearHistoryPlatform === 'all' ? null : clearHistoryPlatform);
      removeTasks(ids);
      setClearHistoryPlatform(null);
      if (ids.length) toast.success(`已清理 ${ids.length} 条下载历史`); else toast.info('暂无可清理的下载历史');
    }); }}>{busy ? '清理中…' : '清理历史'}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <AlertDialog open={!!confirm} onOpenChange={open => { if (!open) setConfirm(null); }}><AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const trigger = cancelTrigger.current; (trigger?.isConnected && !trigger.matches(':disabled') ? trigger : taskListRef.current ?? inputRef.current)?.focus({preventScroll: true}); }); }}><AlertDialogHeader><AlertDialogTitle>取消当前任务？</AlertDialogTitle><AlertDialogDescription>将取消等待，或停止当前解析 / 下载并清理暂存文件。取消完成后从列表移除，其他任务不受影响。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel size="sm">返回</AlertDialogCancel><AlertDialogAction size="sm" onClick={() => { const pending = confirm; setConfirm(null); if (pending) void run(() => cancelTask(pending)); }}>确认取消</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </SidebarProvider></TooltipProvider>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
