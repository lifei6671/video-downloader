import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { ExternalLink, AlertCircle, Play } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type Task, type BackupAsset } from '@/api';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { readPlaybackPosition, writePlaybackPosition } from '@/lib/playback-position';

export type PlaybackSubject = {id: string; path: string | null; completed_at: number | null; video: {title?: string; width?: number | null; height?: number | null} | null; asset?: boolean};
export type MediaSize = { width: number; height: number };

function VideoStartButton({ onClick }: { onClick: () => void }) {
  return <div className="pointer-events-none absolute inset-0 grid place-items-center">
    <Button size="icon-lg" className="pointer-events-auto" aria-label="播放视频" onClick={onClick}>
      <Play aria-hidden="true" />
    </Button>
  </div>;
}

export function PlaybackSession({ task, open, close, removedIds, onMediaSize, renderLayout }: {
  task: PlaybackSubject;
  open: boolean;
  close: () => void;
  removedIds: ReadonlySet<string>;
  onMediaSize?: (size: MediaSize) => void;
  renderLayout?: (media: ReactNode, tools: ReactNode) => ReactNode;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const session = useRef({ active: true, fallback: false, externalPending: false });
  const sourceRequest = useRef<Promise<{path: string | null; source: string}> | null>(null);
  const identity = useRef(task);
  const progress = useRef({ metadataReady: false, lastSavedAt: 0, storageFailed: false });
  const [source, setSource] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [paused, setPaused] = useState(true);
  const [externalPending, setExternalPending] = useState(false);
  const [failure, setFailure] = useState('');

  function remember(element: HTMLVideoElement | null, force = true) {
    if (!element || !progress.current.metadataReady || progress.current.storageFailed || removedIds.has(task.id)
      || !Number.isFinite(element.duration) || element.duration <= 0 || !Number.isFinite(element.currentTime)) return;
    const now = performance.now();
    if (!force && (element.seeking || now - progress.current.lastSavedAt < 5000)) return;
    try {
      writePlaybackPosition(identity.current, element.ended || element.currentTime >= element.duration ? 0 : element.currentTime);
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
      if (task.asset) { await api.ensureAsset(task.id); if (!session.current.active) return; await api.playAsset(task.id); }
      else await api.play(task.id);
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

  function playInPlace() {
    const element = video.current;
    if (!element || !session.current.active) return;
    element.focus({ preventScroll: true });
    void element.play().catch(reason => {
      if (session.current.active && !session.current.fallback && reason?.name !== 'AbortError') {
        setLoading(false);
        setFailure('无法开始播放视频');
      }
    });
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
    sourceRequest.current ??= task.asset ? api.assetPlaybackSource(task.id) : api.playbackSource(task.id).then(source => ({path: task.path, source}));
    sourceRequest.current.then(value => {
      if (active && session.current.active && !session.current.fallback) { identity.current = {...task, path: value.path}; setSource(value.source); }
    }).catch(reason => {
      if (!active) return;
      if (task.asset) { setLoading(false); setFailure(errorText(reason)); }
      else void openExternal();
    });
    return () => { active = false; };
  }, [task.id, open]);

  const media = <>
    <div className="video-player-stage" aria-busy={loading || externalPending}>
      <video ref={video} src={source} controls autoPlay playsInline preload="metadata" aria-label="本地视频播放器"
        onCanPlay={() => { if (session.current.active && !session.current.fallback) setLoading(false); }}
        onWaiting={() => { if (session.current.active && !session.current.fallback) setLoading(true); }}
        onPlay={() => { if (session.current.active && !session.current.fallback) { setPaused(false); setFailure(''); } }}
        onPlaying={() => { if (session.current.active) setLoading(false); }}
        onPause={event => { if (session.current.active) { setPaused(true); setLoading(false); remember(event.currentTarget); } }}
        onSeeked={event => { if (session.current.active && !session.current.fallback) remember(event.currentTarget); }}
        onTimeUpdate={event => { if (session.current.active && !session.current.fallback) remember(event.currentTarget, false); }}
        onEnded={event => { if (session.current.active) { setPaused(true); setLoading(false); remember(event.currentTarget); } }}
        onLoadedMetadata={event => {
          if (!session.current.active || session.current.fallback) return;
          const element = event.currentTarget;
          if (!element.videoWidth || !element.videoHeight) { void openExternal(); return; }
          onMediaSize?.({ width: element.videoWidth, height: element.videoHeight });
          const position = readPlaybackPosition(identity.current);
          if (position > 0 && Number.isFinite(element.duration) && position < element.duration) element.currentTime = position;
          progress.current.metadataReady = true;
        }}
        onError={() => { if (source && session.current.active) void openExternal(); }} />
      {open && source && paused && !loading && !externalPending && !failure && <VideoStartButton onClick={playInPlace} />}
      {open && (loading || externalPending) && !failure && <div className="video-player-loading" role="status"><Spinner aria-hidden="true" /><span>{externalPending ? '正在打开系统播放器…' : '正在加载视频…'}</span></div>}
    </div>
    {failure && <Alert variant="destructive"><AlertCircle /><AlertDescription>{failure}</AlertDescription></Alert>}
  </>;
  const tools = <div className="video-player-tools flex justify-end"><Button variant="outline" disabled={externalPending} onClick={() => void openExternal(true)}><ExternalLink />{externalPending ? '打开中…' : '使用系统播放器'}</Button></div>;
  return renderLayout ? renderLayout(media, tools) : <>{media}{tools}</>;
}

export function VideoPlayer({ task, asset, close, returnFocus, fallbackFocus, removedIds }: { task: Task | null; asset?: BackupAsset | null; close: () => void; returnFocus: HTMLElement | null; fallbackFocus: () => HTMLElement | null; removedIds: ReadonlySet<string> }) {
  const focus = useRef<HTMLElement | null>(null);
  const lastTask = useRef<PlaybackSubject | null>(null);
  const [measured, setMeasured] = useState<{ id: string; ratio: number } | null>(null);
  const subject: PlaybackSubject | null = task ?? (asset ? { id: asset.asset_id, path: asset.path, completed_at: null, video: asset.metadata, asset: true } : null);
  if (subject) { focus.current = returnFocus; lastTask.current = subject; }
  const current = lastTask.current;
  const initialRatio = current?.video?.width && current.video.height ? current.video.width / current.video.height : 16 / 9;
  const ratio = measured?.id === current?.id ? measured?.ratio : initialRatio;
  const playerStyle = { '--video-player-ratio': ratio && Number.isFinite(ratio) && ratio > 0 ? ratio : 16 / 9 } as CSSProperties;
  return <Dialog open={!!subject} onOpenChange={open => { if (!open) close(); }}>
    <DialogContent className="video-player-dialog" style={playerStyle} onCloseAutoFocus={event => {
      event.preventDefault();
      const target = focus.current;
      (target?.isConnected && !target.closest('[inert]') ? target : fallbackFocus())?.focus({ preventScroll: true });
    }}>
      <DialogHeader className="pr-8"><DialogTitle className="line-clamp-2">{lastTask.current?.video?.title?.trim() || '视频播放'}</DialogTitle><DialogDescription className="sr-only">本地视频播放器</DialogDescription></DialogHeader>
      {current && <PlaybackSession key={current.id} task={current} open={!!subject} close={close} removedIds={removedIds} onMediaSize={({ width, height }) => setMeasured({ id: current.id, ratio: width / height })} />}
    </DialogContent>
  </Dialog>;
}
