import { useEffect, useRef, useState } from 'react';
import { Camera, RotateCcw, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { errorText } from '@/api';
import { libraryApi, type LibraryFile, type LibraryKeyframe } from '@/lib/library-api';
import { clipTime } from '@/lib/clip-playback';
import type { LibraryVideoHandle } from '@/components/library-video';
import type { useLibraryKeyframes } from '@/hooks/use-library-keyframes';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Spinner } from '@/components/ui/spinner';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';

function FrameImage({ frame, open }: { frame: LibraryKeyframe; open: (frame: LibraryKeyframe, url: string) => void }) {
  const [source, setSource] = useState('');
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false; let url: string | undefined; setError('');
    void libraryApi.keyframeImage(frame.asset_id, frame.id).then(bytes => {
      if (disposed) return;
      url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/png' })); setSource(url);
    }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; if (url) URL.revokeObjectURL(url); };
  }, [frame.asset_id, frame.id, attempt]);
  return <>{source && !error ? <button className="library-frame-image" type="button" aria-label={`查看关键帧 ${clipTime(frame.actual_ms)}`} onClick={() => open(frame, source)}><img src={source} alt={`关键帧 ${clipTime(frame.actual_ms)}`} loading="lazy" onError={() => setError('图片无法显示，请重新读取')} /></button> : error ? <div><LibraryError message={error} /><Button size="sm" variant="ghost" onClick={() => { setSource(''); setAttempt(value => value + 1); }}><RotateCcw />重新读取图片</Button></div> : <Spinner aria-label="正在读取关键帧" />}</>;
}

export function LibraryKeyframes({ assetId, files, file, player, state }: {
  assetId: string; files: LibraryFile[]; file?: LibraryFile; player: React.RefObject<LibraryVideoHandle | null>; state: ReturnType<typeof useLibraryKeyframes>;
}) {
  const [selected, setSelected] = useState<{ frame: LibraryKeyframe; url: string } | null>(null);
  const [deleting, setDeleting] = useState('');
  const [limit, setLimit] = useState(12);
  const section = useRef<HTMLElement>(null);
  const operation = state.operation?.asset_id === assetId ? state.operation : null;
  useEffect(() => { setSelected(null); setLimit(12); }, [assetId]);
  useEffect(() => { if (selected && !state.frames.some(frame => frame.id === selected.frame.id)) setSelected(null); }, [state.frames, selected]);
  function capture() {
    const clock = player.current?.clock();
    if (!file || !clock) { state.setError('请先播放视频，再提取当前位置画面'); return; }
    if (!clock.ranges.some(range => clock.current_ms >= range.start_ms && clock.current_ms < range.end_ms)) { state.setError('当前位置不在可播放范围内'); return; }
    void state.start({ kind: 'position', file_id: file.id, position_ms: clock.current_ms });
  }
  async function remove(frame: LibraryKeyframe) {
    if (deleting) return;
    setDeleting(frame.id); state.setError('');
    try { await libraryApi.deleteKeyframe(assetId, frame.id); if (selected?.frame.id === frame.id) setSelected(null); state.refresh(); toast.success('关键帧已删除'); }
    catch (reason) { state.setError(errorText(reason)); }
    finally { setDeleting(''); }
  }
  return <section ref={section} className="library-keyframes" aria-label="关键帧" aria-busy={state.loading}>
    <div className="library-section-heading"><h3>关键帧</h3><Button size="sm" variant="outline" disabled={!file || file.missing || file.changed || state.busy} onClick={capture}><Camera />当前位置取帧</Button></div>
    <LibraryError message={state.error || (operation?.state === 'failed' && operation.error ? errorText(operation.error) : '')} />
    {operation?.state === 'running' && <div className="library-file-actions" role="status"><Spinner />正在提取 {operation.completed} / {operation.total}<Button size="sm" variant="ghost" disabled={state.pending || operation.canceling} onClick={() => void state.cancel()}>{operation.canceling ? '取消中…' : '取消提取关键帧'}</Button></div>}
    {operation?.state === 'canceled' && <p role="status" className="text-xs text-muted-foreground">本次关键帧提取已取消</p>}
    {operation?.state === 'completed' && <p role="status" className="text-xs text-muted-foreground">已保存 {operation.frame_ids.length} 张关键帧</p>}
    {state.operation?.state === 'running' && !operation && <p role="status" className="text-xs text-muted-foreground">其他视频正在提取关键帧</p>}
    {!state.loading && !state.frames.length && <p className="text-xs text-muted-foreground">暂无关键帧</p>}
    <ul className="library-keyframe-grid">{state.frames.slice(0, limit).map(frame => <li key={frame.id} data-keyframe={frame.id}>
      <FrameImage frame={frame} open={(value, url) => setSelected({ frame: value, url })} />
      <div className="library-frame-caption"><span>{clipTime(frame.actual_ms)}</span>{frame.black && <Badge variant="outline">黑帧</Badge>}<Button variant="ghost" size="icon-sm" aria-label={`删除关键帧 ${clipTime(frame.actual_ms)}`} disabled={!!deleting} onClick={() => void remove(frame)}>{deleting === frame.id ? <Spinner /> : <Trash2 />}</Button></div>
      <p className="text-xs text-muted-foreground">请求 {clipTime(frame.requested_ms)}，文件版本 {files.findIndex(file => file.id === frame.file_id) + 1}</p>
      {frame.clip_id && <p className="text-xs text-muted-foreground">{clipTime(frame.sample_start_ms)}–{clipTime(frame.sample_end_ms!)}，采样 {frame.sample_index + 1} / {frame.sample_count}</p>}
      {(frame.missing || frame.source_changed) && <p className="text-xs text-muted-foreground">{frame.missing ? '源文件缺失' : '源文件已变化'}</p>}
    </li>)}</ul>
    {limit < state.frames.length && <Button size="sm" variant="outline" onClick={() => setLimit(value => value + 12)}>加载更多关键帧</Button>}
    {selected && <Dialog open onOpenChange={open => { if (!open) setSelected(null); }}><DialogContent className="library-frame-dialog"><DialogHeader><DialogTitle>关键帧 {clipTime(selected.frame.actual_ms)}</DialogTitle><DialogDescription>请求 {clipTime(selected.frame.requested_ms)}，{selected.frame.width} × {selected.frame.height}</DialogDescription></DialogHeader><img src={selected.url} alt={`关键帧 ${clipTime(selected.frame.actual_ms)}`} /></DialogContent></Dialog>}
  </section>;
}
