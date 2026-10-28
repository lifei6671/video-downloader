import type { ClipRange } from './library-api';

export interface MediaClock { current_ms: number; ranges: ClipRange[] }
export function mediaRanges(video: HTMLVideoElement, lower = 0): ClipRange[] {
  if (!Number.isFinite(video.duration) || video.duration <= 0) return [];
  const result: ClipRange[] = [];
  for (let i = 0; i < video.seekable.length; i++) {
    const start = Math.max(lower, video.seekable.start(i), 0);
    const end = Math.min(video.duration, video.seekable.end(i));
    if (Number.isFinite(start) && Number.isFinite(end) && end > start) {
      result.push({ start_ms: Math.ceil(start * 1000 - .001), end_ms: Math.floor(end * 1000 + .001) });
    }
  }
  return result.filter(r => r.end_ms > r.start_ms);
}
export function coveringRange(ranges: ClipRange[], start: number, end: number) {
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || start >= end) return undefined;
  return ranges.find(r => start >= r.start_ms && end <= r.end_ms);
}
export function clipTime(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;
}
export function seekMedia(video: HTMLVideoElement, seconds: number, signal: AbortSignal): Promise<number> {
  return new Promise((resolve, reject) => {
    const finish = (error?: Error) => {
      clearTimeout(timer); video.removeEventListener('seeked', done); video.removeEventListener('error', failed); signal.removeEventListener('abort', canceled);
      if (error) reject(error); else resolve(video.currentTime);
    };
    const done = () => finish();
    const failed = () => finish(Error('无法定位视频，请尝试系统播放器'));
    const canceled = () => finish(Error('CANCELED'));
    const timer = setTimeout(() => finish(Error('视频定位超时，请重试或使用系统播放器')), 8000);
    if (signal.aborted) { canceled(); return; }
    video.addEventListener('seeked', done); video.addEventListener('error', failed); signal.addEventListener('abort', canceled, { once: true });
    try { video.currentTime = seconds; if (!video.seeking && video.readyState >= 2) done(); }
    catch { failed(); }
  });
}
