import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryTransferSnapshot } from '@/lib/library-api';

export function useLibraryTransfer() {
  const [snapshot, setSnapshot] = useState<LibraryTransferSnapshot | null>(null);
  const latest = useRef<LibraryTransferSnapshot | null>(null);
  const announced = useRef(new Set<string>());
  const selecting = useRef(false);
  const [picking, setPicking] = useState(false);
  const [pickingKind, setPickingKind] = useState<'backup' | 'restore' | null>(null);
  const [canceling, setCanceling] = useState(false);
  const [error, setError] = useState('');
  const [connectionError, setConnectionError] = useState('');
  const [readError, setReadError] = useState('');
  const [notice, setNotice] = useState('');
  const [connection, setConnection] = useState(0);
  const merge = useCallback((next: LibraryTransferSnapshot | null, announce = true) => {
    if (!next) return;
    const old = latest.current;
    if (old && (old.order > next.order || old.order === next.order && old.sequence > next.sequence)) return;
    if (!old || next.order > old.order || next.state !== 'running') { setError(''); setNotice(''); }
    latest.current = next; setSnapshot(next);
    if (next.state !== 'running' && announce && !announced.current.has(next.id)) {
      announced.current.add(next.id);
      if (next.state === 'completed') toast.success(next.kind === 'backup' ? '备份完成' : '恢复完成');
      else if (next.state === 'failed') toast.error(errorText(next.error));
      else toast.info('操作已取消');
    }
  }, []);
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined; setConnectionError('');
    const read = () => { void libraryApi.transfer().then(value => { if (!disposed) { setReadError(''); merge(value, false); } }).catch(reason => { if (!disposed) setReadError(errorText(reason)); }); };
    void libraryApi.subscribeTransfer(value => { if (!disposed) { setConnectionError(''); merge(value); } }).then(unsubscribe => {
      if (disposed) unsubscribe(); else { stop = unsubscribe; setConnectionError(''); read(); }
    }).catch(reason => { if (!disposed) setConnectionError(errorText(reason)); });
    read(); window.addEventListener('focus', read);
    return () => { disposed = true; stop?.(); window.removeEventListener('focus', read); };
  }, [connection, merge]);
  async function start(kind: 'backup' | 'restore', includeMedia = false) {
    if (selecting.current || latest.current?.state === 'running') return;
    selecting.current = true; setPicking(true); setPickingKind(kind); setError(''); setNotice('');
    try {
      const next = await (kind === 'backup' ? libraryApi.startBackup(includeMedia) : libraryApi.startRestore());
      if (next) merge(next); else { setNotice('已取消选择'); toast.info('已取消选择'); }
    } catch (reason) { const message = errorText(reason); setError(message); toast.error(message); }
    finally { selecting.current = false; setPicking(false); setPickingKind(null); }
  }
  async function cancel() {
    if (!latest.current || canceling) return;
    setCanceling(true); setError('');
    try { await libraryApi.cancelTransfer(latest.current.id); }
    catch (reason) { setError(errorText(reason)); }
    finally { setCanceling(false); }
  }
  return { snapshot, picking, pickingKind, canceling, error: error || connectionError || readError, notice, start, cancel, reconnect: () => setConnection(value => value + 1) };
}
export type LibraryTransferClient = ReturnType<typeof useLibraryTransfer>;
