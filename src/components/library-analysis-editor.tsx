import { useEffect, useState } from 'react';
import { Play, Plus, Trash2 } from 'lucide-react';
import { errorText } from '@/api';
import { libraryApi, type AnalysisInput, type LibraryClip, type LibraryKeyframe, type LibraryNoteAnalysis, type ObservationInput } from '@/lib/library-api';
import { blankObservation, copywritingFields, observationDimensions, observationEvidence } from '@/lib/note-analysis';
import { clipTime } from '@/lib/clip-playback';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Spinner } from '@/components/ui/spinner';
import { LibraryError } from '@/components/library-organization';

function AnalysisFrame({ assetId, frameId, close }: { assetId: string; frameId: string; close: () => void }) {
  const [url, setUrl] = useState(''), [error, setError] = useState(''), [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false, image = ''; setUrl(''); setError('');
    void libraryApi.keyframeImage(assetId, frameId).then(bytes => {
      if (disposed) return;
      image = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/png' })); setUrl(image);
    }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; if (image) URL.revokeObjectURL(image); };
  }, [assetId, frameId, attempt]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="library-frame-dialog"><DialogHeader><DialogTitle>引用关键帧</DialogTitle><DialogDescription className="sr-only">笔记引用的已保存画面</DialogDescription></DialogHeader><LibraryError message={error} />{error ? <Button variant="outline" onClick={() => setAttempt(value => value + 1)}>重新读取图片</Button> : url ? <img src={url} alt="笔记引用关键帧" onError={() => setError('图片无法显示，请重新读取')} /> : <Spinner />}</DialogContent></Dialog>;
}

