import type { ReactNode } from 'react';
import { AlertCircle } from 'lucide-react';
import { errorText, type Task } from '@/api';
import { Card, CardContent, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { CardStatus } from '@/lib/task-card-status';

export function TaskCardNotice({ message, warning = false }: { message: string; warning?: boolean }) {
  return <Tooltip><TooltipTrigger asChild><p role="alert" tabIndex={0} className={`task-card-notice${warning ? ' is-warning' : ''}`}><AlertCircle aria-hidden="true" className="size-3.5 shrink-0" /><span>{message}</span></p></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(24rem,var(--radix-tooltip-content-available-width))] overflow-y-auto whitespace-pre-wrap break-words">{message}</TooltipContent></Tooltip>;
}

export function TaskCardFeedback({ children }: { children: ReactNode }) {
  return <div className="task-card-feedback">{children}</div>;
}

export function TaskCardMessage({ message }: { message: string }) {
  return <Tooltip><TooltipTrigger asChild><p tabIndex={0} className="task-card-message">{message}</p></TooltipTrigger><TooltipContent collisionPadding={8} className="max-h-[var(--radix-tooltip-content-available-height)] max-w-[min(24rem,var(--radix-tooltip-content-available-width))] overflow-y-auto break-words">{message}</TooltipContent></Tooltip>;
}

export const labels = { parsing: '解析中', ready: '已解析，可下载', queued: '等待中', downloading: '下载中', completed: '已完成', failed: '失败', canceled: '已取消' };
export const statusVariant = { parsing: 'info', ready: 'info', queued: 'warning', downloading: 'info', completed: 'success', failed: 'destructive', canceled: 'muted' } as const;

export function TaskRow({ task, selected, onSelect, onPlay, cover, coverBadge, coverActions, coverProgress, feedback, statusOverride, feed = false, waitingForResources = false, hidePreviousError = false, title, metadata, progress, actions, queuePosition }: {
  task: Task;
  selected: boolean;
  onSelect: () => void;
  onPlay?: () => void;
  cover: ReactNode;
  coverBadge?: ReactNode;
  coverActions?: ReactNode;
  coverProgress?: ReactNode;
  feedback?: ReactNode;
  statusOverride?: Pick<CardStatus, 'label' | 'variant'>;
  feed?: boolean;
  waitingForResources?: boolean;
  hidePreviousError?: boolean;
  title: ReactNode;
  metadata: ReactNode;
  progress: ReactNode;
  actions: ReactNode;
  queuePosition: number;
}) {
  const reconnecting = task.error === 'TELEGRAM_RECONNECTING' && ['parsing', 'queued', 'downloading'].includes(task.status);
  const visibleError = statusOverride || waitingForResources || hidePreviousError || reconnecting ? null : task.error;
  const interrupted = task.status === 'failed' && task.error === 'INTERRUPTED';
  const badgeVariant = statusOverride?.variant ?? (waitingForResources || task.canceling || reconnecting ? 'warning' : statusVariant[task.status]);
  const statusLabel = statusOverride?.label ?? (reconnecting && !task.canceling ? '等待连接' : waitingForResources ? labels.queued : task.canceling ? '正在取消…' : interrupted ? '已中断' : task.status === 'queued' && !task.video && task.telegram_link ? '等待解析' : task.status === 'downloading' && task.merging ? '合并中' : task.status === 'failed' ? '下载失败' : labels[task.status]);
  return <Card size="sm" data-task-id={task.id} data-status={task.status} data-selected={selected || undefined} onClick={onSelect} onFocus={onSelect} onDoubleClick={event => {
    if (event.target instanceof Element && !event.target.closest('button, a, input, select, textarea, [role="button"], [role="dialog"], [role="menuitem"], [role="alert"]')) onPlay?.();
  }} className={`task-row${task.status === 'downloading' ? ' is-active' : ''}`}>
    <CardContent className="task-row-content">
      <div className="task-row-cover">
        {cover}
        {feed && (!statusOverride && !waitingForResources && coverBadge ? coverBadge : <Badge variant={badgeVariant} className={`task-feed-status${!statusOverride && task.status === 'completed' ? ' task-feed-completed' : ''}`}>{statusLabel}</Badge>)}
        {coverActions}
        {coverProgress && <div className="task-feed-progress">{coverProgress}</div>}
      </div>
      <div className="task-row-information">
        <CardTitle className="task-row-heading">{title}</CardTitle>
        {metadata}
        {progress}
        <TaskCardFeedback>{(statusOverride || !waitingForResources) && <>{visibleError ? <TaskCardNotice message={errorText(visibleError)} /> : <>{task.cover_error && <TaskCardNotice message="封面未保存" warning />}{task.file_missing && <TaskCardNotice message="文件不存在" warning />}{reconnecting ? <TaskCardMessage message="正在重连 Telegram，连接恢复后自动继续" /> : feedback}{!feed && task.status === 'queued' && !task.video && task.telegram_link && <TaskCardMessage message="等待 Telegram 服务空闲后自动解析并下载" />}</>}</>}</TaskCardFeedback>
      </div>
      {!feed && <div className="task-row-footer"><div className="task-row-status">
        <Badge variant={badgeVariant}>{statusLabel}</Badge>
        {task.status === 'queued' && <span className="task-queue-position">队列第 {queuePosition} 位</span>}
      </div>
      <div className="task-row-controls">{actions}</div></div>}
    </CardContent>
  </Card>;
}
