import { useCallback, useEffect, useRef, useState } from 'react';
import { Cloud, CircleCheck, CircleArrowUp, CircleArrowDown, Play, FileText, MoreHorizontal, ImageOff } from 'lucide-react';
import { api, errorText, type BackupAsset, type BackupOperation, type BackupSettings, type Task } from '@/api';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { Spinner } from '@/components/ui/spinner';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { Card, CardContent, CardTitle } from '@/components/ui/card';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from '@/components/ui/context-menu';
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
  const operation = (id: string) => operations.find(o => o.asset_id === id && o.state !== 'completed' && o.phase !== 'superseded' && !(o.kind === 'upload' && assets.find(a => a.asset_id === id)?.asset_state === 'deleted'));
  return { settings, setSettings, revision, assets, operations, operation, restoring, refreshing, refresh, reload, run, restore };
}
export type BackupTasks = ReturnType<typeof useBackupTasks>;

export function SyncIcon({ uploading = false, indeterminate = false, restoring = false, download = false, complete = false, progress = 0, label, failed = false }: { uploading?: boolean; indeterminate?: boolean; restoring?: boolean; download?: boolean; complete?: boolean; progress?: number; label: string; failed?: boolean }) {
  if (complete) return <CircleCheck aria-label={label} role="img" strokeWidth={1.75} className="size-3.5 text-success" />;
  const percent = Math.max(0, Math.min(100, progress));
  if (uploading) return indeterminate
    ? <span className="backup-sync-icon inline-flex size-3.5 text-primary" role="progressbar" aria-label={label}><Spinner aria-hidden="true" className="size-full" /></span>
    : <Progress variant="circular" className="backup-sync-icon backup-upload-progress" value={percent} aria-label={label}>{''}</Progress>;
  if (!restoring && !download) return <Cloud role="img" aria-label={label} strokeWidth={1.75} className={`backup-sync-icon size-3.5 ${failed ? 'text-destructive' : 'text-muted-foreground'}`} />;
  const TransferIcon = CircleArrowDown;
  return <span role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className={`backup-sync-icon relative inline-flex size-3.5 ${failed ? 'text-destructive' : 'text-primary'}`}>
    <TransferIcon aria-hidden="true" strokeWidth={1.75} className="backup-sync-track absolute size-full text-muted-foreground" />
    <TransferIcon aria-hidden="true" strokeWidth={1.75} fillOpacity={0.2} className="absolute size-full [&>circle]:fill-current" style={{ clipPath: `inset(${100 - percent}% 0 0 0)` }} />
  </span>;
}

