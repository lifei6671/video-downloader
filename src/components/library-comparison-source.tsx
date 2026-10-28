import { useEffect, useRef, useState } from 'react';
import { libraryApi, type ComparisonSource, type ComparisonAnalysis, type LibraryClip } from '@/lib/library-api';
import { errorText } from '@/api';
import { LibraryError } from '@/components/library-organization';
import { LibraryVideo } from '@/components/library-video';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';

const unknown = '未知';
const fields = ['开头', '观点', '论据', '转折', '结尾', '可借鉴点'];
const missing = () => {};
export function AnalysisSnapshot({ value }: { value: ComparisonAnalysis | null }) {
  if (!value) return <p>{unknown}</p>;
  return <><p className="text-xs text-muted-foreground">r{value.revision} {value.origin === 'ai' ? `AI / ${value.model_name ?? unknown}` : '人工'}{value.human_revised ? ' / 已人工修订' : ''}</p>{value.structure ? <dl>{fields.map((label, i) => <div key={label}><dt>{label}</dt><dd>{value.structure?.[i] || unknown}</dd></div>)}</dl> : <p className="whitespace-pre-wrap">{value.excerpt || unknown}</p>}</>;
}
type Target = { kind: 'document' | 'copywriting' | 'technique' } | { kind: 'clip'; id: string };
export function ComparisonSourceView({ source, compact = false }: { source: ComparisonSource; compact?: boolean }) {
  const [target, setTarget] = useState<Target | null>(null);
  const trigger = useRef<HTMLElement | null>(null);
  function open(next: Target) { trigger.current = document.activeElement as HTMLElement; setTarget(next); }
  function close() { setTarget(null); requestAnimationFrame(() => { if (trigger.current?.isConnected) trigger.current.focus({ preventScroll: true }); }); }
  return <div className="comparison-source-links" data-comparison-source={source.pin.asset_id}>{!compact && <strong title={source.title}>{source.title || '未命名视频'}</strong>}
    {source.document && <Button size="sm" variant="outline" onClick={() => open({ kind: 'document' })}>原文 v{source.document.version}</Button>}
    {(['copywriting', 'technique'] as const).map(kind => source[kind] && <Button key={kind} size="sm" variant="outline" onClick={() => open({ kind })}>{kind === 'copywriting' ? '文案结构' : '制作手法'} r{source[kind]!.revision}</Button>)}
    {source.clips.map(c => <Button key={c.id} size="sm" variant="outline" onClick={() => open({ kind: 'clip', id: c.id })}>{c.title} {c.start_ms}–{c.end_ms} ms</Button>)}
    {!source.document && !source.copywriting && !source.technique && !source.clips.length && <span>来源维度未知</span>}
    {target && <SourceDialog key={JSON.stringify(target)} source={source} target={target} close={close} />}
  </div>;
}
function SourceDialog({ source, target, close }: { source: ComparisonSource; target: Target; close: () => void }) {
  const [text, setText] = useState('');
  const [status, setStatus] = useState('读取中…');
  const [error, setError] = useState('');
  const [clip, setClip] = useState<LibraryClip>();
  useEffect(() => {
    let disposed = false;
    async function read() {
      try {
        if (target.kind === 'document') {
          const saved = source.document!;
          const doc = await libraryApi.document(source.pin.asset_id, saved.id);
          if (doc.summary.asset_id !== source.pin.asset_id || doc.summary.id !== saved.id || doc.summary.version !== saved.version) throw Error('来源版本不匹配');
          if (!disposed) { setText(doc.text); setStatus(`原文 v${saved.version}`); }
        } else if (target.kind === 'clip') {
          const saved = source.clips.find(c => c.id === target.id)!;
          const clips = await libraryApi.clips(source.pin.asset_id);
          const live = clips.find(c => c.id === saved.id && c.asset_id === source.pin.asset_id);
          if (!live) throw Error('片段已删除，保留保存时的范围');
          if (live.file_id !== saved.file_id || live.start_ms !== saved.start_ms || live.end_ms !== saved.end_ms) throw Error('片段来源不匹配');
          if (!disposed) { setClip(live); setText(saved.quote ?? '引用文案未知'); setStatus(`${saved.title} ${saved.start_ms}–${saved.end_ms} ms`); }
        } else {
          const saved = source[target.kind]!;
          // Display the captured excerpt, even if a newer revision exists.
          if (!disposed) setText(saved.excerpt);
          const note = await libraryApi.note(source.pin.asset_id, saved.id);
          if (note.asset_id !== source.pin.asset_id || note.id !== saved.id) throw Error('来源资产不匹配');
          if (!disposed) setStatus(note.revision === saved.revision ? `拆解 r${saved.revision}（保存时摘录）` : `保存时 r${saved.revision}，当前 r${note.revision}`);
        }
      } catch (reason) { if (!disposed) { setError(errorText(reason)); setStatus('来源读取失败'); } }
    }
    void read(); return () => { disposed = true; };
  }, [source, target]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="comparison-source-dialog"><DialogHeader><DialogTitle>来源：{source.title || '未命名视频'}</DialogTitle></DialogHeader><span className="text-xs text-muted-foreground">{source.pin.asset_id}</span><p role="status">{status}</p><LibraryError message={error} />{target.kind === 'clip' && clip && <LibraryVideo assetId={source.pin.asset_id} fileId={clip.file_id} clipId={clip.id} onMissing={missing} />}<pre className="comparison-source-text">{text}</pre></DialogContent></Dialog>;
}
