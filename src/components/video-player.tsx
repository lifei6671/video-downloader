import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ExternalLink, AlertCircle, Play } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type Video } from '@/api';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { readPlaybackPosition, writePlaybackPosition } from '@/lib/playback-position';

export type PlaybackSubject = {id: string; path: string | null; completed_at: number | null; video: Pick<Video, 'title' | 'width' | 'height' | 'published_at' | 'published_date'> | null; asset?: boolean};
export type MediaSize = { width: number; height: number };

function VideoStartButton({ onClick }: { onClick: () => void }) {
  return <div className="pointer-events-none absolute inset-0 z-[4] grid place-items-center">
    <Button size="icon-lg" className="pointer-events-auto" aria-label="播放视频" onClick={onClick}>
      <Play aria-hidden="true" />
    </Button>
  </div>;
}

export function PlaybackSession({ task, open, close, removedIds, onMediaSize, onMediaReady, renderLayout, loop = false, preloading = false }: {
  task: PlaybackSubject;
  open: boolean;
  close: () => void;
  removedIds: ReadonlySet<string>;
  onMediaSize?: (size: MediaSize) => void;
  onMediaReady?: (ready: boolean) => void;
  renderLayout?: (media: ReactNode, tools: ReactNode) => ReactNode;
  loop?: boolean;
  preloading?: boolean;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const session = useRef({ active: false, fallback: false, externalPending: false, mediaReady: false });
  const sourceRequest = useRef<Promise<{path: string | null; source: string}> | null>(null);
  const identity = useRef(task);
  const progress = useRef({ metadataReady: false, positioned: false, lastSavedAt: 0, storageFailed: false });
  const [source, setSource] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [paused, setPaused] = useState(true);
  const [externalPending, setExternalPending] = useState(false);
  const [failure, setFailure] = useState('');
  useLayoutEffect(() => { if (failure) onMediaReady?.(false); }, [failure, onMediaReady]);

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

  function prepareMetadata(element: HTMLVideoElement) {
    if (progress.current.metadataReady || session.current.fallback) return;
    if (!element.videoWidth || !element.videoHeight) {
      if (session.current.active) void openExternal();
      return;
    }
    if (!progress.current.positioned) {
      const position = readPlaybackPosition(identity.current);
      if (position > 0 && Number.isFinite(element.duration) && position < element.duration) element.currentTime = position;
      progress.current.positioned = true;
    }
    if (!session.current.active) return;
    onMediaSize?.({ width: element.videoWidth, height: element.videoHeight });
    progress.current.metadataReady = true;
  }

  function reportPlaying() {
    if (!session.current.active || session.current.fallback) return;
    setLoading(false);
    if (session.current.mediaReady) return;
    session.current.mediaReady = true;
    onMediaReady?.(true);
  }

  useLayoutEffect(() => {
    const element = video.current;
    const flush = () => remember(element);
    const hidden = () => { if (document.visibilityState === 'hidden') flush(); };
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

  useLayoutEffect(() => {
    session.current.active = open && !preloading;
    if (session.current.active) session.current.mediaReady = false;
    const element = video.current;
    // Promote the same buffered element without reassigning its source.
    if (!session.current.active || !loop || !element?.hasAttribute('src')) return;
    if (element.error) { void openExternal(); return; }
    if (element.readyState >= HTMLMediaElement.HAVE_METADATA) prepareMetadata(element);
    if (session.current.fallback) return;
    setPaused(false);
    void element.play().catch(reason => {
      if (!session.current.active || session.current.fallback || reason?.name === 'AbortError') return;
      setPaused(true);
      setLoading(false);
      // The existing play control can start playback when autoplay is denied.
      if (reason?.name !== 'NotAllowedError') setFailure('无法开始播放视频');
    });
  }, [open, preloading, loop]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const request = sourceRequest.current ??= task.asset ? api.assetPlaybackSource(task.id) : api.playbackSource(task.id).then(source => ({path: task.path, source}));
    request.then(value => {
      if (active && !session.current.fallback) { identity.current = {...task, path: value.path}; setSource(value.source); }
    }).catch(reason => {
      if (!active) return;
      if (preloading) {
        if (sourceRequest.current === request) sourceRequest.current = null;
        return;
      }
      if (task.asset) { setLoading(false); setFailure(errorText(reason)); }
      else void openExternal();
    });
    return () => { active = false; };
  }, [task.id, open, preloading]);

  const media = <>
    <div className="video-player-stage" aria-busy={loading || externalPending}>
      <video ref={video} src={source} controls={!preloading} autoPlay={open && !preloading} loop={loop} playsInline preload={loop ? 'auto' : 'metadata'} aria-label="本地视频播放器"
        onCanPlay={() => { if (open && !session.current.fallback) setLoading(false); }}
        onWaiting={() => { if (session.current.active && !session.current.fallback) setLoading(true); }}
        onPlay={() => { if (session.current.active && !session.current.fallback) { setPaused(false); setFailure(''); } }}
        onPlaying={reportPlaying}
        onPause={event => { if (session.current.active) { setPaused(true); setLoading(false); remember(event.currentTarget); } }}
        onSeeked={event => { if (session.current.active && !session.current.fallback) remember(event.currentTarget); }}
        onTimeUpdate={event => {
          const element = event.currentTarget;
          if (!session.current.mediaReady && !element.paused && !element.seeking && element.currentTime > 0
            && element.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) reportPlaying();
          if (session.current.active && !session.current.fallback) remember(element, false);
        }}
        onEnded={event => { if (session.current.active) { setPaused(true); setLoading(false); remember(event.currentTarget); } }}
        onLoadedMetadata={event => { if (open) prepareMetadata(event.currentTarget); }}
        onError={() => { if (source && session.current.active) void openExternal(); }} />
      {open && source && paused && !loading && !externalPending && !failure && <VideoStartButton onClick={playInPlace} />}
      {open && (loading || externalPending) && !failure && <div className="video-player-loading" role="status"><Spinner aria-hidden="true" /><span>{externalPending ? '正在打开系统播放器…' : '正在加载视频…'}</span></div>}
    </div>
    {failure && <Alert variant="destructive"><AlertCircle /><AlertDescription>{failure}</AlertDescription></Alert>}
  </>;
  const tools = <div className="video-player-tools flex justify-end"><Button variant="outline" disabled={externalPending} onClick={() => void openExternal(true)}><ExternalLink />{externalPending ? '打开中…' : '使用系统播放器'}</Button></div>;
  return renderLayout ? renderLayout(media, tools) : <>{media}{tools}</>;
}
