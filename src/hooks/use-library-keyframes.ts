import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type KeyframeRequest, type KeyframeSnapshot, type LibraryKeyframe } from '@/lib/library-api';

export function useLibraryKeyframes(assetId: string, active: boolean, revision: number) {
  const [frames, setFrames] = useState<LibraryKeyframe[]>([]);
  const [operation, setOperation] = useState<KeyframeSnapshot | null>(null);
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const [connectionError, setConnectionError] = useState('');
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [refresh, setRefresh] = useState(0);
  const scope = useRef(0);
  const operationRead = useRef(0);
  const merge = useCallback((next: KeyframeSnapshot | null) => setOperation(old => old && next && old.id === next.id && old.sequence > next.sequence ? old : next), []);
  const refreshFrames = useCallback(() => setRefresh(value => value + 1), []);
  const readOperation = useCallback(async (token: number) => {
    const sequence = ++operationRead.current;
    try {
      const next = await libraryApi.keyframeOperation();
      if (token !== scope.current || sequence !== operationRead.current) return;
      merge(next); setConnectionError('');
      if (next?.asset_id === assetId && next.state === 'completed') refreshFrames();
    } catch (reason) {
      if (token === scope.current && sequence === operationRead.current) setConnectionError(errorText(reason));
    }
  }, [assetId, merge, refreshFrames]);
  useEffect(() => {
    if (!active) return;
    let disposed = false; setLoading(true);
    void libraryApi.keyframes(assetId).then(value => { if (!disposed) { setFrames(value); setReadError(''); } })
      .catch(reason => { if (!disposed) setReadError(errorText(reason)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [assetId, active, revision, refresh]);
  useEffect(() => {
    const token = ++scope.current;
    if (!active) return;
    let disposed = false; let stop: (() => void) | undefined;
    // Fetch Core's current operation on each event, so a late event from the
    // previous run cannot replace a newer run with another ID.
    const read = () => readOperation(token);
    void libraryApi.subscribeKeyframes(() => { void read(); }).then(unsubscribe => { if (disposed) unsubscribe(); else { stop = unsubscribe; void read(); } }).catch(reason => { if (!disposed) setConnectionError(errorText(reason)); });
    return () => { disposed = true; scope.current++; operationRead.current++; stop?.(); };
  }, [assetId, active, readOperation]);
  async function start(request: KeyframeRequest) {
    if (pendingRef.current) return;
    const token = scope.current; operationRead.current++; pendingRef.current = true; setPending(true); setError('');
    try {
      await libraryApi.startKeyframes(assetId, request); toast.success('已开始提取关键帧');
      if (token === scope.current) { await readOperation(token); refreshFrames(); }
    } catch (reason) { if (token === scope.current) setError(errorText(reason)); }
    finally { pendingRef.current = false; setPending(false); }
  }
  async function cancel() {
    if (!operation || pendingRef.current) return;
    const token = scope.current; operationRead.current++; pendingRef.current = true; setPending(true); setError('');
    try { await libraryApi.cancelKeyframes(operation.id); if (token === scope.current) await readOperation(token); }
    catch (reason) { if (token === scope.current) setError(errorText(reason)); }
    finally { pendingRef.current = false; setPending(false); }
  }
  const busy = pending || operation?.state === 'running';
  return { frames, operation, loading, busy, pending, error: error || readError || connectionError, setError, start, cancel, refresh: refreshFrames };
}
