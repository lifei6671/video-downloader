import { useEffect, useState, type ReactElement } from 'react';
import { api, type DeleteActivity } from '@/api';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { backupDeletion } from './backup-tasks';

type Deletion = NonNullable<ReturnType<typeof backupDeletion>>;

const steps: Record<string, string> = {
  wait_writer: '正在等待备份写入完成',
  publish: '正在发布删除记录',
  publish_locate: '正在定位备份索引',
  publish_upload: '正在上传删除索引',
  publish_send: '正在发送删除索引',
  publish_receipt: '正在核对索引发送回执',
  publish_refresh: '正在读取发布后的索引',
  session_ready: '正在准备 Telegram 会话',
  session_lock: '正在等待 Telegram 会话',
  session_reconnect: '正在检查 Telegram 连接',
  account_check: '正在核对 Telegram 账号',
  channel_lookup: '正在定位备份频道',
  channel_by_id: '正在按 ID 定位备份频道',
  channel_dialogs: '正在从会话列表查找备份频道',
  channel_read_probe: '正在检查备份频道的读取权限',
  app_config: '正在获取上传限制',
  app_config_reuse: '正在复用已确认的上传限制',
  channel_changes: '正在获取频道变更',
  history: '正在扫描频道历史',
  by_id: '正在按 ID 读取索引消息',
  index_download: '正在下载索引正文',
  remote_cleanup: '正在清理远端备份',
  locate_messages: '正在定位待清理的消息',
  read_index: '正在读取索引并核对删除记录',
  archive: '正在核对归档',
  archive_upload: '正在上传新归档',
  archive_send: '正在发送新归档',
  archive_receipt: '正在核对归档发送回执',
  archive_refresh: '正在读取最新归档索引',
  archive_confirm: '正在确认远端归档',
  archive_identity_check: '正在按 ID 核对归档附件',
  archive_body_download: '正在重新下载归档正文',
  archive_replay: '正在重放归档，确认删除记录',
  archive_delete_old: '正在清理旧索引',
  read_old_index: '正在回读核对旧索引',
  delete_old_index: '正在删除旧索引消息',
  confirm_old_index_gone: '正在确认旧索引已删除',
  prepare_cleanup: '正在核对待删除的视频',
  remove_messages: '正在清理远端视频消息',
  read_owned_message: '正在核对远端视频消息',
  delete_message: '正在发送视频消息删除请求',
  confirm_message_gone: '正在确认视频消息已删除',
  verify: '正在复查远端删除结果',
  local_cleanup: '正在删除本地文件',
  finalizing: '正在移除本地记录',
};

const batchSteps: Record<string, string> = {
  remove_messages: '正在清理同批视频的远端消息',
  read_owned_message: '正在核对同批视频的远端消息',
  delete_message: '正在批量删除远端视频消息',
  confirm_message_gone: '正在确认同批视频消息均已删除',
};

function detail(deletion: Deletion, activity: DeleteActivity | null, retryHint: boolean) {
  const { operation, label, failed, phase } = deletion;
  if (failed) return retryHint ? label + '，点击重试删除' : label;
  if (!operation || phase === 'complete') return label;
  if (activity && [activity.operation_id, activity.companion].includes(operation.operation_id)) {
    if (activity.companion && batchSteps[activity.step]) return batchSteps[activity.step];
    if (steps[activity.step]) return steps[activity.step];
  }
  if (operation.phase === 'archive_ready') {
    return operation.payload.archive_next ? '归档已确认，等待同批清理完成' : '归档已确认，等待继续清理';
  }
  if (operation.state === 'running' && operation.phase === 'tombstone') return steps.publish;
  if (operation.state === 'running' && operation.phase === 'objects') return steps.remote_cleanup;
  return label;
}

export function DeleteProgressTooltip({ deletion, retryHint = false, children }: {
  deletion: Deletion; retryHint?: boolean; children: ReactElement;
}) {
  const [open, setOpen] = useState(false);
  const [activity, setActivity] = useState<DeleteActivity | null>(null);
  const operation = deletion.operation?.operation_id;
  const active = open && !!operation && !deletion.failed && deletion.phase !== 'complete';
  useEffect(() => {
    setActivity(null);
    if (!active) return;
    let disposed = false, received = false;
    let stop: (() => void) | undefined;
    const subscribe = async () => {
      const off = await api.onDeleteActivity(value => {
        received = true;
        if (!disposed) setActivity(value);
      });
      if (disposed) { off(); return; }
      stop = off;
      const snapshot = await api.deleteActivity();
      if (!disposed && !received) setActivity(snapshot);
    };
    // The persisted operation phase remains visible if the live IPC connection is unavailable.
    void subscribe().catch(() => {});
    return () => { disposed = true; stop?.(); };
  }, [active, operation, deletion.operation?.attempt]);
  return <Tooltip onOpenChange={value => { setOpen(value); if (!value) setActivity(null); }}>
    <TooltipTrigger asChild>{children}</TooltipTrigger>
    <TooltipContent collisionPadding={8}>{detail(deletion, activity, retryHint)}</TooltipContent>
  </Tooltip>;
}
