import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { AlertCircle, Cloud, CircleCheck, CircleArrowUp, CircleArrowDown, Play, RotateCcw, FileText, ImageOff, ListX, Trash2, X } from 'lucide-react';
import { TaskCardFeedback, TaskCardMessage, TaskCardNotice } from '@/components/task-row';
import { bytes } from '@/lib/format-bytes';
import { taskCardStatus } from '@/lib/task-card-status';
import { api, errorText, type BackupAsset, type BackupOperation, type BackupSettings, type Task } from '@/api';
import { DeleteProgressTooltip, deleteProgressDetail, useDeleteActivity } from './delete-progress-tooltip';
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

function deletionFinished(operation: BackupOperation) {
  return operation.kind === 'delete' && operation.phase !== 'superseded' && (operation.payload.local_cleanup_done === true || operation.state === 'completed' && operation.phase === 'completed');
}

function backupFailure(operation: BackupOperation | undefined, assets: BackupAsset[], operations: BackupOperation[]) {
  if (!operation?.last_error || operation.state !== 'blocked' || operation.phase === 'superseded' || deletionFinished(operation) || operation.last_error === 'CANCELED') return null;
  // Deletion deliberately stops an upload. Preserve its cleanup evidence without
  // presenting the expected stop as an upload/restore failure.
  if (operation.kind === 'upload' && ['BACKUP_DELETE_PENDING','BACKUP_ASSET_DELETED'].includes(operation.last_error)) {
    const deleted = assets.some(asset => asset.asset_id === operation.asset_id && asset.space_id === operation.space_id && (asset.asset_state === 'deleted' || asset.in_current_space !== false && asset.remote_deleted));
    const deletes = operations.filter(op => op.space_id === operation.space_id && op.asset_id === operation.asset_id && op.kind === 'delete' && op.phase !== 'superseded');
    const deleting = deletes.some(op => !['completed','canceled'].includes(op.state) && !deletionFinished(op));
    // Once the asset leaves the list, old upload cleanup may still be polled.
    // A historical deletion alone cannot silence a new active asset generation.
    const removed = !assets.some(asset => asset.asset_id === operation.asset_id) && deletes.some(deletionFinished);
    if (deleted || deleting || removed) return null;
  }
  return operation.last_error;
}

export function indexBackup(assets: BackupAsset[], operations: BackupOperation[]) {
  const assetsById = new Map(assets.map(asset => [asset.asset_id, asset]));
  const operationsByAsset = new Map<string, BackupOperation[]>();
  for (const op of operations) {
    if (!op.asset_id) continue;
    const group = operationsByAsset.get(op.asset_id);
    if (group) group.push(op); else operationsByAsset.set(op.asset_id, [op]);
  }
  return {assetsById, operationsByAsset};
}