export function LibraryAnalysisEditor({ assetId, content, saved, disabled, resourcesRevision, onChange, onPlay }: {
  assetId: string; content: AnalysisInput; saved?: LibraryNoteAnalysis | null; disabled: boolean; resourcesRevision: unknown;
  onChange: (content: AnalysisInput) => void; onPlay: (clip: LibraryClip) => void;
}) {
  const [clips, setClips] = useState<LibraryClip[]>([]), [frames, setFrames] = useState<LibraryKeyframe[]>([]);
  const [error, setError] = useState(''), [loading, setLoading] = useState(false), [reload, setReload] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [preview, setPreview] = useState<string>();
  useEffect(() => {
    if (content.kind !== 'technique') return;
    let disposed = false; setLoading(true); setLoaded(false);
    void Promise.all([libraryApi.clips(assetId), libraryApi.keyframes(assetId)]).then(([clips, frames]) => { if (!disposed) { setClips(clips); setFrames(frames); setError(''); setLoaded(true); } })
      .catch(reason => { if (!disposed) setError(errorText(reason)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [assetId, content.kind, resourcesRevision, reload]);
  if (content.kind === 'copywriting') return <div className="library-analysis-copy" aria-label="文案拆解模板">{Object.entries(copywritingFields).map(([key, label]) => <label className="library-analysis-field" key={key}>{label}<Textarea aria-label={`拆解${label}`} value={content[key as keyof typeof copywritingFields]} maxLength={16384} disabled={disabled} placeholder="未记录" onChange={event => onChange({ ...content, [key]: event.target.value })} /></label>)}</div>;
  const update = (index: number, patch: Partial<ObservationInput>) => onChange({ ...content, observations: content.observations.map((row, i) => i === index ? { ...row, ...patch } : row) });
  return <section className="library-analysis-observations" aria-label="制作手法拆解" aria-busy={loading}>
    <LibraryError message={error} />{error && <Button size="sm" variant="outline" onClick={() => setReload(value => value + 1)}>重新读取引用资源</Button>}
    {loaded && !clips.length && <p className="text-xs text-muted-foreground">暂无片段，可先在视频下方收藏范围。</p>}
    {content.observations.map((row, index) => {
      const clip = clips.find(clip => clip.id === row.clip_id);
      const previous = saved?.content.kind === 'technique' ? saved.content.observations.find(entry => entry.clip_id === row.clip_id) : undefined;
      const range = clip ?? previous;
      const availableFrames = frames.filter(frame => { const time = frame.pts / frame.time_base_den * frame.time_base_num * 1000; return range && frame.file_id === range.file_id && time >= range.start_ms && time < range.end_ms; });
      const frame = frames.find(frame => frame.id === row.keyframe_id);
      return <div className="library-observation" key={index} data-observation={index}>
        <div className="library-analysis-selects"><label>观察维度<Select value={row.dimension} disabled={disabled} onValueChange={value => update(index, { dimension: value as ObservationInput['dimension'] })}><SelectTrigger aria-label={`观察维度 ${index + 1}`}><SelectValue /></SelectTrigger><SelectContent>{Object.entries(observationDimensions).map(([key, name]) => <SelectItem value={key} key={key}>{name}</SelectItem>)}</SelectContent></Select></label>
          <label>观察依据<Select value={row.evidence} disabled={disabled} onValueChange={value => update(index, { evidence: value as ObservationInput['evidence'] })}><SelectTrigger aria-label={`观察依据 ${index + 1}`}><SelectValue /></SelectTrigger><SelectContent>{Object.entries(observationEvidence).map(([key, name]) => <SelectItem value={key} key={key} disabled={key === 'keyframe' && row.dimension === 'sound'}>{name}</SelectItem>)}</SelectContent></Select></label>
        </div>
        <label className="library-analysis-field">引用片段<Select value={row.clip_id || 'none'} disabled={disabled || loading} onValueChange={id => update(index, { clip_id: id === 'none' ? '' : id, keyframe_id: null })}><SelectTrigger aria-label={`引用片段 ${index + 1}`}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="none">选择片段</SelectItem>{!clip && row.clip_id && <SelectItem value={row.clip_id}>{previous?.clip_title ?? '原引用片段'}{loaded ? '（已删除）' : '（尚未读取）'}</SelectItem>}{clips.map(clip => <SelectItem key={clip.id} value={clip.id}>{clip.title} {clipTime(clip.start_ms)}–{clipTime(clip.end_ms)}</SelectItem>)}</SelectContent></Select></label>
        {range && <div className="library-file-actions"><span className="text-xs text-muted-foreground">{clipTime(range.start_ms)}–{clipTime(range.end_ms)}</span><Button size="sm" variant="ghost" disabled={disabled || loading || !!error || !clip || clip.missing || clip.source_changed} onClick={() => { if (clip) onPlay(clip); }}><Play />播放引用片段</Button>{!clip ? <span role="status" className="text-xs text-muted-foreground">{loaded ? '片段已删除，保留原引用' : '尚未读取引用片段'}</span> : (clip.missing || clip.source_changed) && <span role="status" className="text-xs text-muted-foreground">{clip.missing ? '源文件缺失' : '源文件已变化'}</span>}</div>}
        <label className="library-analysis-field">关联关键帧<Select value={row.keyframe_id ?? 'none'} disabled={disabled || loading || !row.clip_id} onValueChange={id => update(index, { keyframe_id: id === 'none' ? null : id })}><SelectTrigger aria-label={`关联关键帧 ${index + 1}`}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="none">不关联关键帧</SelectItem>{row.keyframe_id && !availableFrames.some(frame => frame.id === row.keyframe_id) && <SelectItem value={row.keyframe_id}>原引用关键帧{!frame ? loaded ? '（已删除）' : '（尚未读取）' : ''}</SelectItem>}{availableFrames.map(frame => <SelectItem key={frame.id} value={frame.id}>{clipTime(frame.actual_ms)}（请求 {clipTime(frame.requested_ms)}）</SelectItem>)}</SelectContent></Select></label>
        {row.keyframe_id && <div className="library-file-actions"><Button size="sm" variant="ghost" disabled={!frame || loading || !!error} onClick={() => setPreview(row.keyframe_id!)}>查看引用关键帧</Button>{!frame && loaded && <span role="status" className="text-xs text-muted-foreground">关键帧已删除，保留原引用</span>}</div>}
        {row.evidence === 'keyframe' && <p className="text-xs text-muted-foreground" role="status">仅看关键帧，未核验连续运动。</p>}
        {row.evidence === 'keyframe' && row.dimension === 'sound' && <LibraryError message="声音观察不能仅依据关键帧，请调整观察依据" />}
        <label className="library-analysis-field">观察内容<Textarea aria-label={`观察内容 ${index + 1}`} value={row.text} maxLength={16384} disabled={disabled} onChange={event => update(index, { text: event.target.value })} /></label>
        <Button size="sm" variant="ghost" disabled={disabled} onClick={() => onChange({ ...content, observations: content.observations.filter((_, i) => i !== index) })}><Trash2 />移除观察 {index + 1}</Button>
      </div>;
    })}
    <Button size="sm" variant="outline" disabled={disabled || content.observations.length >= 64} onClick={() => onChange({ ...content, observations: [...content.observations, blankObservation()] })}><Plus />添加观察</Button>
    {preview && <AnalysisFrame assetId={assetId} frameId={preview} close={() => setPreview(undefined)} />}
  </section>;
}
