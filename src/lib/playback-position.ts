import type { Task } from '@/api';

const key = (id: string) => `yingqu.playback-position.${id}`;

export function readPlaybackPosition(task: Task): number {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key(task.id)) ?? 'null');
    if (!value || typeof value !== 'object') return 0;
    const entry = value as Record<string, unknown>;
    return entry.path === task.path && entry.completedAt === task.completed_at
      && typeof entry.seconds === 'number' && Number.isFinite(entry.seconds) && entry.seconds > 0
      ? entry.seconds : 0;
  } catch { return 0; }
}

export function writePlaybackPosition(task: Task, seconds: number) {
  if (seconds <= 0) localStorage.removeItem(key(task.id));
  else localStorage.setItem(key(task.id), JSON.stringify({ path: task.path, completedAt: task.completed_at, seconds }));
}

export function forgetPlaybackPositions(ids: string[]) {
  for (const id of ids) localStorage.removeItem(key(id));
}
