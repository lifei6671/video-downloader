import type { ReactNode } from 'react';
import { AlertCircle } from 'lucide-react';
import { errorText, type Task } from '@/api';
import { Card, CardContent, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';

export const labels = { parsing: '解析中', ready: '已解析，可下载', queued: '等待中', downloading: '下载中', completed: '已完成', failed: '失败', canceled: '已取消' };
export const statusVariant = { parsing: 'info', ready: 'info', queued: 'warning', downloading: 'info', completed: 'success', failed: 'destructive', canceled: 'muted' } as const;

export function TaskRow({ task, selected, onSelect, onPlay, cover, title, metadata, progress, actions, queuePosition }: {
  task: Task;
  selected: boolean;
  onSelect: () => void;
  onPlay?: () => void;
  cover: ReactNode;
  title: ReactNode;
  metadata: ReactNode;
  progress: ReactNode;
  actions: ReactNode;
  queuePosition: number;
}) {
  return <Card size="sm" data-task-id={task.id} data-status={task.status} data-selected={selected || undefined} onClick={onSelect} onFocus={onSelect} onDoubleClick={event => {
    if (event.target instanceof Element && !event.target.closest('button, a, input, select, textarea, [role="button"], [role="dialog"], [role="menuitem"]')) onPlay?.();
  }} className={`task-row${task.status === 'downloading' ? ' is-active' : ''}`}>
    <CardContent className="task-row-content">
      {cover}
      <div className="task-row-information">
        <CardTitle className="task-row-heading">{title}</CardTitle>
        {metadata}
        {progress}
        {task.status === 'queued' && !task.video && task.telegram_link && <p className="text-xs text-muted-foreground">等待 Telegram 服务空闲后自动解析并下载</p>}
        {task.error && <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive"><AlertCircle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" /><span>{errorText(task.error)}</span></p>}
        {task.cover_error && <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><AlertCircle className="size-3.5" />封面未保存</p>}
      </div>
      <div className="task-row-status">
        <Badge variant={task.canceling ? 'warning' : statusVariant[task.status]}>{task.canceling ? '正在取消…' : task.status === 'queued' && !task.video && task.telegram_link ? '等待解析' : labels[task.status]}</Badge>
        {task.status === 'queued' && <span className="task-queue-position">队列第 {queuePosition} 位</span>}
      </div>
      <div className="task-row-controls">{actions}</div>
    </CardContent>
  </Card>;
}
