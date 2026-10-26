import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ExternalLink, AlertCircle } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type Task } from '@/api';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { readPlaybackPosition, writePlaybackPosition } from '@/lib/playback-position';

function PlaybackSession({ task, open, close, removedIds }: { task: Task; open: boolean; close: () => void; removedIds: ReadonlySet<string> }) {
  const video = useRef<HTMLVideoElement>(null);
  const session = useRef({ active: true, fallback: false, externalPending: false });
  const sourceRequest = useRef<Promise<string> | null>(null);
  const progress = useRef({ metadataReady: false, lastSavedAt: 0, storageFailed: false });
  const [source, setSource] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [externalPending, setExternalPending] = useState(false);
  const [failure, setFailure] = useState('');

  function remember(element: HTMLVideoElement | null, force = true) {
    if (!element || !progress.current.metadataReady || progress.current.storageFailed || removedIds.has(task.id)
      || !Number.isFinite(element.duration) || element.duration <= 0 || !Number.isFinite(element.currentTime)) return;
    const now = performance.now();
    if (!force && (element.seeking || now - progress.current.lastSavedAt < 5000)) return;
    try {
      writePlaybackPosition(task, element.ended || element.currentTime >= element.duration ? 0 : element.currentTime);
      progress.current.lastSavedAt = now;
    } catch (reason) {
      progress.current.storageFailed = true;
      toast.error('无法保存播放进度', { description: errorText(reason) });
    }
  }

  function releaseVideo() {
    const element = video.current;
    if (!element) return;
    remember(element);
    element.pause();
    element.removeAttribute('src');
    element.load();
  }

  async function openExternal(manual = false) {
    if (!session.current.active || session.current.externalPending || !manual && session.current.fallback) return;
    session.current.fallback = true;
    session.current.externalPending = true;
    setExternalPending(true);
    setFailure('');
    releaseVideo();
    setSource(undefined);
    try {
      await api.play(task.id);
      if (session.current.active) {
        toast.info(manual ? '已使用系统播放器打开' : '内置播放不可用，已使用系统播放器打开');
        close();
      }
    } catch (reason) {
      if (session.current.active) {
        setLoading(false);
        setFailure(errorText(reason));
        toast.error('无法播放视频', { description: errorText(reason) });
      }
    } finally {
      session.current.externalPending = false;
      if (session.current.active) setExternalPending(false);
    }
  }

  useLayoutEffect(() => {
    const element = video.current;
    const flush = () => remember(element);
    const hidden = () => { if (document.visibilityState === 'hidden') flush(); };
    session.current.active = open;
    if (!open) releaseVideo();
    else {
      window.addEventListener('pagehide', flush);
      document.addEventListener('visibilitychange', hidden);
    }
    return () => {
      session.current.active = false;
      window.removeEventListener('pagehide', flush);
      document.removeEventListener('visibilitychange', hidden);
      flush();
      element?.pause();
      element?.removeAttribute('src');
      element?.load();
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    sourceRequest.current ??= api.playbackSource(task.id);
    sourceRequest.current.then(value => {
      if (active && session.current.active && !session.current.fallback) setSource(value);
    }).catch(() => { if (active) void openExternal(); });
    return () => { active = false; };
  }, [task.id, open]);

  return <>
    <div className="video-player-stage" aria-busy={loading || externalPending}>
      <video ref={video} src={source} controls autoPlay playsInline preload="metadata" aria-label="本地视频播放器"
        onCanPlay={() => { if (session.current.active && !session.current.fallback) setLoading(false); }}
        onWaiting={() => { if (session.current.active && !session.current.fallback) setLoading(true); }}
        onPlaying={() => { if (session.current.active) setLoading(false); }}
        onPause={event => { if (session.current.active) { setLoading(false); remember(event.currentTarget); } }}
        onSeeked={event => { if (session.current.active && !session.current.fallback) remember(event.currentTarget); }}
        onTimeUpdate={event => { if (session.current.active && !session.current.fallback) remember(event.currentTarget, false); }}
        onEnded={event => { if (session.current.active) { setLoading(false); remember(event.currentTarget); } }}
        onLoadedMetadata={event => {
          if (!session.current.active || session.current.fallback) return;
          const element = event.currentTarget;
          if (!element.videoWidth || !element.videoHeight) { void openExternal(); return; }
          const position = readPlaybackPosition(task);
          if (position > 0 && Number.isFinite(element.duration) && position < element.duration) element.currentTime = position;
          progress.current.metadataReady = true;
        }}
        onError={() => { if (source) void openExternal(); }} />
      {open && (loading || externalPending) && !failure && <div className="video-player-loading" role="status"><Spinner aria-hidden="true" /><span>{externalPending ? '正在打开系统播放器…' : '正在加载视频…'}</span></div>}
    </div>
    {failure && <Alert variant="destructive"><AlertCircle /><AlertDescription>{failure}</AlertDescription></Alert>}
    <div className="flex justify-end"><Button variant="outline" disabled={externalPending} onClick={() => void openExternal(true)}><ExternalLink />{externalPending ? '打开中…' : '使用系统播放器'}</Button></div>
  </>;
}

export function VideoPlayer({ task, close, returnFocus, fallbackFocus, removedIds }: { task: Task | null; close: () => void; returnFocus: HTMLElement | null; fallbackFocus: () => HTMLElement | null; removedIds: ReadonlySet<string> }) {
  const focus = useRef<HTMLElement | null>(null);
  const lastTask = useRef<Task | null>(null);
  if (task) { focus.current = returnFocus; lastTask.current = task; }
  return <Dialog open={!!task} onOpenChange={open => { if (!open) close(); }}>
    <DialogContent className="video-player-dialog" onCloseAutoFocus={event => {
      event.preventDefault();
      const target = focus.current;
      (target?.isConnected && !target.closest('[inert]') ? target : fallbackFocus())?.focus({ preventScroll: true });
    }}>
      <DialogHeader className="pr-8"><DialogTitle className="line-clamp-2">{lastTask.current?.video?.title?.trim() || '视频播放'}</DialogTitle><DialogDescription className="sr-only">本地视频播放器</DialogDescription></DialogHeader>
      {lastTask.current && <PlaybackSession key={lastTask.current.id} task={lastTask.current} open={!!task} close={close} removedIds={removedIds} />}
    </DialogContent>
  </Dialog>;
}
