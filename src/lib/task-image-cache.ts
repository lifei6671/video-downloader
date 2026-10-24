import { useEffect, useState } from 'react';
import { api, type Task } from '@/api';

type Entry = { taskId: string; source?: string; bytes: number; users: number; obsolete: boolean; ready: Promise<string> };
const images = new Map<string, Entry>();
// Keep recently viewed images across page switches; mounted images stay valid.
function trim() {
  const idle = [...images].filter(([, entry]) => entry.users === 0);
  let count = idle.length;
  let bytes = idle.reduce((sum, [, entry]) => sum + entry.bytes, 0);
  for (const [key, entry] of idle) {
    if (!entry.obsolete && count <= 128 && bytes <= 64 * 1024 * 1024) continue;
    images.delete(key);
    if (entry.source) URL.revokeObjectURL(entry.source);
    count--; bytes -= entry.bytes;
  }
}

export function forgetTaskImages(ids: string[]) {
  const removed = new Set(ids);
  for (const entry of images.values()) if (removed.has(entry.taskId)) entry.obsolete = true;
  trim();
}

export function useTaskImage(task: Task, index?: number) {
  const filename = index === undefined ? task.cover_path : task.files[index]?.filename ?? task.video?.images?.[index]?.preview_filename;
  const local = index !== undefined || !!(task.cover_path || task.cover_cached);
  const key = local ? JSON.stringify([task.id, index ?? 'cover', filename, task.cached_at,
    index === undefined ? task.video?.cover_url : task.status === 'completed',
    index === undefined ? task.cover_cached : task.path]) : undefined;
  const [state, setState] = useState<{ key?: string; source?: string; failed?: boolean }>({});
  useEffect(() => {
    if (!key) return;
    let disposed = false;
    let entry = images.get(key);
    if (!entry) {
      const next: Entry = { taskId: task.id, users: 0, obsolete: false, bytes: 0, ready: Promise.resolve('') };
      next.ready = (index === undefined ? api.cover(task.id) : api.image(task.id, index)).then(data => {
        const bytes = new Uint8Array(data);
        const extension = filename?.split('.').pop()?.toLowerCase() ?? '';
        const mime = index === undefined ? (bytes[0] === 0xff ? 'image/jpeg' : bytes[0] === 0x89 ? 'image/png' : 'image/webp')
          : ({ jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', webp: 'image/webp', gif: 'image/gif', avif: 'image/avif', heic: 'image/heic' } as Record<string, string>)[extension];
        const source = URL.createObjectURL(new Blob([bytes], { type: mime }));
        next.source = source; next.bytes = bytes.byteLength;
        // A pending read may finish after its unused entry was evicted.
        if (images.get(key) !== next) URL.revokeObjectURL(source);
        else trim();
        return source;
      });
      entry = next;
    }
    const current = entry;
    current.users++;
    images.delete(key); images.set(key, current);
    setState({ key, source: current.source });
    current.ready.then(source => { if (!disposed) setState({ key, source }); }).catch(() => {
      if (images.get(key) === current) images.delete(key);
      if (!disposed) setState({ key, failed: true });
    });
    return () => { disposed = true; current.users--; trim(); };
  }, [key]); // The key includes every input used by the controlled image read.
  return state.key === key ? state : { source: key ? images.get(key)?.source : undefined, failed: false };
}
