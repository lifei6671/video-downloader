import type { ReactNode, Ref } from 'react';
import type { Task } from '@/api';
import { Tabs, TabsContent } from '@/components/ui/tabs';
import { TaskPlatformFilters } from '@/components/task-filters';
import { TaskRemovalRow } from '@/components/task-removal-row';

export function TaskList({ tasks, platform, onPlatformChange, listRef, removingIds, onExited, renderRow, empty }: {
  tasks: Task[];
  platform: string;
  onPlatformChange: (value: string) => void;
  listRef: Ref<HTMLDivElement>;
  removingIds: Set<string>;
  onExited: (id: string) => void;
  renderRow: (task: Task) => ReactNode;
  empty: ReactNode;
}) {
  return <Tabs value={platform} onValueChange={onPlatformChange} className="task-browser min-h-0 flex-1">
    <TaskPlatformFilters />
    <TabsContent ref={listRef} value={platform} className="flex min-h-0 flex-col overflow-y-auto">
      {tasks.length ? <div role="list" aria-label="媒体任务列表" className="task-list">
        {tasks.map(task => <TaskRemovalRow key={task.id} id={task.id} removing={removingIds.has(task.id)} onExited={onExited}>{renderRow(task)}</TaskRemovalRow>)}
      </div> : empty}
    </TabsContent>
  </Tabs>;
}
