import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { libraryApi, type LibraryClip } from '@/lib/library-api';
import { clipTime, coveringRange, mediaRanges, seekMedia, type MediaClock } from '@/lib/clip-playback';
import { errorText } from '@/api';
import { LibraryError } from '@/components/library-organization';
import { Spinner } from '@/components/ui/spinner';

export interface LibraryPlaybackClock extends MediaClock { paused: boolean }
export interface LibraryVideoHandle {
  clock: () => MediaClock | null; toggle: () => void;
  seek: (positionMs: number) => Promise<void>;
  subscribe: (callback: (clock: LibraryPlaybackClock | null) => void) => () => void;
}
const playbackError = (reason: unknown) => reason instanceof Error ? reason.message : errorText(reason);

/** One mounted, source-bound file session. Range requests are prepared again by Core. */
export const LibraryVideo = forwardRef<LibraryVideoHandle, {
  assetId: string; fileId: string; clipId?: string; positionMs?: number; onMissing: () => void; onReady?: () => void;
}>(function LibraryVideo({ assetId, fileId, clipId, positionMs, onMissing, onReady }, ref) {
  const video = useRef<HTMLVideoElement>(null);
  const [source, setSource] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [status, setStatus] = useState('');
  const clip = useRef<LibraryClip | undefined>(undefined);
  const clock = useRef<MediaClock | null>(null);
  const lower = useRef(0);
  const initialized = useRef(false);
  const initializing = useRef(false);
  const finished = useRef(false);
  const abort = useRef(new AbortController());
  const frame = useRef<number | undefined>(undefined);
  const listeners = useRef(new Set<(clock: LibraryPlaybackClock | null) => void>());
  const jump = useRef<AbortController | null>(null);
  function snapshot(): LibraryPlaybackClock | null {
    const v = video.current;
    return v && clock.current && initialized.current ? { current_ms: Math.round(v.currentTime * 1000), ranges: mediaRanges(v, lower.current), paused: v.paused } : null;
  }
  function publish() { const next = snapshot(); listeners.current.forEach(callback => callback(next)); }
  async function seek(position: number) {
    const v = video.current;
    if (!v || !initialized.current || initializing.current) throw Error('视频尚未就绪，请重试');
    if (!coveringRange(mediaRanges(v, lower.current), position, position + 1)) throw Error('段落起点不在可播放范围内');
    jump.current?.abort(); const controller = new AbortController(); jump.current = controller;
    v.pause(); setError(''); setStatus('');
    try {
      const actual = await seekMedia(v, position / 1000, controller.signal);
      if (controller.signal.aborted) return;
      if (Math.abs(actual * 1000 - position) > 50) throw Error('播放器无法定位段落起点');
      publish(); await v.play();
    } catch (reason) { if (!controller.signal.aborted) throw reason; }
    finally { if (jump.current === controller) jump.current = null; }
  }
  useImperativeHandle(ref, () => ({
    clock: () => video.current && clock.current ? { current_ms: Math.round(video.current.currentTime * 1000), ranges: mediaRanges(video.current, lower.current) } : null,
    toggle: () => { const v = video.current; if (v) { if (v.paused) void v.play().catch(reason => setError(playbackError(reason))); else v.pause(); } },
    seek,
    subscribe: callback => { listeners.current.add(callback); callback(snapshot()); return () => { listeners.current.delete(callback); }; },
  }), []);
  useEffect(() => {
    let disposed = false;
    const element = video.current;
    const controller = new AbortController(); abort.current = controller;
    const releaseFrame = () => { if (frame.current !== undefined) cancelAnimationFrame(frame.current); frame.current = undefined; };
    const hidden = () => { if (clip.current) { element?.pause(); releaseFrame(); } };
    const visibility = () => { if (document.hidden) hidden(); };
    window.addEventListener('blur', hidden); document.addEventListener('visibilitychange', visibility);
    const request = clipId ? libraryApi.clipPlayback(assetId, clipId).then(value => {
      if (value.clip.file_id !== fileId) throw 'CLIP_SOURCE_MISMATCH';
      return { url: value.url, clip: value.clip };
    }) : libraryApi.playbackUrl(assetId, fileId).then(url => ({ url, clip: undefined }));
    void request.then(value => { if (!disposed) { clip.current = value.clip; setSource(value.url); } })
      .catch(reason => {
        if (disposed) return;
        setLoading(false); setError(String(reason) === 'PLAYBACK_SOURCE_FAILED' ? '无法读取本地视频，请尝试系统播放器' : errorText(reason));
        if (String(reason) === 'FILE_MISSING' || String(reason) === 'FILE_NOT_FOUND' || String(reason) === 'CLIP_SOURCE_CHANGED') onMissing();
      });
    return () => {
      disposed = true; controller.abort(); jump.current?.abort(); releaseFrame(); clock.current = null;
      listeners.current.forEach(callback => callback(null)); listeners.current.clear();
      window.removeEventListener('blur', hidden); document.removeEventListener('visibilitychange', visibility);
      element?.pause(); element?.removeAttribute('src'); element?.load();
    };
  }, [assetId, fileId, clipId, onMissing]);
  async function initialize(v: HTMLVideoElement) {
    if (initialized.current || initializing.current || !source || v.readyState < 2 || !v.videoWidth || !v.videoHeight) return;
    const ranges = mediaRanges(v);
    if (!ranges.length) { setLoading(false); if (clipId) setError('无法读取片段播放范围，请尝试系统播放器'); onReady?.(); return; }
    initializing.current = true;
    try {
      v.pause();
      lower.current = await seekMedia(v, ranges[0].start_ms / 1000, abort.current.signal);
      if (abort.current.signal.aborted) return;
      const actual = mediaRanges(v, lower.current);
      clock.current = { current_ms: Math.round(v.currentTime * 1000), ranges: actual };
      initialized.current = true; onReady?.();
      publish();
      const selected = clip.current;
      if (selected) {
        if (!coveringRange(actual, selected.start_ms, selected.end_ms)) {
          const available = actual.map(r => `${clipTime(r.start_ms)}–${clipTime(r.end_ms)}`).join('、');
          throw Error(`片段无法完整播放，可播放范围：${available || '未知'}`);
        }
        const position = await seekMedia(v, selected.start_ms / 1000, abort.current.signal);
        if (Math.abs(position * 1000 - selected.start_ms) > 50) throw Error('播放器无法定位片段起点，请尝试系统播放器');
        setStatus(`正在播放片段：${selected.title}`);
      } else if (positionMs !== undefined) {
        if (!coveringRange(actual, positionMs, positionMs + 1)) throw Error('段落起点不在可播放范围内');
        const position = await seekMedia(v, positionMs / 1000, abort.current.signal);
        if (Math.abs(position * 1000 - positionMs) > 50) throw Error('播放器无法定位段落起点');
      }
      if (!abort.current.signal.aborted) { setLoading(false); await v.play(); }
    } catch (reason) { if (!abort.current.signal.aborted) { v.pause(); setLoading(false); setError(playbackError(reason)); } }
    finally { initializing.current = false; }
  }
  function stopFrame() { if (frame.current !== undefined) cancelAnimationFrame(frame.current); frame.current = undefined; }
  function refreshClock(v: HTMLVideoElement) {
    if (!initialized.current || initializing.current) return;
    const ranges = mediaRanges(v, lower.current);
    if (JSON.stringify(ranges) !== JSON.stringify(clock.current?.ranges)) {
      clock.current = { current_ms: Math.round(v.currentTime * 1000), ranges }; onReady?.();
      const selected = clip.current;
      if (selected && !v.paused && !coveringRange(ranges, selected.start_ms, selected.end_ms)) {
        v.pause(); stopFrame(); finished.current = true; setLoading(false); setStatus('');
        setError('片段未完整播放：可播放范围已变化，请重新打开或使用系统播放器');
      }
    }
  }
  function ended(v: HTMLVideoElement) {
    setLoading(false); stopFrame();
    const selected = clip.current;
    if (!selected) return;
    finished.current = true;
    if (Math.round(v.currentTime * 1000) < selected.end_ms) {
      setStatus(''); setError('片段未完整播放：视频提前结束，请尝试系统播放器');
    } else setStatus('片段播放完成');
  }
  function stopAtEnd(v: HTMLVideoElement) {
    const range = clip.current;
    if (!range || finished.current || v.seeking || v.currentTime * 1000 < range.end_ms) return;
    finished.current = true; v.pause(); stopFrame();
    v.currentTime = range.end_ms / 1000;
    setStatus('片段播放完成');
  }
  function tick() {
    const v = video.current;
    if (!v || !clip.current || v.paused) return;
    stopAtEnd(v);
    if (!v.paused) frame.current = requestAnimationFrame(tick);
  }
  async function play(v: HTMLVideoElement) {
    const selected = clip.current;
    if (!selected || !initialized.current || initializing.current) return;
    const ranges = mediaRanges(v, lower.current);
    if (!coveringRange(ranges, selected.start_ms, selected.end_ms)) { v.pause(); setError('片段范围已不可播放，请重新打开或使用系统播放器'); return; }
    if (v.currentTime * 1000 < selected.start_ms - 1 || v.currentTime * 1000 >= selected.end_ms) {
      v.pause();
      try {
        const position = await seekMedia(v, selected.start_ms / 1000, abort.current.signal);
        if (Math.abs(position * 1000 - selected.start_ms) > 50) throw Error('播放器无法定位片段起点');
        if (!abort.current.signal.aborted) { finished.current = false; setError(''); await v.play(); }
      } catch (reason) { if (!abort.current.signal.aborted) setError(playbackError(reason)); }
      return;
    }
    finished.current = false; setError(''); setStatus(`正在播放片段：${selected.title}`); stopFrame(); frame.current = requestAnimationFrame(tick);
  }
  return <div className="library-video-session">
    <div className="video-player-stage" aria-busy={loading}>
      <video ref={video} src={source} controls playsInline preload="metadata" aria-label="知识库视频播放器"
        onCanPlay={event => { void initialize(event.currentTarget); }} onProgress={event => { if (!initialized.current) void initialize(event.currentTarget); else refreshClock(event.currentTarget); }}
        onDurationChange={event => { refreshClock(event.currentTarget); publish(); }} onSeeked={event => { refreshClock(event.currentTarget); publish(); }}
        onPlay={event => { publish(); void play(event.currentTarget); }} onPlaying={() => { setLoading(false); publish(); if (clip.current) { stopFrame(); frame.current = requestAnimationFrame(tick); } }}
        onPause={() => { setLoading(false); stopFrame(); publish(); }} onEnded={event => { ended(event.currentTarget); publish(); }}
        onTimeUpdate={event => { stopAtEnd(event.currentTarget); publish(); }}
        onSeeking={event => { const v = event.currentTarget; const r = clip.current; if (initialized.current && !initializing.current && r && (v.currentTime * 1000 < r.start_ms - 1 || v.currentTime * 1000 > r.end_ms + 1)) { v.pause(); setStatus('已暂停：当前位置不在片段范围'); } }}
        onWaiting={() => { if (!error) setLoading(true); }}
        onError={() => { if (source) { setLoading(false); setError('内置播放器无法播放此视频，请尝试系统播放器'); } }}
        onLoadedMetadata={event => { if (!event.currentTarget.videoWidth || !event.currentTarget.videoHeight) { setLoading(false); setError('内置播放器无法显示视频画面，请尝试系统播放器'); event.currentTarget.pause(); } }} />
      {loading && !error && <div className="video-player-loading" role="status"><Spinner />正在加载视频…</div>}
    </div>
    {status && <p role="status" className="text-xs text-muted-foreground">{status}</p>}
    <LibraryError message={error} />
  </div>;
});
