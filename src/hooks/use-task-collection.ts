import { useEffect, useRef, useState } from 'react';
import type { Task } from '@/api';
import { errorText } from '@/api';
import { libraryApi, type TaskCollectionState } from '@/lib/library-api';

export function useTaskCollection(tasks: Task[]) {
  const [states, setStates] = useState<Record<string, TaskCollectionState>>({});
  const [pending, setPending] = useState<Set<string>>(new Set());
  const requests = useRef(new Map<string, Promise<TaskCollectionState>>());
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState('');
  const ids = JSON.stringify(tasks.filter(t => t.status === 'completed' && t.video?.kind !== 'image_album').map(t => t.id).sort());
  useEffect(() => {
    if (ids === '[]') return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    const refresh = () => setRevision(value => value + 1);
    void libraryApi.subscribe(refresh).then(stop => { if (disposed) stop(); else unsubscribe = stop; }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    window.addEventListener('focus', refresh);
    return () => { disposed = true; unsubscribe?.(); window.removeEventListener('focus', refresh); };
  }, [ids]);
  useEffect(() => {
    if (ids === '[]') { setStates({}); return; }
    let disposed = false;
    const taskIds: string[] = JSON.parse(ids);
    const batches: Promise<TaskCollectionState[]>[] = [];
    for (let offset = 0; offset < taskIds.length; offset += 10000) batches.push(libraryApi.taskCollectionStates(taskIds.slice(offset, offset + 10000)));
    void Promise.all(batches).then(pages => {
      const items = pages.flat();
      if (!disposed) { setStates(Object.fromEntries(items.map(item => [item.task_id, item]))); setError(''); }
    }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; };
  }, [ids, revision]);
  function setCollected(taskId: string, collected: boolean) {
    const existing = requests.current.get(taskId);
    if (existing) return existing;
    setPending(old => new Set(old).add(taskId));
    const request = libraryApi.setTaskCollected(taskId, collected).then(item => {
      setStates(old => ({ ...old, [taskId]: item }));
      return item;
    }).finally(() => {
      requests.current.delete(taskId);
      setPending(old => { const next = new Set(old); next.delete(taskId); return next; });
    });
    requests.current.set(taskId, request);
    return request;
  }

  return { states, setCollected, pending, error };
}
