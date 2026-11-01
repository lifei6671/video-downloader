import { useCallback, useEffect, useRef, useState } from 'react';
import { AlertCircle, Cloud, CircleCheck, CircleArrowUp, CircleArrowDown, Play, FileText, ImageOff, ListX, Trash2, X } from 'lucide-react';
import { api, errorText, type BackupAsset, type BackupOperation, type BackupSettings, type Task } from '@/api';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { Spinner } from '@/components/ui/spinner';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { Card, CardContent, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { DropdownMenuItem, DropdownMenuSeparator } from '@/components/ui/dropdown-menu';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from '@/components/ui/context-menu';
import { VideoPlayer } from '@/components/video-player';
import { VideoTranscriptDialog, useVideoTranscripts } from '@/components/video-transcript';
import type { useRuntimeDependencies } from '@/components/runtime-dependencies';
import { BackupDataDialog } from '@/components/backup-conflicts';
import { toast } from 'sonner';

export function useBackupTasks() {
  const [settings, setSettings] = useState<BackupSettings | null>(null);
  const [assets, setAssets] = useState<BackupAsset[]>([]);
  const [operations, setOperations] = useState<BackupOperation[]>([]);
  const [restoring, setRestoring] = useState<Set<string>>(new Set());
  const pollError = useRef('');
  const operationErrors = useRef<{ spaceId: string; errors: Map<string, string> } | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [revision, setRevision] = useState(0);
  const live = useRef(true);
  const currentSpace = useRef(settings?.space_id); currentSpace.current = settings?.space_id;
  useEffect(() => { setAssets([]); setOperations([]); }, [settings?.space_id]);
  useEffect(() => { live.current = true; void api.backupSettings().then(v => { if (live.current) setSettings(v); }).catch(e => { if (live.current) toast.error(errorText(e), { id: 'backup-error' }); }); return () => { live.current = false; }; }, []);
  const reload = useCallback(async () => {
    if (!settings) return;
    const [list, ops] = await Promise.all([api.backupAssets(settings.space_id), api.backupOperations()]);
    if (live.current && currentSpace.current === settings.space_id) {
      const scoped = ops.filter(o => o.space_id === settings.space_id);
      const previous = operationErrors.current?.spaceId === settings.space_id ? operationErrors.current.errors : null;
      const next = new Map<string, string>();
      for (const op of scoped) {
        if (!op.last_error || op.last_error === 'CANCELED') continue;
        const identity = `${op.last_error}:${op.attempt}`;
        next.set(op.operation_id, identity);
        // The first successful load is a history snapshot, not a new failure event.
        if (previous && previous.get(op.operation_id) !== identity) toast.error(errorText(op.last_error), { id: 'backup-error' });
      }
      operationErrors.current = { spaceId: settings.space_id, errors: next };
      setAssets(list); setOperations(scoped);
    }
  }, [settings]);
  const refresh = useCallback(async () => {
    if (!settings?.enabled) return;
    setRefreshing(true);
    try { const result = await api.refreshBackupAssets(settings.space_id); await reload(); if (live.current) { setRevision(v => v + 1); if (result.rejected.length) toast.error(`有 ${result.rejected.length} 条备份记录无法校验`, { id: 'backup-error' }); } }
    catch (e) { if (live.current) toast.error(errorText(e), { id: 'backup-error' }); }
    finally { if (live.current) setRefreshing(false); }
  }, [settings, reload]);
  useEffect(() => {
    let running = false;
    const tick = async () => { if (running) return; running = true; try { await reload(); pollError.current = ''; } catch (e) { const message = errorText(e); if (live.current && pollError.current !== message) { pollError.current = message; toast.error(message, { id: 'backup-error' }); } } finally { running = false; } };
    void tick(); const timer = setInterval(() => void tick(), 1000); return () => clearInterval(timer);
  }, [reload, refresh]);
  async function run(action: () => Promise<unknown>, message?: string) {
    try { await action(); await reload(); if (live.current && message) toast.success(message); }
    catch (e) { if (live.current && e !== 'CANCELED') toast.error(errorText(e), { id: 'backup-error' }); }
  }
  async function restore(asset: BackupAsset) {
    setRestoring(old => new Set(old).add(asset.asset_id));
    await run(() => api.ensureAsset(asset.asset_id), '本机副本已恢复');
    if (live.current) setRestoring(old => { const next = new Set(old); next.delete(asset.asset_id); return next; });
  }
  const operation = (id: string) => operations.find(o => o.asset_id === id && !['completed', 'canceled'].includes(o.state) && o.phase !== 'superseded' && !(o.kind === 'upload' && assets.find(a => a.asset_id === id)?.asset_state === 'deleted'));
  return { settings, setSettings, revision, assets, operations, operation, restoring, refreshing, refresh, reload, run, restore };
}
export type BackupTasks = ReturnType<typeof useBackupTasks>;

function isActiveTransfer(operation: BackupOperation | undefined, kind: 'upload' | 'restore') {
  return operation?.kind === kind && (kind === 'restore' ? ['pending', 'running'] : ['pending', 'running', 'retry_wait']).includes(operation.state);
}

function restoreStatus(backup: BackupTasks, assetId: string, operation: BackupOperation | undefined) {
  const restoring = backup.restoring.has(assetId) || isActiveTransfer(operation, 'restore');
  return { restoring, waiting: restoring && !(operation?.kind === 'restore' && operation.state === 'running') };
}

export function backupAssetStatus(backup: BackupTasks, asset: BackupAsset): 'queued' | 'downloading' | 'completed' | 'failed' | null {
  const operation = backup.operation(asset.asset_id);
  const { restoring, waiting } = restoreStatus(backup, asset.asset_id, operation);
  if (restoring) return waiting ? 'queued' : 'downloading';
  if (asset.local_state === 'present') return 'completed';
  if (operation?.kind === 'restore' && ['blocked', 'retry_wait'].includes(operation.state)) return 'failed';
  return null;
}

export function BackupTaskError({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const operation = backup.operation(assetId);
  if (!operation?.last_error || operation.last_error === 'CANCELED' || !['blocked', 'retry_wait'].includes(operation.state)) return null;
  return <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive"><AlertCircle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" /><span className="min-w-0 break-words">{errorText(operation.last_error)}</span></p>;
}

export function BackupAssetCoverBadge({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const asset = backup.assets.find(value => value.asset_id === assetId);
  if (!asset) return null;
  if (asset.path) return <Badge variant="success" className="task-feed-status">已完成</Badge>;
  if (asset.remote_state !== 'backed_up' || asset.remote_deleted || asset.asset_state === 'deleted') return null;
  const status = backupAssetStatus(backup, asset);
  const label = status === 'queued' ? '待下载' : status === 'downloading' ? '下载中' : status === 'failed' ? '下载失败' : '待同步';
  const variant = status === 'downloading' ? 'info' : status === 'failed' ? 'destructive' : 'warning';
  return <Badge variant={variant} className="task-feed-status">{label}</Badge>;
}

export function SyncIcon({ uploading = false, indeterminate = false, restoring = false, download = false, complete = false, progress = 0, label, failed = false }: { uploading?: boolean; indeterminate?: boolean; restoring?: boolean; download?: boolean; complete?: boolean; progress?: number; label: string; failed?: boolean }) {
  if (complete) return <CircleCheck aria-label={label} role="img" strokeWidth={1.75} className="size-3.5 text-success" />;
  const percent = Math.max(0, Math.min(100, progress));
  if (uploading || restoring && indeterminate) return indeterminate
    ? <span className="backup-sync-icon inline-flex size-3.5 text-primary" role="progressbar" aria-label={label}><Spinner aria-hidden="true" className="size-full" /></span>
    : <Progress variant="circular" className="backup-sync-icon backup-upload-progress" value={percent} aria-label={label}>{''}</Progress>;
  if (!restoring && !download) return <Cloud role="img" aria-label={label} strokeWidth={1.75} className={`backup-sync-icon size-3.5 ${failed ? 'text-destructive' : 'text-muted-foreground'}`} />;
  if (!restoring) return <CircleArrowDown role="img" aria-label={label} strokeWidth={1.75} className={`backup-sync-icon size-3.5 ${failed ? 'text-destructive' : 'text-primary'}`} />;
  const TransferIcon = CircleArrowDown;
  return <span role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className={`backup-sync-icon relative inline-flex size-3.5 ${failed ? 'text-destructive' : 'text-primary'}`}>
    <TransferIcon aria-hidden="true" strokeWidth={1.75} className="backup-sync-track absolute size-full text-muted-foreground" />
    <TransferIcon aria-hidden="true" strokeWidth={1.75} fillOpacity={0.2} className="absolute size-full [&>circle]:fill-current" style={{ clipPath: `inset(${100 - percent}% 0 0 0)` }} />
  </span>;
}

export function BackupTaskStatus({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const [busy, setBusy] = useState(false), [inspect, setInspect] = useState(false);
  const asset = backup.assets.find(a => a.asset_id === assetId), op = backup.operation(assetId);
  if (!backup.settings || !asset) return null;
  const { restoring, waiting: waitingRestore } = restoreStatus(backup, assetId, op);
  const uploading = isActiveTransfer(op, 'upload');
  const needsDownload = !!asset && !asset.path && !asset.remote_deleted && asset.remote_state === 'backed_up';
  const completeInCurrent = asset.remote_state === 'backed_up' && !!asset.path && !restoring && op?.kind !== 'upload';
  const locations = asset.backup_locations ?? [];
  const backedElsewhere = locations.some(value => value.space_id !== backup.settings?.space_id);
  const complete = !op?.last_error && (completeInCurrent || backedElsewhere && !restoring && !uploading);
  const deleting = asset?.remote_state === 'pending_delete' || asset?.remote_deleted;
  const reupload = !!asset.remote_deleted && !!asset.path;
  const retryDelete = op?.kind === 'delete' && ['blocked', 'retry_wait'].includes(op.state) && backup.settings.enabled;
  const startingUpload = busy && !restoring && !uploading && !retryDelete;
  const waitingUpload = uploading && (op?.state !== 'running' || !op.payload.transfer_started && !op.payload.completed_bytes);
  const progress = op?.payload.completed_bytes == null || !asset ? 0 : Math.min(99, Math.round(op.payload.completed_bytes / asset.content_bytes * 100));
  const label = asset?.remote_deleted && !asset.path ? '云端已删除' : restoring ? '取消下载' : uploading ? '取消上传' : deleting ? reupload ? '重新上传' : retryDelete ? '重试删除' : '远端待删除' : needsDownload ? '下载到本机' : complete ? completeInCurrent || !asset.path ? '已备份' : '上传到当前空间' : '上传备份';
  const state = restoring ? waitingRestore ? '待下载' : op?.payload.completed_bytes != null ? `正在下载 ${progress}%` : '正在下载' : startingUpload ? '正在提交上传' : uploading ? waitingUpload ? '待上传' : op?.payload.completed_bytes != null ? `备份中 ${progress}%` : '备份中' : needsDownload ? '待下载' : complete ? '已备份' : deleting ? '远端待删除' : '待上传';
  const disabled = busy || completeInCurrent || !backup.settings.enabled || (!restoring && !needsDownload && (!backup.settings.enabled || deleting && !retryDelete && !reupload || !asset?.path && !deleting));
  async function act() {
    if (disabled) return;
    if (needsDownload && !restoring && asset) { void backup.restore(asset); return; }
    setBusy(true);
    try {
      if (restoring) await backup.run(() => api.cancelAssetRestore(assetId), '下载已取消');
      else if (uploading && op) await backup.run(() => api.cancelBackup(op.operation_id), '上传已取消');
      else if (retryDelete && op && !reupload) await backup.run(() => api.retryBackup(op.operation_id), '已加入重试队列');
      else { await backup.run(() => api.startAssetBackup(backup.settings!.space_id, assetId), '已加入上传队列'); }
    } finally { setBusy(false); }
  }
  const destinations = locations.map(value => value.display_name);
  const backedUpHint = destinations.length ? `已备份至：${destinations.join('、')}` : '';
  const tooltip = complete ? backedUpHint || state : [backedUpHint, op?.last_error && !startingUpload ? `${errorText(op.last_error)}，${uploading || restoring ? '点击' : ''}${label}` : `${state}，${uploading || restoring ? '点击' : ''}${label}`].filter(Boolean).join('。');
  return <><Tooltip><TooltipTrigger asChild><span className="backup-sync-tooltip inline-flex rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" tabIndex={disabled ? 0 : undefined}><Button variant="ghost" size="icon" className="backup-sync-button shrink-0" data-complete={complete || undefined} disabled={disabled} aria-busy={busy} aria-label={label} onClick={() => void act()}><span className="backup-sync-glyph"><SyncIcon uploading={uploading || startingUpload} indeterminate={waitingUpload || startingUpload || uploading && op?.payload.completed_bytes == null || restoring && (waitingRestore || op?.payload.completed_bytes == null)} restoring={restoring} download={needsDownload} complete={complete} progress={progress} label={state} failed={!!op?.last_error} /></span></Button></span></TooltipTrigger><TooltipContent>{tooltip}</TooltipContent></Tooltip>{(asset.conflict_count > 0 || asset.remote_deleted && asset.has_pending_local_changes) && <Button variant="ghost" size="sm" className="h-6 px-1 text-xs text-destructive" onClick={() => setInspect(true)}>{asset.remote_deleted ? '本机待处理' : `冲突 ${asset.conflict_count}`}</Button>}<BackupDataDialog asset={asset} open={inspect} close={() => setInspect(false)} changed={backup.reload} /></>;
}

export function BackupTransferMenuItems({ backup, assetId, context = false }: { backup: BackupTasks; assetId: string; context?: boolean }) {
  const [busy, setBusy] = useState(false);
  const asset = backup.assets.find(a => a.asset_id === assetId), op = backup.operation(assetId);
  if (!backup.settings?.enabled || !asset || asset.remote_state === 'pending_delete' && !asset.remote_deleted) return null;
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  const uploading = isActiveTransfer(op, 'upload');
  const restoring = backup.restoring.has(assetId) || isActiveTransfer(op, 'restore');
  const transferring = uploading || restoring || op?.state === 'running';
  const canUpload = backup.settings.enabled && !!asset.path && (asset.remote_deleted || asset.remote_state !== 'backed_up') && !transferring;
  const canDownload = !asset.path && asset.remote_state === 'backed_up' && !transferring;
  async function upload() {
    if (!canUpload || busy) return;
    setBusy(true);
    try { await backup.run(() => api.startAssetBackup(asset!.space_id, assetId), '已加入上传队列'); }
    finally { setBusy(false); }
  }
  async function cancelTransfer() {
    if (busy || !uploading && !restoring) return;
    setBusy(true);
    try { await backup.run(() => restoring ? api.cancelAssetRestore(assetId) : api.cancelBackup(op!.operation_id), restoring ? '下载已取消' : '上传已取消'); }
    finally { setBusy(false); }
  }
  return <>
    {(uploading || restoring) && <Item disabled={busy} onSelect={() => void cancelTransfer()}><X />{restoring ? '取消下载' : '取消上传'}</Item>}
    <Item disabled={busy || !canUpload} onSelect={() => void upload()}><CircleArrowUp />{asset.remote_deleted ? '重新上传' : '上传云端'}</Item>
    <Item disabled={busy || !canDownload} onSelect={() => { if (canDownload && !busy) void backup.restore(asset); }}><CircleArrowDown />从远端下载</Item>
  </>;
}

function BackupAssetMenuItems({ asset, backup, transcript, context = false }: { asset: BackupAsset; backup: BackupTasks; transcript: () => void; context?: boolean }) {
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  const Separator = context ? ContextMenuSeparator : DropdownMenuSeparator;
  const hasTransfers = backup.settings?.enabled && !(asset.remote_state === 'pending_delete' && !asset.remote_deleted);
  return <>
    <Item onSelect={transcript}><FileText />查看文案</Item>
    <Separator />
    <BackupTransferMenuItems backup={backup} assetId={asset.asset_id} context={context} />
    {hasTransfers && <Separator />}
    <Item onSelect={() => void backup.run(() => api.hideAsset(asset.asset_id), '已从列表移除')}><ListX />从列表移除</Item>
    {asset.in_current_space !== false && asset.remote_state === 'backed_up' && !asset.remote_deleted && asset.path && <Item onSelect={() => void backup.run(() => api.releaseAsset(asset.asset_id), '本机副本已释放')}><Trash2 />释放本机空间</Item>}
    {asset.in_current_space !== false && backup.settings?.enabled && (backup.settings.provider_type !== 'telegram' || backup.settings.can_delete_owned_media) && !asset.remote_deleted && <Item variant="destructive" onSelect={() => void backup.run(() => api.deleteAsset(asset.space_id, asset.asset_id), backup.settings?.provider_type === 'telegram' ? '已加入删除队列' : '本机已删除，等待清理远端')}><Trash2 />从当前同步库删除</Item>}
  </>;
}

function Cover({ asset, revision }: { asset: BackupAsset; revision: number }) {
  const ref = useRef<HTMLDivElement>(null), [url, setUrl] = useState(''), [failed, setFailed] = useState(false);
  useEffect(() => {
    setUrl(''); setFailed(false);
    if (!asset.has_cover || asset.asset_state === 'deleted') return;
    let live = true, blob = '';
    const observer = new IntersectionObserver(entries => { if (!entries.some(e => e.isIntersecting)) return; observer.disconnect(); void api.assetCover(asset.space_id, asset.asset_id).then(body => { if (live) { blob = URL.createObjectURL(new Blob([body])); setUrl(blob); } }).catch(error => { if (live) { setFailed(true); toast.error(errorText(error), { id: 'backup-cover-error' }); } }); });
    if (ref.current) observer.observe(ref.current); return () => { live = false; observer.disconnect(); if (blob) URL.revokeObjectURL(blob); };
  }, [asset.space_id, asset.asset_id, asset.has_cover, asset.asset_state, asset.remote_revision, revision]);
  return <div ref={ref} className="media-thumbnail text-xs text-muted-foreground">{url ? <img src={url} alt="" className="size-full object-contain" /> : failed ? <ImageOff role="img" aria-label="封面暂不可用" className="size-4" /> : '无封面'}</div>;
}

export function useBackupDialogs() {
  const [playing, setPlaying] = useState<BackupAsset | null>(null), playTrigger = useRef<HTMLElement | null>(null);
  const transcript = useVideoTranscripts();
  return { playing, setPlaying, playTrigger, transcript, play: (asset: BackupAsset, target?: HTMLElement | null) => { playTrigger.current = target ?? null; setPlaying(asset); } };
}
export function BackupDialogs({ dialogs, backup, tasks, runtime }: { dialogs: ReturnType<typeof useBackupDialogs>; backup: BackupTasks; tasks: Task[]; runtime: ReturnType<typeof useRuntimeDependencies> }) {
  return <><VideoPlayer task={null} asset={dialogs.playing} close={() => dialogs.setPlaying(null)} returnFocus={dialogs.playTrigger.current} fallbackFocus={() => document.querySelector('[aria-current="page"]')} removedIds={new Set()} /><VideoTranscriptDialog client={dialogs.transcript} tasks={tasks} assets={backup.assets} runtime={runtime} /></>;
}
export function BackupAssetRow({ asset, backup, dialogs, feed = false }: { asset: BackupAsset; backup: BackupTasks; dialogs: ReturnType<typeof useBackupDialogs>; feed?: boolean }) {
  const row = useRef<HTMLDivElement>(null);
  const title = asset.metadata.title?.trim() || asset.original_filename || '已删除视频';
  const author = asset.metadata.author?.trim();
  const status = backupAssetStatus(backup, asset);
  const location = status === 'queued' ? '待下载' : status === 'downloading' ? '下载中' : status === 'failed' ? '下载失败' : asset.local_state === 'present' ? '本机有副本' : asset.remote_state === 'pending_delete' ? '远端待删除' : asset.remote_deleted ? '本机待处理' : '仅云端';
  const playAction = <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" className={feed ? 'task-feed-play cursor-pointer' : undefined} aria-label="播放视频" disabled={!asset.path && (asset.in_current_space === false || asset.remote_deleted)} onClick={e => dialogs.play(asset,e.currentTarget)}><Play aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>在应用内播放</TooltipContent></Tooltip>;
  return <ContextMenu><ContextMenuTrigger asChild><Card ref={row} size="sm" className="task-row task-context-target task-row-asset" data-asset-id={asset.asset_id} tabIndex={0} aria-label="远端视频操作" onKeyDown={event => {
    if (event.target !== event.currentTarget || !(event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) return;
    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: bounds.right - 24, clientY: bounds.top + bounds.height / 2 }));
  }}><CardContent className="task-row-content">
    <div className="task-row-cover"><Cover asset={asset} revision={backup.revision} />{feed && <><BackupAssetCoverBadge backup={backup} assetId={asset.asset_id} />{playAction}</>}</div>
    <div className="task-row-information"><CardTitle className="task-row-heading"><span className="task-title-backup"><BackupTaskStatus backup={backup} assetId={asset.asset_id} /></span><Tooltip><TooltipTrigger asChild><span className="task-row-title" tabIndex={0}>{title}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{title}</TooltipContent></Tooltip></CardTitle>
      <div className="task-row-byline">{author && <Tooltip><TooltipTrigger asChild><span className="task-row-author" tabIndex={0}>{author}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{author}</TooltipContent></Tooltip>}<span>{location}</span></div>
      <BackupTaskError backup={backup} assetId={asset.asset_id} />
    </div>
    {!feed && <div className="task-row-footer"><div className="task-row-controls">
      {playAction}
      <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" aria-label="查看文案" onClick={e => dialogs.transcript.open(asset.asset_id,e.currentTarget)}><FileText aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>查看文案</TooltipContent></Tooltip>
    </div></div>}
  </CardContent></Card></ContextMenuTrigger><ContextMenuContent collisionPadding={8} className="task-action-menu task-context-menu" aria-label="远端视频操作"><BackupAssetMenuItems asset={asset} backup={backup} transcript={() => dialogs.transcript.open(asset.asset_id,row.current)} context /></ContextMenuContent></ContextMenu>;
}
