import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { TabsList, TabsTrigger } from '@/components/ui/tabs';

export const taskStatuses = [['all', '全部'], ['downloading', '下载中'], ['queued', '等待中'], ['completed', '已完成'], ['failed', '失败']] as const;
export type TaskStatusFilter = typeof taskStatuses[number][0];

export function TaskStatusFilters({ value, counts, onChange }: {
  value: string;
  counts: Record<TaskStatusFilter, number>;
  onChange: (value: string) => void;
}) {
  return <ToggleGroup type="single" value={value} onValueChange={next => { if (next) onChange(next); }} aria-label="按状态筛选下载任务" className="task-status-tabs">
    {taskStatuses.map(([status, label]) => <ToggleGroupItem key={status} value={status} aria-label={label}>
      {label}<span className="task-filter-count" aria-hidden="true">{counts[status]}</span>
    </ToggleGroupItem>)}
  </ToggleGroup>;
}

export function TaskPlatformFilters() {
  return <TabsList variant="line" aria-label="按平台筛选下载任务" className="min-w-0 flex-1 justify-start">
    {[['all', '全部'], ['douyin', '抖音'], ['tiktok', 'TikTok'], ['youtube', 'YouTube'], ['telegram', 'Telegram']].map(([value, label]) => <TabsTrigger key={value} value={value}>{label}</TabsTrigger>)}
  </TabsList>;
}
