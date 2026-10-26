import { useRef, type ReactNode } from 'react';
import { Copy, FileText, FolderOpen, ListX, MoreHorizontal, Trash2, X } from 'lucide-react';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from '@/components/ui/context-menu';
import type { Task } from '@/api';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';

export function TaskMoreMenu({ task, busy, reveal, copy, copyPath, remove, deleteFile, cancel, transcript, children, onOpenChange }: {
  task: Task;
  children?: ReactNode;
  onOpenChange?: (open: boolean) => void;
  transcript?: { label: string; open: (returnFocus: HTMLElement | null) => void };
  cancel?: (returnFocus: HTMLElement | null) => void;
  busy: boolean;
  reveal: () => void;
  copy: () => void;
  copyPath: () => void;
  remove: () => void;
  deleteFile: () => void;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const row = useRef<HTMLDivElement>(null);
  const removalFocus = useRef<HTMLElement | null>(null);
  const list = useRef<HTMLElement | null>(null);
  const context = children !== undefined;
  const Item = context ? ContextMenuItem : DropdownMenuItem;
  const Separator = context ? ContextMenuSeparator : DropdownMenuSeparator;
  const hasFile = task.status === 'completed' && !task.file_missing;
  const hasLink = !!(task.video || task.telegram_link);
  const removable = ['completed', 'failed', 'canceled'].includes(task.status);
  const available = hasFile || hasLink || removable || !!cancel;
  const items = <>
      {hasFile && <Item disabled={busy} onSelect={reveal}><FolderOpen />打开文件夹</Item>}
      {transcript && <Item disabled={busy} onSelect={() => transcript.open(context ? row.current : trigger.current)}><FileText />{transcript.label}</Item>}
      {hasLink && <Item disabled={busy} onSelect={copy}><Copy />复制原链接</Item>}
      {task.status === 'completed' && task.path && task.video?.kind !== 'image_album' && <Item disabled={busy} onSelect={copyPath}><Copy />复制本地地址</Item>}
      {cancel && <Item disabled={busy || task.canceling} onSelect={() => cancel(context ? row.current : trigger.current)}><X />{task.canceling ? '正在取消…' : '取消下载'}</Item>}
      {removable && (hasFile || hasLink || transcript) && <Separator />}
      {removable && <Item disabled={busy} onSelect={remove}><ListX />从列表移除</Item>}
      {hasFile && <Item variant="destructive" disabled={busy} onSelect={deleteFile}>
        <Trash2 />{task.video?.kind === 'image_album' ? '删除本地图集' : '删除本地视频'}
      </Item>}
  </>;
  if (context) return <ContextMenu onOpenChange={open => {
    if (open) {
      const item = row.current?.closest('[data-task-row]');
      const neighbor = item?.nextElementSibling ?? item?.previousElementSibling;
      removalFocus.current = neighbor?.querySelector<HTMLElement>('button:not(:disabled):not([aria-disabled="true"])') ?? null;
      list.current = row.current?.closest<HTMLElement>('[role="tabpanel"]') ?? null;
    }
    onOpenChange?.(open);
  }}>
    <ContextMenuTrigger asChild disabled={!available}>
      <div ref={row} className="task-context-target" tabIndex={0} aria-label="任务操作" onContextMenu={event => { if (!available) event.preventDefault(); }} onKeyDown={event => {
        if (event.target !== event.currentTarget || !available || !(event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) return;
        event.preventDefault();
        const bounds = event.currentTarget.getBoundingClientRect();
        event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: bounds.right - 24, clientY: bounds.top + bounds.height / 2}));
      }}>{children}</div>
    </ContextMenuTrigger>
    <ContextMenuContent className="task-action-menu task-context-menu" aria-label="任务操作" onCloseAutoFocus={event => {
      event.preventDefault();
      // Do not steal focus from a dialog, an outside click or removal's neighbor.
      requestAnimationFrame(() => {
        if (document.activeElement !== document.body && !document.activeElement?.closest('[data-slot="context-menu-content"]')) return;
        const target = row.current?.isConnected && !row.current.closest('[inert]') ? row.current
          : removalFocus.current?.isConnected && !removalFocus.current.closest('[inert]') ? removalFocus.current : list.current;
        if (target?.isConnected) target.focus({preventScroll: true});
      });
    }}>{items}</ContextMenuContent>
  </ContextMenu>;
  if (!available) return null;
  return <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <Button ref={trigger} size="icon" variant="ghost" aria-label="更多操作" disabled={busy}><MoreHorizontal aria-hidden="true" /></Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent className="task-action-menu" align="end" aria-label="任务操作">{items}</DropdownMenuContent>
  </DropdownMenu>;
}