export function BackupTaskStatus({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const [busy, setBusy] = useState(false), [inspect, setInspect] = useState(false);
  const [uploadRequested, setUploadRequested] = useState(false);
  const asset = backup.assets.find(a => a.asset_id === assetId), op = backup.operation(assetId);
  if (!backup.settings || !asset) return null;
  const restoring = backup.restoring.has(assetId) || op?.kind === 'restore' && op.state === 'running';
  const uploading = op?.kind === 'upload' && (op.state === 'running' || uploadRequested && op.state === 'pending');
  const needsDownload = !!asset && !asset.path && !asset.remote_deleted && asset.remote_state === 'backed_up';
  const complete = asset?.remote_state === 'backed_up' && !!asset.path && !restoring && op?.kind !== 'upload';
  const deleting = asset?.remote_state === 'pending_delete' || asset?.remote_deleted;
  const reupload = !!asset.remote_deleted && !!asset.path;
  const retryDelete = op?.kind === 'delete' && ['blocked', 'retry_wait'].includes(op.state) && backup.settings.enabled;
  const startingUpload = busy && !restoring && !uploading && !retryDelete;
  const waitingUpload = uploading && (op?.state !== 'running' || !op.payload.completed_bytes);
  const progress = op?.payload.completed_bytes == null || !asset ? 0 : Math.min(99, Math.round(op.payload.completed_bytes / asset.content_bytes * 100));
  const label = asset?.remote_deleted && !asset.path ? '云端已删除' : restoring ? '取消下载' : uploading ? '取消上传' : deleting ? reupload ? '重新上传' : retryDelete ? '重试删除' : '远端待删除' : needsDownload ? '下载到本机' : complete ? '已备份' : '上传备份';
  const state = restoring ? `正在下载 ${progress}%` : startingUpload ? '正在提交上传' : uploading ? waitingUpload ? '等待上传' : `正在上传 ${progress}%` : needsDownload ? '待下载' : complete ? '已备份' : deleting ? '远端待删除' : '待上传';
  const disabled = busy || complete || !backup.settings.enabled || (!restoring && !needsDownload && (!backup.settings.enabled || deleting && !retryDelete && !reupload || !asset?.path && !deleting));
  async function act() {
    if (disabled) return;
    if (needsDownload && !restoring && asset) { void backup.restore(asset); return; }
    setBusy(true);
    try {
      if (restoring) await backup.run(() => api.cancelAssetRestore(assetId), '下载已取消');
      else if (uploading && op) await backup.run(() => api.cancelBackup(op.operation_id), '上传已取消');
      else if (retryDelete && op && !reupload) await backup.run(() => api.retryBackup(op.operation_id), '已加入重试队列');
      else { setUploadRequested(true); await backup.run(() => api.startAssetBackup(backup.settings!.space_id, assetId), '已加入上传队列'); }
    } finally { setBusy(false); }
  }
  const tooltip = op?.last_error && !startingUpload ? `${errorText(op.last_error)}，${label}` : complete ? state : `${state}，${label}`;
  return <><Tooltip><TooltipTrigger asChild><span className="backup-sync-tooltip inline-flex rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" tabIndex={disabled ? 0 : undefined}><Button variant="ghost" size="icon" className="backup-sync-button shrink-0" data-complete={complete || undefined} disabled={disabled} aria-busy={busy} aria-label={label} onClick={() => void act()}><span className="backup-sync-glyph"><SyncIcon uploading={uploading || startingUpload} indeterminate={waitingUpload || startingUpload} restoring={restoring} download={needsDownload} complete={complete} progress={progress} label={state} failed={!!op?.last_error} /></span></Button></span></TooltipTrigger><TooltipContent>{tooltip}</TooltipContent></Tooltip>{(asset.conflict_count > 0 || asset.remote_deleted && asset.has_pending_local_changes) && <Button variant="ghost" size="sm" className="h-6 px-1 text-xs text-destructive" onClick={() => setInspect(true)}>{asset.remote_deleted ? '本机待处理' : `冲突 ${asset.conflict_count}`}</Button>}<BackupDataDialog asset={asset} open={inspect} close={() => setInspect(false)} changed={backup.reload} /></>;
}

export function BackupTransferMenuItems({ backup, assetId, context = false }: { backup: BackupTasks; assetId: string; context?: boolean }) {
  const [busy, setBusy] = useState(false);
  const asset = backup.assets.find(a => a.asset_id === assetId), op = backup.operation(assetId);
  if (!backup.settings?.enabled || !asset || asset.remote_state === 'pending_delete' && !asset.remote_deleted) return null;
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  const transferring = backup.restoring.has(assetId) || op?.state === 'running';
  const canUpload = backup.settings.enabled && !!asset.path && (asset.remote_deleted || asset.remote_state !== 'backed_up') && !transferring;
  const canDownload = !asset.path && asset.remote_state === 'backed_up' && !transferring;
  async function upload() {
    if (!canUpload || busy) return;
    setBusy(true);
    try { await backup.run(() => api.startAssetBackup(asset!.space_id, assetId), '已加入上传队列'); }
    finally { setBusy(false); }
  }
  return <>
    <Item disabled={busy || !canUpload} onSelect={() => void upload()}><CircleArrowUp />{asset.remote_deleted ? '重新上传' : '上传云端'}</Item>
    <Item disabled={busy || !canDownload} onSelect={() => { if (canDownload && !busy) void backup.restore(asset); }}><CircleArrowDown />从远端下载</Item>
  </>;
}

function BackupAssetMenuItems({ asset, backup, context = false }: { asset: BackupAsset; backup: BackupTasks; context?: boolean }) {
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  return <>
    <BackupTransferMenuItems backup={backup} assetId={asset.asset_id} context={context} />
    {asset.path && <Item onSelect={() => void backup.run(() => api.releaseAsset(asset.asset_id), '本机副本已释放')}>释放本机空间</Item>}
    {backup.settings?.enabled && backup.settings.enabled && !asset.remote_deleted && <Item variant="destructive" onSelect={() => void backup.run(() => api.deleteAsset(asset.space_id, asset.asset_id), '本机已删除，等待清理远端')}>从当前同步库删除</Item>}
    <Item onSelect={() => void backup.run(() => api.hideAsset(asset.asset_id), '已从列表移除')}>从列表移除</Item>
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
  return <div ref={ref} className="flex size-full min-h-16 items-center justify-center bg-muted text-xs text-muted-foreground">{url ? <img src={url} alt="" className="size-full object-contain" /> : failed ? <ImageOff role="img" aria-label="封面暂不可用" className="size-4" /> : '无封面'}</div>;
}

export function useBackupDialogs() {
  const [playing, setPlaying] = useState<BackupAsset | null>(null), playTrigger = useRef<HTMLElement | null>(null);
  const transcript = useVideoTranscripts();
  return { playing, setPlaying, playTrigger, transcript, play: (asset: BackupAsset, target?: HTMLElement | null) => { playTrigger.current = target ?? null; setPlaying(asset); } };
}
export function BackupDialogs({ dialogs, backup, tasks, runtime }: { dialogs: ReturnType<typeof useBackupDialogs>; backup: BackupTasks; tasks: Task[]; runtime: ReturnType<typeof useRuntimeDependencies> }) {
  return <><VideoPlayer task={null} asset={dialogs.playing} close={() => dialogs.setPlaying(null)} returnFocus={dialogs.playTrigger.current} fallbackFocus={() => document.querySelector('[aria-current="page"]')} removedIds={new Set()} /><VideoTranscriptDialog client={dialogs.transcript} tasks={tasks} assets={backup.assets} runtime={runtime} /></>;
}
export function BackupAssetRow({ asset, backup, dialogs, feed }: { asset: BackupAsset; backup: BackupTasks; dialogs: ReturnType<typeof useBackupDialogs>; feed: boolean }) {
  return <ContextMenu><ContextMenuTrigger asChild><Card size="sm" className="task-row task-context-target" data-asset-id={asset.asset_id} tabIndex={0} aria-label="远端视频操作" onKeyDown={event => {
    if (event.target !== event.currentTarget || !(event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) return;
    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: bounds.right - 24, clientY: bounds.top + bounds.height / 2 }));
  }}><CardContent className="task-row-content">
    <div className="task-row-cover"><Cover asset={asset} revision={backup.revision} /></div>
    <div className="task-row-information"><CardTitle className="task-row-heading"><span className="task-title-backup"><BackupTaskStatus backup={backup} assetId={asset.asset_id} /></span><Tooltip><TooltipTrigger asChild><span className="task-row-title" tabIndex={0}>{asset.metadata.title || asset.original_filename || '已删除视频'}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{asset.metadata.title || asset.original_filename || '已删除视频'}</TooltipContent></Tooltip></CardTitle>
      <div className="task-row-byline">{asset.metadata.author && <Tooltip><TooltipTrigger asChild><span tabIndex={0}>{asset.metadata.author}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{asset.metadata.author}</TooltipContent></Tooltip>}<span>{asset.local_state === 'present' ? '本机有副本' : asset.remote_state === 'pending_delete' ? '远端待删除' : asset.remote_deleted ? '本机待处理' : '仅云端'}</span></div>
    </div>
    <div className={feed ? 'flex justify-end gap-1 px-3 pb-3' : 'task-row-footer'}><div className="task-row-controls">
      <Button variant="ghost" size="icon" aria-label="播放视频" disabled={asset.remote_deleted && !asset.path} onClick={e => dialogs.play(asset,e.currentTarget)}><Play /></Button>
      <Button variant="ghost" size="icon" aria-label="查看文案" onClick={e => dialogs.transcript.open(asset.asset_id,e.currentTarget)}><FileText /></Button>
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="icon" aria-label="更多操作"><MoreHorizontal /></Button></DropdownMenuTrigger><DropdownMenuContent align="end">
        <BackupAssetMenuItems asset={asset} backup={backup} />
      </DropdownMenuContent></DropdownMenu>
    </div></div>
  </CardContent></Card></ContextMenuTrigger><ContextMenuContent className="task-action-menu task-context-menu" aria-label="远端视频操作"><BackupAssetMenuItems asset={asset} backup={backup} context /></ContextMenuContent></ContextMenu>;
}