export function useBackupTasks() {
  const [settings, setSettings] = useState<BackupSettings | null>(null);
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [syncPreference, setSyncPreference] = useState<{settings: typeof settings; enabled: boolean} | null>(null);
  const [cancelingUploads, setCancelingUploads] = useState<Set<string>>(new Set());
  const cancelingUploadIds = useRef(new Set<string>());
  const [assets, setAssets] = useState<BackupAsset[]>([]);
  const [operations, setOperations] = useState<BackupOperation[]>([]);
  const [restoring, setRestoring] = useState<Set<string>>(new Set());
  const [submittingDeletes, setSubmittingDeletes] = useState<Set<string>>(new Set());
  const [deleteErrors, setDeleteErrors] = useState<Record<string, string>>({});
  const [exitingAssets, setExitingAssets] = useState<BackupAsset[]>([]);
  const assetSnapshot = useRef<BackupAsset[]>([]), operationSnapshot = useRef<BackupOperation[]>([]);
  const reloadSequence = useRef(0);
  const pollError = useRef('');
  const operationErrors = useRef<{ spaceId: string; errors: Map<string, string> } | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [revision, setRevision] = useState(0);
  const live = useRef(true);
  const currentSpace = useRef(settings?.space_id); currentSpace.current = settings?.space_id;
  const deleteActivity = useDeleteActivity(operations.some(operation => operation.space_id === settings?.space_id && operation.kind === 'delete' && !['completed','canceled','blocked'].includes(operation.state) && operation.phase !== 'superseded' && !deletionFinished(operation)), settings?.space_id);
  useEffect(() => { setAssets([]); setOperations([]); setExitingAssets([]); setSubmittingDeletes(new Set()); setDeleteErrors({}); assetSnapshot.current = []; operationSnapshot.current = []; }, [settings?.space_id]);
  useEffect(() => { live.current = true; void api.backupSettings().then(v => { if (live.current) { setSettings(v); setSettingsLoaded(true); } }).catch(e => { if (live.current) toast.error(errorText(e), { id: 'backup-error' }); }); return () => { live.current = false; }; }, []);
  useEffect(() => {
    if (!settings) { setSyncPreference(null); return; }
    let disposed = false;
    void api.backupSyncEnabled().then(enabled => { if (!disposed) setSyncPreference({settings,enabled}); }).catch(() => {});
    return () => { disposed = true; };
  }, [settings]);
  const backupEnabled = !settingsLoaded ? undefined : !settings ? false : syncPreference?.settings === settings ? !syncPreference.enabled ? false : settings.enabled && !settings.connection_pending ? true : undefined : undefined;
  const reload = useCallback(async (verifyLocal = true) => {
    if (!settings) return;
    const sequence = ++reloadSequence.current;
    const [list, ops, active] = await Promise.all([api.backupAssets(settings.space_id, verifyLocal), api.backupOperations(settings.space_id),
      settings.connection_pending || !settings.enabled ? api.backupSettings() : Promise.resolve(settings)]);
    if (live.current && currentSpace.current === settings.space_id && sequence === reloadSequence.current) {
      if (active?.space_id === settings.space_id && (active.enabled !== settings.enabled || active.connection_pending !== settings.connection_pending)) setSettings(active);
      const scoped = ops.filter(o => o.space_id === settings.space_id);
      const index = indexBackup(list, scoped);
      const clearingErrors = new Set(scoped.filter(op => op.kind === 'delete' && (['pending','running'].includes(op.state) || deletionFinished(op))).map(op => op.asset_id));
      setDeleteErrors(old => Object.fromEntries(Object.entries(old).filter(([id]) => !clearingErrors.has(id))));
      const finished = scoped.filter(deletionFinished);
      const finishedAssets = new Set(finished.map(op => op.asset_id));
      const previousOperations = new Map(operationSnapshot.current.map(op => [op.operation_id, op]));
      const leaving = assetSnapshot.current.filter(asset => !asset.task_id && !index.assetsById.has(asset.asset_id) && finishedAssets.has(asset.asset_id));
      const leavingIds = new Set(leaving.map(asset => asset.asset_id));
      if (leaving.length) setExitingAssets(old => [...old.filter(asset => !leavingIds.has(asset.asset_id)), ...leaving]);
      for (const operation of finished) {
        if (previousOperations.has(operation.operation_id) && !deletionFinished(previousOperations.get(operation.operation_id)!)) toast.success('视频已删除');
      }
      assetSnapshot.current = list; operationSnapshot.current = scoped;
      const previous = operationErrors.current?.spaceId === settings.space_id ? operationErrors.current.errors : null;
      const next = new Map<string, string>();
      for (const op of scoped) {
        const failure = backupFailure(op, op.asset_id && index.assetsById.has(op.asset_id) ? [index.assetsById.get(op.asset_id)!] : [], index.operationsByAsset.get(op.asset_id ?? '') ?? []);
        if (!failure) continue;
        const identity = `${failure}:${op.attempt}`;
        next.set(op.operation_id, identity);
        // The first successful load is a history snapshot, not a new failure event.
        if (previous && previous.get(op.operation_id) !== identity) toast.error(errorText(failure), { id: 'backup-error' });
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
    if (!settings) return;
    let running = false, refreshAfterRead = false, disposed = false;
    const tick = async () => {
      if (running || disposed) return;
      running = true;
      try { await reload(false); pollError.current = ''; }
      catch (e) { const message = errorText(e); if (live.current && pollError.current !== message) { pollError.current = message; toast.error(message, { id: 'backup-error' }); } }
      finally {
        running = false;
        if (refreshAfterRead) { refreshAfterRead = false; void tick(); }
      }
    };
    // A read already in progress may predate the committed deletion.
    const stop = api.onBackupAssetDeleted(() => {
      if (running) refreshAfterRead = true;
      else void tick();
    }).catch(() => undefined);
    void tick();
    const timer = setInterval(() => void tick(), 1000);
    return () => { disposed = true; clearInterval(timer); void stop.then(off => off?.()); };
  }, [settings, reload]);
  async function run(action: () => Promise<unknown>, message?: string) {
    try { await action(); await reload(); if (live.current && message) toast.success(message); }
    catch (e) { if (live.current && e !== 'CANCELED') toast.error(errorText(e), { id: 'backup-error' }); }
  }
  async function cancelUpload(id: string) {
    if (cancelingUploadIds.current.has(id)) return;
    cancelingUploadIds.current.add(id); setCancelingUploads(new Set(cancelingUploadIds.current));
    try { await run(() => api.cancelBackup(id)); }
    finally { cancelingUploadIds.current.delete(id); if (live.current) setCancelingUploads(new Set(cancelingUploadIds.current)); }
  }
  async function restore(asset: BackupAsset) {
    setRestoring(old => new Set(old).add(asset.asset_id));
    await run(() => api.ensureAsset(asset.asset_id), '本机副本已恢复');
    if (live.current) setRestoring(old => { const next = new Set(old); next.delete(asset.asset_id); return next; });
  }
  async function deleteAsset(assetId: string) {
    if (!settings?.enabled || submittingDeletes.has(assetId)) return;
    const space = settings.space_id;
    setSubmittingDeletes(old => new Set(old).add(assetId));
    setDeleteErrors(old => { const next = {...old}; delete next[assetId]; return next; });
    try { await api.deleteAsset(space, assetId); await reload(); }
    catch (error) { if (live.current && currentSpace.current === space) { setDeleteErrors(old => ({...old,[assetId]:errorText(error)})); toast.error(errorText(error), {id:'backup-error'}); } }
    finally { if (live.current && currentSpace.current === space) setSubmittingDeletes(old => { const next = new Set(old); next.delete(assetId); return next; }); }
  }
  const finishAssetRemoval = useCallback((id: string) => setExitingAssets(old => old.filter(asset => asset.asset_id !== id)), []);
  const {assetsById, operationsByAsset} = useMemo(() => indexBackup(assets, operations), [assets, operations]);
  const activeByAsset = useMemo(() => {
    const result = new Map<string, BackupOperation>();
    for (const op of operations) {
      if (op.asset_id && !result.has(op.asset_id) && !['completed', 'canceled'].includes(op.state) && op.phase !== 'superseded' && !deletionFinished(op) && !(op.kind === 'upload' && assetsById.get(op.asset_id)?.asset_state === 'deleted')) result.set(op.asset_id, op);
    }
    return result;
  }, [operations, assetsById]);
  const operation = (id: string) => activeByAsset.get(id);
  const exitingIds = useMemo(() => new Set(exitingAssets.map(asset => asset.asset_id)), [exitingAssets]);
  return { assetsById, operationsByAsset, exitingIds, settings, setSettings, settingsLoaded, deleteActivity, backupEnabled, cancelingUploads, cancelUpload, revision, assets, exitingAssets, finishAssetRemoval, operations, operation, restoring, submittingDeletes, deleteErrors, deleteAsset, refreshing, refresh, reload, run, restore };
}
export type BackupTasks = ReturnType<typeof useBackupTasks>;

export function backupDeletion(backup: BackupTasks, assetId: string) {
  const operation = (backup.operationsByAsset.get(assetId) ?? []).find(value => value.kind === 'delete' && !['completed','canceled'].includes(value.state) && value.phase !== 'superseded' && !deletionFinished(value));
  const submitting = backup.submittingDeletes.has(assetId), error = backup.deleteErrors[assetId];
  const complete = backup.exitingIds.has(assetId) || !operation && !submitting && !error && backup.assetsById.get(assetId)?.remote_state === 'pending_delete' && (backup.operationsByAsset.get(assetId) ?? []).some(deletionFinished);
  if (!complete && !operation && !submitting && !error) return null;
  const failed = !!error || !!backupFailure(operation, backup.assetsById.has(assetId) ? [backup.assetsById.get(assetId)!] : [], backup.operationsByAsset.get(assetId) ?? []);
  const phase = complete ? 'complete' : operation?.phase === 'finalizing' ? 'finalizing' : operation?.phase === 'local_cleanup' ? 'local' : 'remote';
  const reconnecting = operation?.state === 'retry_wait' && operation.last_error === 'TELEGRAM_RECONNECTING';
  const label = reconnecting ? '等待连接，恢复后继续删除' : complete ? '删除完成' : failed ? phase === 'local' ? '本地文件删除失败' : phase === 'finalizing' ? '记录移除失败' : '远端备份删除失败' : submitting && !operation ? '正在提交删除' : phase === 'local' ? '正在删除本地文件' : phase === 'finalizing' ? '正在移除记录' : operation && ['pending','retry_wait'].includes(operation.state) ? '等待删除远端备份' : '正在删除远端备份';
  return {operation, failed, phase, label, error};
}

export function BackupDeleteProgress({backup, assetId, cover = false}: {backup: BackupTasks; assetId: string; cover?: boolean}) {
  const deletion = backupDeletion(backup, assetId);
  if (!deletion) return null;
  if (cover) return <div className="task-feed-progress"><DeleteProgressTooltip deletion={deletion} activity={backup.deleteActivity}><div role="status" aria-live="polite" aria-label={deletion.label} tabIndex={0} className="backup-delete-progress"><Progress variant="circular" value={null} aria-label="删除进度" aria-valuetext={deletion.label}>{deletion.failed ? <AlertCircle aria-hidden="true" /> : deletion.phase === 'complete' ? <CircleCheck aria-hidden="true" /> : <Spinner aria-hidden="true" />}</Progress></div></DeleteProgressTooltip></div>;
  if (!deletion.failed) return null;
  return <div role="status" aria-live="polite" className="flex flex-wrap items-center gap-2 text-xs text-destructive">
    <AlertCircle aria-hidden="true" className="size-3.5" /><span>{deletion.label}</span>
    {deletion.error && <span>{deletion.error}</span>}
    <Button size="sm" variant="ghost" className="h-6 px-1 text-xs" disabled={!backup.settings?.enabled || backup.submittingDeletes.has(assetId)} onClick={() => void backup.deleteAsset(assetId)}>重试删除</Button>
  </div>;
}

function isActiveTransfer(operation: BackupOperation | undefined, kind: 'upload' | 'restore') {
  return operation?.kind === kind && (kind === 'restore' ? ['pending', 'running'] : ['pending', 'running', 'retry_wait']).includes(operation.state);
}

function restoreStatus(backup: BackupTasks, assetId: string, operation: BackupOperation | undefined) {
  const restoring = backup.restoring.has(assetId) || isActiveTransfer(operation, 'restore');
  return { restoring, waiting: restoring && !(operation?.kind === 'restore' && operation.state === 'running') };
}

export function BackupRestoreProgress({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const operation = backup.operation(assetId);
  const { restoring, waiting } = restoreStatus(backup, assetId, operation);
  if (!restoring) return null;
  const asset = backup.assetsById.get(assetId ?? '');
  const confirmed = operation?.payload.completed_bytes;
  const progress = asset && confirmed != null && asset.content_bytes > 0 ? Math.min(99, confirmed / asset.content_bytes * 100) : null;
  return <div className="task-feed-progress"><Progress variant="circular" updateIntervalMs={1000} aria-label="恢复下载进度" value={waiting ? null : progress}>{waiting ? '等待' : undefined}</Progress></div>;
}

export function backupAssetStatus(backup: BackupTasks, asset: BackupAsset): 'queued' | 'downloading' | 'completed' | 'failed' | null {
  const operation = backup.operation(asset.asset_id);
  const { restoring, waiting } = restoreStatus(backup, asset.asset_id, operation);
  if (restoring) return waiting ? 'queued' : 'downloading';
  if (asset.local_state === 'present') return 'completed';
  if (operation?.kind === 'restore' && ['blocked', 'retry_wait'].includes(operation.state)) return 'failed';
  return null;
}

export function BackupTaskError({ backup, assetId, overlay = false }: { backup: BackupTasks; assetId: string; overlay?: boolean }) {
  const deletionError = backup.deleteErrors[assetId];
  if (overlay && deletionError) return <TaskCardNotice message={deletionError} />;
  const deletion = backupDeletion(backup, assetId);
  const operation = deletion ? deletion.operation : backup.operation(assetId);
  const failure = backupFailure(operation, backup.assetsById.has(assetId) ? [backup.assetsById.get(assetId)!] : [], backup.operationsByAsset.get(assetId) ?? []);
  if (!failure) return null;
  if (overlay) return <TaskCardNotice message={errorText(failure)} />;
  return <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive"><AlertCircle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" /><span className="min-w-0 break-words">{errorText(failure)}</span></p>;
}

export function backupCardStatus(backup: BackupTasks, assetId?: string | null, task?: Task, waiting = false) {
  const asset = backup.assetsById.get(assetId ?? '');
  const operations = assetId ? (backup.operationsByAsset.get(assetId) ?? []).filter(op => (!backup.settings?.space_id || op.space_id === backup.settings.space_id)) : [];
  return taskCardStatus({task, asset, assetId, operations, spaceId: backup.settings?.space_id,
    backupEnabled: backup.backupEnabled, waiting, restoring: !!assetId && backup.restoring.has(assetId),
    cancelingBackup: operations.some(op => backup.cancelingUploads?.has(op.operation_id))});
}
export function UnifiedTaskBadge({backup, task, waiting = false}: {backup: BackupTasks; task: Task; waiting?: boolean}) {
  const deletion = task.asset_id ? backupDeletion(backup, task.asset_id) : null;
  if (task.status === 'completed' && deletion && !waiting) return <BackupAssetCoverBadge backup={backup} assetId={task.asset_id!} />;
  const state = backupCardStatus(backup, task.asset_id, task, waiting);
  return <Badge variant={state.variant} className="task-feed-status">{state.label}</Badge>;
}
export function BackupAssetCoverBadge({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const deletion = backupDeletion(backup, assetId);
  if (deletion) return <Badge variant={deletion.failed ? 'destructive' : deletion.phase === 'complete' ? 'success' : 'info'} className="task-feed-status">{deletion.failed ? '删除失败' : deletion.phase === 'complete' ? '删除完成' : '删除中'}</Badge>;
  if (!backup.assetsById.has(assetId)) return null;
  const state = backupCardStatus(backup, assetId);
  return <Badge variant={state.variant} className="task-feed-status">{state.label}</Badge>;
}
export function BackupCardAction({backup, assetId}: {backup: BackupTasks; assetId: string}) {
  const [busy, setBusy] = useState(false);
  const submitting = useRef(false);
  const state = backupCardStatus(backup, assetId);
  const op = state.operation;
  if (!state.action && !['清理中','待确认','正在取消'].includes(state.label)) return null;
  const cancel = state.action === 'cancel-backup';
  const label = cancel ? '取消备份' : state.action === 'retry-backup' ? state.label === '待确认' ? '重试确认' : '重试备份' : state.action === 'start-backup' ? ['备份已取消','备份失败'].includes(state.label) ? '重试备份' : '上传备份' : state.label;
  async function act() {
    if (submitting.current || !state.action || !cancel && (!backup.settings?.enabled || backup.backupEnabled !== true)) return;
    submitting.current = true; setBusy(true);
    try {
      if (cancel && op) await backup.cancelUpload(op.operation_id);
      else if (state.action === 'retry-backup' && op) await backup.run(() => api.retryBackup(op.operation_id));
      else await backup.run(() => api.startAssetBackup(backup.settings!.space_id, assetId));
    } finally { submitting.current = false; setBusy(false); }
  }
  const disabled = busy || !state.action || !cancel && (!backup.settings?.enabled || backup.backupEnabled !== true) || state.label === '正在取消';
  const hint = !state.action && state.label === '待确认' ? '需核对原备份回执，避免重复上传' : state.label === '清理中' ? '正在安全清理前次操作' : label;
  return <Tooltip><TooltipTrigger asChild><span className="task-feed-action-tooltip rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" tabIndex={disabled ? 0 : undefined} role={disabled ? 'status' : undefined} aria-label={disabled ? label : undefined}><Button size="icon" variant="ghost" className="task-feed-play" aria-label={label} disabled={disabled} onClick={() => void act()}>{busy ? <Spinner aria-hidden="true" /> : cancel ? <X aria-hidden="true" /> : state.action ? <RotateCcw aria-hidden="true" /> : <Spinner aria-hidden="true" />}</Button></span></TooltipTrigger><TooltipContent collisionPadding={8}>{hint}</TooltipContent></Tooltip>;

}

export function SyncIcon({ uploading = false, indeterminate = false, restoring = false, download = false, complete = false, progress = 0, label, failed = false }: { uploading?: boolean; indeterminate?: boolean; restoring?: boolean; download?: boolean; complete?: boolean; progress?: number; label: string; failed?: boolean }) {
  if (failed) return <AlertCircle aria-label={label} role="img" strokeWidth={1.75} className="size-3.5 text-destructive" />;
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

function backupTaskPresentation(backup: BackupTasks, assetId: string, busy = false) {
  const asset = backup.assetsById.get(assetId ?? '');
  if (!backup.settings || !asset) return null;
  const uploadState = backupCardStatus(backup, assetId);
  const current = backup.operation(assetId);
  const op = current && current.kind !== 'upload' ? current : uploadState.operation;
  const { restoring, waiting: waitingRestore } = restoreStatus(backup, assetId, op);
  const uploading = uploadState.action === 'cancel-backup' && isActiveTransfer(op, 'upload');
  const failure = backupFailure(op, asset ? [asset] : [], backup.operationsByAsset.get(assetId) ?? []);
  const needsDownload = !!asset && !asset.path && !asset.remote_deleted && asset.remote_state === 'backed_up';
  const completeInCurrent = asset.remote_state === 'backed_up' && !!asset.path && !restoring && uploadState.label === '已完成' && (op?.kind !== 'upload' || op.state === 'completed');
  const locations = asset.backup_locations ?? [];
  const backedElsewhere = locations.some(value => value.space_id !== backup.settings?.space_id);
  const complete = !op?.last_error && (completeInCurrent || backedElsewhere && !restoring && !uploading);
  const deleting = asset?.remote_state === 'pending_delete' || asset?.remote_deleted;
  const reupload = !!asset.remote_deleted && !!asset.path;
  const retryDelete = op?.kind === 'delete' && ['blocked', 'retry_wait'].includes(op.state) && backup.settings.enabled;
  const startingUpload = busy && !restoring && !uploading && !retryDelete;
  const waitingUpload = uploading && (op?.state !== 'running' || !op.payload.transfer_started && !op.payload.completed_bytes);
  const progress = op?.payload.completed_bytes == null || !asset ? 0 : Math.min(99, Math.round(op.payload.completed_bytes / asset.content_bytes * 100));
  const draining = !failure && ['清理中','正在取消'].includes(uploadState.label);
  const awaitingCleanup = uploadState.label === '待确认' && !uploadState.action;
  const uploadPhase = uploadState.operation?.kind === 'upload' && uploadState.operation.state !== 'completed' ? uploadState.label : null;
  const label = draining || awaitingCleanup ? uploadState.label : uploadState.label === '备份已取消' ? '重试备份' : failure && op?.kind === 'upload' ? uploadState.label === '待确认' ? '重试确认' : '重试备份' : asset?.remote_deleted && !asset.path ? '云端已删除' : restoring ? '取消下载' : uploading ? '取消上传' : deleting ? reupload ? '重新上传' : retryDelete ? '重试删除' : '远端待删除' : needsDownload ? '下载到本机' : complete ? completeInCurrent || !asset.path ? '已备份' : '上传到当前空间' : '上传备份';
  const state = uploadState.label === '等待连接' ? '正在重连 Telegram，连接恢复后自动继续' : failure ? uploadState.label : restoring ? waitingRestore ? '待下载' : op?.payload.completed_bytes != null ? `正在下载 ${progress}%` : '正在下载' : startingUpload ? '正在提交上传' : uploadPhase ? uploadPhase === '备份中' && op?.payload.completed_bytes != null ? `备份中 ${progress}%` : uploadPhase : op?.state === 'retry_wait' ? '等待重试' : needsDownload ? '待下载' : complete ? '已备份' : deleting ? '远端待删除' : '待备份';
  const disabled = busy || draining || awaitingCleanup || completeInCurrent || !backup.settings.enabled || (!restoring && !needsDownload && (!backup.settings.enabled || deleting && !retryDelete && !reupload || !asset?.path && !deleting));
  const destinations = locations.map(value => value.display_name);
  const backedUpHint = destinations.length ? `已备份至：${destinations.join('、')}` : '';
  const message = [backedUpHint, failure && !startingUpload ? errorText(failure) : complete && backedUpHint ? '' : state].filter(Boolean).join('。');
  return {asset, op, restoring, waitingRestore, uploading, failure, uploadState, needsDownload, completeInCurrent, complete, deleting, reupload, retryDelete, startingUpload, waitingUpload, draining, progress, label, state, disabled, message};
}

export function BackupTaskStatus({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const [busy, setBusy] = useState(false), [inspect, setInspect] = useState(false);
  const deletion = backupDeletion(backup, assetId);
  if (deletion) return <DeleteProgressTooltip deletion={deletion} activity={backup.deleteActivity} retryHint><span className="backup-sync-tooltip inline-flex rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" tabIndex={deletion.failed ? undefined : 0}><Button variant="ghost" size="icon" className="backup-sync-button shrink-0" disabled={!deletion.failed || busy || !backup.settings?.enabled || backup.submittingDeletes.has(assetId)} aria-label={deletion.failed ? '重试删除' : deletion.label} onClick={() => void backup.deleteAsset(assetId)}>{deletion.failed ? <Trash2 className="size-3.5 text-destructive" /> : deletion.phase === 'complete' ? <CircleCheck className="size-3.5 text-success" /> : <Spinner className="size-3.5" />}</Button></span></DeleteProgressTooltip>;
  const presentation = backupTaskPresentation(backup, assetId, busy);
  if (!presentation) return null;
  const {asset, op, restoring, waitingRestore, uploading, failure, uploadState, needsDownload, complete, reupload, retryDelete, startingUpload, waitingUpload, draining, progress, label, state, disabled} = presentation;
  async function act() {
    if (disabled) return;
    if (needsDownload && !restoring && asset) { void backup.restore(asset); return; }
    setBusy(true);
    try {
      if (restoring) await backup.run(() => api.cancelAssetRestore(assetId), '下载已取消');
      else if (uploading && op) await backup.cancelUpload(op.operation_id);
      else if (failure && uploadState.action === 'retry-backup' && op) await backup.run(() => api.retryBackup(op.operation_id));
      else if (retryDelete && op && !reupload) await backup.run(() => api.retryBackup(op.operation_id), '已加入重试队列');
      else { await backup.run(() => api.startAssetBackup(backup.settings!.space_id, assetId), '已加入上传队列'); }
    } finally { setBusy(false); }
  }
  return <><Tooltip><TooltipTrigger asChild><span className="backup-sync-tooltip inline-flex rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" tabIndex={disabled ? 0 : undefined}><Button variant="ghost" size="icon" className="backup-sync-button shrink-0" data-complete={complete || undefined} disabled={disabled} aria-busy={busy} aria-label={label} onClick={() => void act()}><span className="backup-sync-glyph"><SyncIcon uploading={uploading || startingUpload || draining} indeterminate={draining || waitingUpload || startingUpload || uploading && op?.payload.completed_bytes == null || restoring && (waitingRestore || op?.payload.completed_bytes == null)} restoring={restoring} download={needsDownload} complete={complete} progress={progress} label={state} failed={!!failure} /></span></Button></span></TooltipTrigger><TooltipContent>{label}</TooltipContent></Tooltip>{(asset.conflict_count > 0 || asset.remote_deleted && asset.has_pending_local_changes) && <Button variant="ghost" size="sm" className="h-6 px-1 text-xs text-destructive" onClick={() => setInspect(true)}>{asset.remote_deleted ? '本机待处理' : `冲突 ${asset.conflict_count}`}</Button>}<BackupDataDialog asset={asset} open={inspect} close={() => setInspect(false)} changed={backup.reload} /></>;
}

export function BackupTaskFeedback({ backup, assetId }: { backup: BackupTasks; assetId: string }) {
  const deletion = backupDeletion(backup, assetId);
  if (backup.deleteErrors[assetId] || deletion?.failed) return <BackupTaskError backup={backup} assetId={assetId} overlay />;
  if (deletion) return <TaskCardMessage message={deleteProgressDetail(deletion, backup.deleteActivity)} />;
  const presentation = backupTaskPresentation(backup, assetId);
  if (!presentation) return null;
  if (presentation.failure) return <TaskCardNotice message={presentation.message} />;
  return <TaskCardMessage message={presentation.message} />;
}

export function BackupTransferMenuItems({ backup, assetId, context = false }: { backup: BackupTasks; assetId: string; context?: boolean }) {
  const [busy, setBusy] = useState(false);
  const deletion = backupDeletion(backup, assetId);
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  if (deletion) return <Item disabled={!deletion.failed || !backup.settings?.enabled || backup.submittingDeletes.has(assetId)} onSelect={() => void backup.deleteAsset(assetId)}><Trash2 />{deletion.failed ? '重试删除' : deletion.label}</Item>;
  const asset = backup.assetsById.get(assetId ?? ''), op = backup.operation(assetId);
  if (!backup.settings?.enabled || !asset || asset.remote_state === 'pending_delete' && !asset.remote_deleted) return null;
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
    try { if (restoring) await backup.run(() => api.cancelAssetRestore(assetId), '下载已取消'); else await backup.cancelUpload(op!.operation_id); }
    finally { setBusy(false); }
  }
  return <>
    {(uploading || restoring) && <Item disabled={busy} onSelect={() => void cancelTransfer()}><X />{restoring ? '取消下载' : '取消上传'}</Item>}
    <Item disabled={busy || !canUpload} onSelect={() => void upload()}><CircleArrowUp />{asset.remote_deleted ? '重新上传' : '上传云端'}</Item>
    <Item disabled={busy || !canDownload} onSelect={() => { if (canDownload && !busy) void backup.restore(asset); }}><CircleArrowDown />从远端下载</Item>
  </>;
}

function BackupAssetMenuItems({ asset, backup, transcript, context = false, play }: { asset: BackupAsset; backup: BackupTasks; transcript: () => void; context?: boolean; play?: () => void }) {
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  const Separator = context ? ContextMenuSeparator : DropdownMenuSeparator;
  if (backupDeletion(backup, asset.asset_id)) return <BackupTransferMenuItems backup={backup} assetId={asset.asset_id} context={context} />;
  const hasTransfers = backup.settings?.enabled && !(asset.remote_state === 'pending_delete' && !asset.remote_deleted);
  return <>
    {play && <Item onSelect={play}><Play />播放视频</Item>}
    <Item onSelect={transcript}><FileText />查看文案</Item>
    <Separator />
    <BackupTransferMenuItems backup={backup} assetId={asset.asset_id} context={context} />
    {hasTransfers && <Separator />}
    <Item onSelect={() => void backup.run(() => api.hideAsset(asset.asset_id), '已从列表移除')}><ListX />从列表移除</Item>
    {asset.in_current_space !== false && asset.remote_state === 'backed_up' && !asset.remote_deleted && asset.path && <Item onSelect={() => void backup.run(() => api.releaseAsset(asset.asset_id), '本机副本已释放')}><Trash2 />释放本机空间</Item>}
    {asset.in_current_space !== false && backup.settings?.enabled && (backup.settings.provider_type !== 'telegram' || backup.settings.can_delete_owned_media) && !asset.remote_deleted && <Item variant="destructive" onSelect={() => void backup.deleteAsset(asset.asset_id)}><Trash2 />从当前同步库删除</Item>}
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
  const deletion = backupDeletion(backup, asset.asset_id);
  const cardState = backupCardStatus(backup, asset.asset_id);
  const backupAction = !!cardState.action || ['清理中','待确认','正在取消'].includes(cardState.label);
  const restoring = status === 'queued' || status === 'downloading';
  const width = Number(asset.metadata.width), height = Number(asset.metadata.height);
  const resolution = width > 0 && height > 0 ? `${Math.min(width, height)}P` : '清晰度未知';
  const location = deletion ? deletion.failed ? '删除失败' : deletion.phase === 'complete' ? '删除完成' : '删除中' : status === 'queued' ? '待下载' : status === 'downloading' ? '下载中' : status === 'failed' ? '下载失败' : asset.local_state === 'present' ? '本机有副本' : asset.remote_state === 'pending_delete' ? '远端待删除' : asset.remote_deleted ? '本机待处理' : '仅云端';
  const playDisabled = !!deletion || feed && restoring || !asset.path && (asset.in_current_space === false || asset.remote_deleted);
  const playAction = <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" className={feed ? 'task-feed-play cursor-pointer' : undefined} aria-label={feed && restoring ? '正在下载' : feed && status === 'failed' ? '重试下载' : '播放视频'} disabled={playDisabled} onClick={e => dialogs.play(asset,e.currentTarget)}>{feed && restoring ? <Spinner aria-hidden="true" /> : feed && status === 'failed' ? <RotateCcw aria-hidden="true" /> : <Play aria-hidden="true" />}</Button></TooltipTrigger><TooltipContent>{feed && restoring ? '正在下载' : feed && status === 'failed' ? '下载失败，点击重试下载' : '在应用内播放'}</TooltipContent></Tooltip>;
  return <ContextMenu><ContextMenuTrigger asChild><Card ref={row} size="sm" className="task-row task-context-target task-row-asset" data-asset-id={asset.asset_id} tabIndex={0} aria-label="远端视频操作" onKeyDown={event => {
    if (event.target !== event.currentTarget || !(event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) return;
    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: bounds.right - 24, clientY: bounds.top + bounds.height / 2 }));
  }}><CardContent className="task-row-content">
    <div className="task-row-cover"><Cover asset={asset} revision={backup.revision} />{feed && <><BackupAssetCoverBadge backup={backup} assetId={asset.asset_id} />{!deletion && (backupAction ? <BackupCardAction backup={backup} assetId={asset.asset_id} /> : playAction)}{!deletion && <BackupRestoreProgress backup={backup} assetId={asset.asset_id} />}<BackupDeleteProgress backup={backup} assetId={asset.asset_id} cover /></>}</div>
    <div className="task-row-information"><CardTitle className="task-row-heading"><span className="task-title-backup"><BackupTaskStatus backup={backup} assetId={asset.asset_id} /></span><Tooltip><TooltipTrigger asChild><span className="task-row-title" tabIndex={0}>{title}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{title}</TooltipContent></Tooltip></CardTitle>
      <div className="task-row-byline">{author && <Tooltip><TooltipTrigger asChild><span className="task-row-author" tabIndex={0}>{author}</span></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(20rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-all">{author}</TooltipContent></Tooltip>}<span>{location}</span>{feed && <div role="group" aria-label="下载信息" className="task-row-metadata flex flex-wrap items-center text-xs text-muted-foreground"><Tooltip><TooltipTrigger asChild><span tabIndex={0}>{resolution}</span></TooltipTrigger><TooltipContent>{width > 0 && height > 0 ? `${width} × ${height}` : resolution}</TooltipContent></Tooltip><Tooltip><TooltipTrigger asChild><span tabIndex={0}>{bytes(asset.content_bytes)}</span></TooltipTrigger><TooltipContent>文件大小</TooltipContent></Tooltip></div>}</div>
      {!feed && <BackupDeleteProgress backup={backup} assetId={asset.asset_id} />}
      <TaskCardFeedback><BackupTaskFeedback backup={backup} assetId={asset.asset_id} /></TaskCardFeedback>
    </div>
    {!feed && <div className="task-row-footer"><div className="task-row-controls">
      {playAction}
      <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" aria-label="查看文案" disabled={!!deletion} onClick={e => dialogs.transcript.open(asset.asset_id,e.currentTarget)}><FileText aria-hidden="true" /></Button></TooltipTrigger><TooltipContent>查看文案</TooltipContent></Tooltip>
    </div></div>}
  </CardContent></Card></ContextMenuTrigger><ContextMenuContent collisionPadding={8} className="task-action-menu task-context-menu" aria-label="远端视频操作"><BackupAssetMenuItems asset={asset} backup={backup} transcript={() => dialogs.transcript.open(asset.asset_id,row.current)} play={playDisabled ? undefined : () => dialogs.play(asset, row.current)} context /></ContextMenuContent></ContextMenu>;
}
