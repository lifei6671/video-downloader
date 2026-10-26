import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import type { Task } from '@/api';
import { LayoutGrid, List } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { Tabs, TabsContent } from '@/components/ui/tabs';
import { TaskPlatformFilters } from '@/components/task-filters';
import { TaskRemovalRow } from '@/components/task-removal-row';
import { Spinner } from '@/components/ui/spinner';

export type TaskView = 'list' | 'feed';

export function TaskList({ tasks, paginationKey, platform, onPlatformChange, view, onViewChange, listRef, removingIds, onExited, renderRow, empty }: {
  tasks: Task[];
  paginationKey: string;
  platform: string;
  onPlatformChange: (value: string) => void;
  view: TaskView;
  onViewChange: (view: TaskView) => void;
  listRef: RefObject<HTMLDivElement | null>;
  removingIds: Set<string>;
  onExited: (id: string) => void;
  renderRow: (task: Task) => ReactNode;
  empty: ReactNode;
}) {
  const [page, setPage] = useState({ key: paginationKey, limit: 30 });
  const [loading, setLoading] = useState(false);
  const sentinel = useRef<HTMLDivElement>(null);
  const limit = page.key === paginationKey ? page.limit : 30;
  const hasMore = tasks.length > limit;
  useLayoutEffect(() => {
    setPage({ key: paginationKey, limit: 30 });
    setLoading(false);
    listRef.current?.scrollTo(0, 0);
  }, [paginationKey, listRef]);
  useEffect(() => {
    setLoading(false);
    if (!hasMore || !sentinel.current || !listRef.current) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(entry => entry.isIntersecting) || timer !== undefined) return;
      setLoading(true);
      // Leave a short painted loading state before mounting the next batch.
      timer = setTimeout(() => {
        setPage({ key: paginationKey, limit: limit + 10 });
        setLoading(false);
      }, 180);
    }, { root: listRef.current, rootMargin: '0px 0px 500px 0px' });
    observer.observe(sentinel.current);
    return () => { observer.disconnect(); clearTimeout(timer); };
  }, [hasMore, limit, paginationKey, platform, view, listRef]);
  return <Tabs value={platform} onValueChange={onPlatformChange} className="task-browser min-h-0 flex-1">
    <div className="task-platform-toolbar">
      <TaskPlatformFilters />
      <Tooltip><TooltipTrigger asChild><Button type="button" size="icon" variant="ghost" className="task-view-toggle shrink-0 cursor-pointer" aria-label={view === 'list' ? '切换为信息流' : '切换为列表'} aria-pressed={view === 'feed'} onClick={() => onViewChange(view === 'list' ? 'feed' : 'list')}>
        {view === 'list' ? <LayoutGrid aria-hidden="true" /> : <List aria-hidden="true" />}
      </Button></TooltipTrigger><TooltipContent>{view === 'list' ? '切换为信息流' : '切换为列表'}</TooltipContent></Tooltip>
    </div>
    <TabsContent ref={listRef} value={platform} className="flex min-h-0 flex-col overflow-y-auto">
      {tasks.length ? <div role="list" aria-label="媒体任务列表" data-view={view} className="task-list">
        {tasks.slice(0, limit).map(task => <TaskRemovalRow key={task.id} id={task.id} removing={removingIds.has(task.id)} onExited={onExited}>{renderRow(task)}</TaskRemovalRow>)}
      </div> : empty}
      {hasMore && <div ref={sentinel} className="task-load-more" role="status" aria-live="polite" aria-busy={loading}>
        {loading && <><Spinner aria-hidden="true" />正在加载更多…</>}
      </div>}
    </TabsContent>
  </Tabs>;
}
