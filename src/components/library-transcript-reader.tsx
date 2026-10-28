import { useEffect, useRef, useState } from 'react';
import { Pause, Play } from 'lucide-react';
import type { LibraryCitation, LibraryDocumentContent } from '@/lib/library-api';
import { clipTime } from '@/lib/clip-playback';
import type { LibraryVideoHandle } from '@/components/library-video';
import { Button } from '@/components/ui/button';
import { Switch as SwitchPrimitive } from 'radix-ui';
import { LibraryError } from '@/components/library-organization';

export function LibraryTranscriptReader({ document, available, unavailableReason, player, ready, onJump, onSelect }: {
  document: LibraryDocumentContent; available: boolean; unavailableReason: string;
  player: React.RefObject<LibraryVideoHandle | null>; ready: number;
  onJump: (positionMs: number) => Promise<void>;
  onSelect: (citation: LibraryCitation, range: { start_ms: number; end_ms: number }) => void;
}) {
  const segments = document.timing!.segments;
  const [current, setCurrent] = useState(-1);
  const [paused, setPaused] = useState(true);
  const [scroll, setScroll] = useState(true);
  const [selecting, setSelecting] = useState(false);
  const [selection, setSelection] = useState<{ anchor: number; end: number } | null>(null);
  const [error, setError] = useState('');
  const region = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setCurrent(-1); setPaused(true);
    if (!available) return;
    return player.current?.subscribe(clock => {
      if (!clock) { setCurrent(-1); setPaused(true); return; }
      setPaused(clock.paused);
      // Segments are ordered and disjoint, as validated by Core on persistence/read.
      let left = 0, right = segments.length - 1, found = -1;
      while (left <= right) {
        const middle = (left + right) >>> 1, segment = segments[middle];
        if (clock.current_ms < segment.start_ms) right = middle - 1;
        else if (clock.current_ms >= segment.end_ms) left = middle + 1;
        else { found = middle; break; }
      }
      setCurrent(found);
    });
  }, [available, player, ready, segments]);
  useEffect(() => {
    if (!scroll || selecting || current < 0) return;
    const container = region.current, row = container?.querySelector<HTMLElement>(`[data-segment="${current}"]`);
    if (!container || !row) return;
    const bounds = container.getBoundingClientRect(), item = row.getBoundingClientRect();
    if (item.top < bounds.top || item.bottom > bounds.bottom) container.scrollTop += item.top - bounds.top - container.clientHeight / 3;
  }, [current, scroll, selecting]);
  useEffect(() => { if (!available) { setSelecting(false); setSelection(null); } }, [available]);
  const first = selection ? Math.min(selection.anchor, selection.end) : -1;
  const last = selection ? Math.max(selection.anchor, selection.end) : -1;
  async function activate(index: number) {
    if (!available) return;
    setError('');
    if (selecting) { setSelection(old => ({ anchor: old?.anchor ?? index, end: index })); return; }
    try { await onJump(segments[index].start_ms); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
  }
  function togglePlayback() {
    if (player.current?.clock()) player.current.toggle();
    else {
      setError('');
      void onJump(segments[current >= 0 ? current : 0].start_ms).catch(reason => setError(reason instanceof Error ? reason.message : String(reason)));
    }
  }
  return <section className="library-transcript-reader" aria-label="同步阅读">
    <div className="library-writing-toolbar">
      <Button size="sm" variant="outline" disabled={!available} onClick={togglePlayback}>{paused ? <Play /> : <Pause />}{paused ? '播放' : '暂停'}</Button>
      <label className="library-follow-switch"><SwitchPrimitive.Root className="mcp-switch" checked={scroll} onCheckedChange={setScroll} aria-label="自动滚动"><SwitchPrimitive.Thumb className="mcp-switch-thumb" /></SwitchPrimitive.Root>自动滚动</label>
      <Button size="sm" variant="outline" disabled={!available} aria-pressed={selecting} onClick={() => { setSelecting(value => !value); setSelection(null); setError(''); }}>{selecting ? '取消选取' : '选取片段'}</Button>
    </div>
    {!available && <p className="text-xs text-muted-foreground" role="status">{unavailableReason}</p>}
    <LibraryError message={error} />
    {selecting && <div className="library-writing-toolbar"><span className="text-xs text-muted-foreground" role="status">{selection ? `${clipTime(segments[first].start_ms)}–${clipTime(segments[last].end_ms)}` : '选择起始和结束段落'}</span><Button size="sm" disabled={!selection} onClick={() => {
      if (!selection) return;
      onSelect({ asset_id: document.summary.asset_id, document_id: document.summary.id, quote: segments.slice(first, last + 1).map(segment => segment.text).join('\n') }, { start_ms: segments[first].start_ms, end_ms: segments[last].end_ms });
      setSelecting(false); setSelection(null);
    }}>收藏所选片段</Button></div>}
    <div ref={region} className="library-transcript-segments" tabIndex={0} aria-label="文案段落" aria-keyshortcuts="Space ArrowUp ArrowDown" onKeyDown={event => {
      const target = event.target as HTMLElement;
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || (target !== region.current && !target.closest('[data-segment]'))) return;
      if (event.code === 'Space' && available) { event.preventDefault(); if (!event.repeat) togglePlayback(); }
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        event.preventDefault();
        const focused = target.closest<HTMLElement>('[data-segment]');
        const index = focused ? Number(focused.dataset.segment) : (current >= 0 ? current : 0);
        const next = focused ? Math.max(0, Math.min(segments.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1))) : index;
        region.current?.querySelector<HTMLElement>(`[data-segment="${next}"]`)?.focus();
      }
    }}>
      {segments.map((segment, index) => <button key={index} type="button" data-segment={index} data-current={current === index || undefined} data-selected={selection && index >= first && index <= last || undefined} aria-current={current === index ? 'true' : undefined} aria-pressed={selecting ? !!selection && index >= first && index <= last : undefined} aria-label={`${selecting ? '选择' : '跳转'}第 ${index + 1} 段 ${clipTime(segment.start_ms)} ${segment.text}`} disabled={!available} onClick={() => void activate(index)}><span className="library-segment-time">{clipTime(segment.start_ms)}</span><span>{segment.text}</span></button>)}
    </div>
    <p className="text-xs text-muted-foreground">语音段级时间</p>
  </section>;
}
