import type { BackupAsset, BackupOperation, Task } from '@/api';

export type CardStatus = {
  label: string;
  variant: 'info' | 'warning' | 'success' | 'destructive' | 'muted';
  action?: 'cancel-backup' | 'retry-backup' | 'start-backup';
  operation?: BackupOperation;
};

type Context = {
  task?: Task;
  asset?: BackupAsset;
  assetId?: string | null;
  operations?: BackupOperation[];
  spaceId?: string;
  backupEnabled?: boolean;
  waiting?: boolean;
  cancelingBackup?: boolean;
  restoring?: boolean;
};

const status = (label: string, variant: CardStatus['variant'] = 'info'): CardStatus => ({label, variant});
export const isBackupUnconfirmed = (error?: string | null) => error === 'TELEGRAM_SEND_UNCONFIRMED' || error === 'BACKUP_PUBLICATION_UNCONFIRMED';

export function backupUploadCleanup(operation: BackupOperation | undefined, operations: BackupOperation[], spaceId?: string) {
  const dependency = operation?.payload.cleanup_dependency;
  if (operation?.state === 'completed' ||
      !(dependency || operation?.state === 'canceled' && !['queued', 'cancel_cleaned'].includes(operation.phase))) return;
  const cleanup = dependency ? operations.find(op => op.operation_id === dependency && (!spaceId || op.space_id === spaceId)) : operation;
  return { error: cleanup?.payload.cleanup_error, blocked: cleanup?.payload.cleanup_blocked };
}

/** Project existing task/operation facts into a badge; never mutate their state. */
export function taskCardStatus({ task, asset, assetId, operations = [], spaceId, backupEnabled, waiting, cancelingBackup, restoring }: Context): CardStatus {
  if (waiting) return status('等待中', 'warning');
  if (task?.canceling) return status('正在取消', 'warning');
  if (task?.error === 'TELEGRAM_RECONNECTING' && ['parsing', 'queued', 'downloading'].includes(task.status)) return status('等待连接', 'warning');
  if (task && task.status !== 'completed') {
    if (task.status === 'failed') return status(task.error === 'INTERRUPTED' ? '已中断' : '下载失败', 'destructive');
    if (task.status === 'canceled') return status('已取消', 'muted');
    if (task.status === 'downloading') return status(task.merging ? '合并中' : '下载中');
    return status('等待中', 'warning');
  }
  if (cancelingBackup) return status('正在取消', 'warning');
  const id = assetId ?? asset?.asset_id ?? task?.asset_id;
  const scoped = operations.filter(op => !!id && op.asset_id === id && (!spaceId || op.space_id === spaceId) && op.phase !== 'superseded');
  const restore = scoped.find(op => op.kind === 'restore' && !['completed', 'canceled'].includes(op.state));
  if (restore?.phase === 'reconnecting' && restore.state === 'running') return status('等待连接', 'warning');
  if (restoring || restore && ['pending', 'running'].includes(restore.state)) return status(restore?.state === 'running' ? '下载中' : '等待中', restore?.state === 'running' ? 'info' : 'warning');
  if (restore && ['blocked', 'retry_wait'].includes(restore.state)) return status('下载失败', 'destructive');
  if (task?.video?.kind === 'image_album') return status('已完成', 'success');

  const uploads = scoped.filter(op => op.kind === 'upload' && !(op.state === 'blocked' && op.phase === 'cancel_cleaned'));
  const op = uploads.find(op => !['completed', 'canceled'].includes(op.state)) ?? uploads[0];
  const cleanup = backupUploadCleanup(op, operations, spaceId);
  if (cleanup) {
    if (isBackupUnconfirmed(cleanup.error)) return {...status('待确认', 'warning'), operation: op};
    if (cleanup.blocked) return {...status('备份失败', 'destructive'), action: 'start-backup', operation: op};
    return {...status('清理中'), operation: op};
  }
  if (backupEnabled === false && (!op || ['pending','completed','retry_wait'].includes(op.state)) && !isBackupUnconfirmed(op?.last_error)) return status('已完成', 'success');
  if (op && op.state !== 'completed') {
    const action = op.state === 'blocked' ? ['BACKUP_SOURCE_CHANGED','BACKUP_REUPLOAD_STALE'].includes(op.last_error ?? '') ? 'start-backup' : 'retry-backup' : op.state === 'canceled' ? 'start-backup' : 'cancel-backup';
    const result = (value: CardStatus): CardStatus => ({...value, action, operation: op});
    if (op.state === 'canceled') return result(status('备份已取消', 'muted'));
    if (isBackupUnconfirmed(op.last_error)) return result(status('待确认', 'warning'));
    if (op.state === 'blocked') return result(status('备份失败', 'destructive'));
    if (op.state === 'retry_wait' && op.last_error === 'TELEGRAM_RECONNECTING') return result(status('等待连接', 'warning'));
    if (op.state === 'retry_wait') return result(status('等待重试', 'warning'));
    if (op.state === 'pending') return result(status('待备份', 'warning'));
    if (op.state === 'running') {
      if (op.phase === 'publishing' || op.phase === 'captured' && !!asset?.content_bytes && (op.payload.completed_bytes ?? 0) >= asset.content_bytes) return result(status('确认备份'));
      if (op.phase === 'queued' || op.phase === 'captured' && !op.payload.transfer_started && !(op.payload.completed_bytes ?? 0)) return result(status('准备备份'));
      return result(status('备份中'));
    }
  }
  if (backupEnabled === true && (!spaceId || asset?.space_id === spaceId) && asset?.in_current_space !== false && asset?.remote_state === 'backed_up' && !asset.remote_deleted && !asset.has_pending_local_changes) return status('已完成', 'success');
  if (backupEnabled === false) return status('已完成', 'success');
  return {...status('待备份', 'warning'), action: asset?.path ? 'start-backup' : undefined};
}
