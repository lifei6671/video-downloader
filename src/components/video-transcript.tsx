import { useEffect, useRef, useState } from 'react';
import { AlertCircle, Check, Copy, Download, FileText, RotateCcw, Square } from 'lucide-react';
import { api, errorText, type Task, type BackupAsset, type AssetTranscript, type VideoTranscript } from '@/api';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Spinner } from '@/components/ui/spinner';
import { Progress } from '@/components/ui/progress';
import { Textarea } from '@/components/ui/textarea';
import { ModelVersionSelect, type useRuntimeDependencies } from '@/components/runtime-dependencies';

export function useVideoTranscripts() {
  const [values, setValues] = useState<Record<string, VideoTranscript>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState('');
  const removed = useRef(new Set<string>());
  const trigger = useRef<HTMLElement | null>(null);
  function merge(value: VideoTranscript) {
    setValues(old => removed.current.has(value.task_id) || (old[value.task_id]?.sequence ?? -1) > value.sequence ? old : {...old, [value.task_id]: value});
  }
  useEffect(() => {
    let disposed = false, unsubscribe: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await api.subscribeTranscripts(value => { if (!disposed) merge(value); });
        if (disposed) { stop(); return; } unsubscribe = stop;
        const snapshots = await api.transcripts();
        if (!disposed) snapshots.forEach(merge);
      } catch (e) { if (!disposed) setError(String(e)); }
    })();
    return () => { disposed = true; unsubscribe?.(); };
  }, []);
  function open(id: string, returnFocus?: HTMLElement | null) { trigger.current = returnFocus ?? document.activeElement as HTMLElement; setError(''); setSelected(id); }
  function remove(ids: string[]) {
    ids.forEach(id => removed.current.add(id));
    setValues(old => Object.fromEntries(Object.entries(old).filter(([id]) => !removed.current.has(id))));
    setSelected(old => old && ids.includes(old) ? null : old);
  }
  return {values, selected, setSelected, error, setError, merge, open, remove, trigger};
}

export function VideoTranscriptDialog({client, tasks, assets = [], runtime}: {client: ReturnType<typeof useVideoTranscripts>; tasks: Task[]; assets?: BackupAsset[]; runtime: ReturnType<typeof useRuntimeDependencies>}) {
  const asset = client.selected ? assets.find(a => a.asset_id === client.selected || a.task_id === client.selected) : undefined;
  const originalTask = tasks.find(t => t.id === client.selected);
  const task = (originalTask && asset ? { ...originalTask, file_missing: asset.remote_deleted && !asset.path } : originalTask) ?? (asset ? {id: asset.asset_id, video: {title: asset.metadata.title}, file_missing: asset.remote_deleted && !asset.path} : undefined);
  const [cloudText, setCloudText] = useState<AssetTranscript | null>(null);
  const transcriptionId = asset?.asset_id ?? client.selected;
  const summary = transcriptionId ? client.values[transcriptionId] ?? (client.selected ? client.values[client.selected] : undefined) : undefined;
  const [detail, setDetail] = useState<VideoTranscript | null>(null);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState('');
  const [reading, setReading] = useState(false);
  const [copied, setCopied] = useState(false);
  const copyResetTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const copyScope = useRef({active: true});
  useEffect(() => {
    const scope = {active: true};
    copyScope.current = scope;
    setCopied(false);
    return () => { scope.active = false; clearTimeout(copyResetTimer.current); };
  }, [client.selected]);
  useEffect(() => {
    let disposed = false;
    setDetail(null); setCloudText(null); setNotice('');
    if (client.selected) {
      setReading(true);
      void api.transcript(transcriptionId!).then(async value => { if (!disposed) setDetail(value); if (asset && (asset.has_transcript || value?.has_result)) { const text = await api.assetTranscript(asset.space_id, asset.asset_id); if (!disposed) setCloudText(text); } })
        .catch(e => { if (!disposed) client.setError(String(e)); }).finally(() => { if (!disposed) setReading(false); });
    }
    return () => { disposed = true; };
  }, [client.selected, summary?.state === 'running' ? null : summary?.sequence, asset?.remote_revision]);
  const state = summary ?? detail;
  const running = state?.state === 'running';
  const other = Object.values(client.values).find(v => v.state === 'running' && v.task_id !== transcriptionId);
  const model = runtime.status?.tools.find(t => t.id === 'fun-asr-nano');
  const ffmpeg = runtime.status?.tools.find(t => t.id === 'ffmpeg');
  const ready = model?.available && ffmpeg?.available;
  async function act(action: () => Promise<void>) {
    setPending(true); client.setError(''); setNotice('');
    try { await action(); } catch (e) { client.setError(String(e)); } finally { setPending(false); }
  }
  const progress = runtime.progress;
  const percent = progress && progress.total_bytes > 0 ? Math.floor(progress.downloaded_bytes / progress.total_bytes * 100) : null;
  const installing = runtime.installing !== null;
  const text = cloudText?.text ?? detail?.result?.text;
  const backend = running ? state.backend : detail?.result ? detail.result.backend ?? 'cpu' : model?.provider;
  const modelName = running || !detail?.result ? 'Fun-ASR-Nano' : detail.result.model_name ?? 'SenseVoiceSmall';
  const recognized = state?.phase === 'recognizing' && state.total_segments != null && state.total_segments > 0;
  const recognitionPercent = recognized ? Math.floor((state.completed_segments ?? 0) / state.total_segments! * 100) : null;
  return <Dialog open={!!task} onOpenChange={open => { if (!open) client.setSelected(null); }}>
    <DialogContent className="transcript-dialog flex max-h-[85dvh] max-w-[calc(100vw-32px)] flex-col sm:max-w-[720px]" onCloseAutoFocus={event => {
      event.preventDefault(); const target = client.trigger.current;
      (target?.isConnected ? target : document.querySelector<HTMLElement>('[aria-current="page"]'))?.focus({preventScroll:true});
    }}>
      <DialogHeader className="min-w-0 shrink-0"><DialogTitle>视频文案</DialogTitle><DialogDescription className="line-clamp-2">{task?.video?.title || '已下载视频'}，{modelName} {backend === 'cuda' ? 'CUDA 加速识别' : backend === 'cpu' ? 'CPU 本地识别' : '本地识别'}</DialogDescription></DialogHeader>
      <div className="flex min-h-0 min-w-0 flex-col gap-3 overflow-y-auto pr-3 [&>*]:shrink-0 [&>textarea]:shrink">
        {client.error && <Alert variant="destructive" role="alert"><AlertCircle /><AlertDescription>{errorText(client.error)}</AlertDescription></Alert>}
        {runtime.error && <Alert variant="destructive" role="alert"><AlertCircle /><AlertDescription>{errorText(runtime.error)}</AlertDescription></Alert>}
        {notice && <p role="status" className="break-all text-xs text-muted-foreground">{notice}</p>}
        {state?.error_code && <Alert variant="destructive" role="alert"><AlertCircle /><AlertDescription>{state.error_code === 'CANCELED' ? '本次文案提取已取消' : errorText(state.error_code)}</AlertDescription></Alert>}
        {state?.state === 'no_speech' && <Alert><AlertCircle /><AlertDescription>本次未识别到可用语音{state.has_result ? '，以下为上次保存的文案' : ''}。</AlertDescription></Alert>}
        {detail?.source_changed && <Alert><AlertCircle /><AlertDescription>这是上次提取的文案，本地视频已变化或缺失。</AlertDescription></Alert>}
        {/* Keep the rotating spinner's bounds out of the surrounding scroll area. */}
        {running && <div className="flex flex-col gap-2" role="status">{recognized && <Progress aria-label="文案识别进度" value={recognitionPercent} />}<p className="flex flex-wrap items-center gap-2 overflow-clip text-xs text-muted-foreground"><Spinner aria-hidden="true" />{state.phase === 'extracting_audio' ? '正在提取音频…' : state.phase === 'segmenting' ? '正在检测语音片段…' : state.phase === 'loading_model' ? '正在加载语音模型…' : recognized ? `已识别 ${state.completed_segments ?? 0} / ${state.total_segments} 段，${recognitionPercent}%` : '正在完成文案提取…'}关闭此窗口后仍会继续处理。</p></div>}
        {reading && <p role="status" className="flex items-center gap-2 overflow-clip text-xs text-muted-foreground"><Spinner />正在读取文案…</p>}
        {text && <><Textarea readOnly aria-label="识别文案" value={text} className="min-h-40 min-w-0 resize-none overflow-y-auto [overflow-wrap:anywhere] rounded-md border-[0.5px] bg-background p-4 text-sm leading-relaxed" /><p className="text-xs text-muted-foreground">提取时间：{new Date((cloudText?.completed_at ?? detail?.result?.completed_at ?? 0) * 1000).toLocaleString('zh-CN')}{state?.state !== 'completed' ? '，上次成功保存的文案' : ''}</p></>}
        {!running && !ready && !task?.file_missing && <div className="flex flex-col gap-3"><p className="text-xs text-muted-foreground">首次使用需要下载所选模型及缺失的 FFmpeg；下载后可离线提取文案。</p>
          {!runtime.status && <Button variant="outline" disabled={runtime.busy} onClick={() => void runtime.check()}>检测运行依赖</Button>}
          {model && !model.available && <div className="flex flex-wrap items-center gap-2"><ModelVersionSelect runtime={runtime} disabled={!!other} /><Button variant="outline" disabled={runtime.busy || !!other || !model.can_install} onClick={() => void runtime.install('fun-asr-nano')}><Download />下载或修复语音模型</Button></div>}
          {ffmpeg && !ffmpeg.available && <Button variant="outline" disabled={runtime.busy || !!other || !ffmpeg.can_install} onClick={() => void runtime.install('ffmpeg')}><Download />下载 FFmpeg</Button>}
          {model && !model.can_install && !model.available && <p className="text-xs text-muted-foreground">当前应用的原生语音程序不可用，请检查安装包是否完整。</p>}
        </div>}
        {installing && <div className="flex flex-col gap-2" role="status"><Progress aria-label="依赖下载进度" value={percent} /><p className="text-xs text-muted-foreground">{progress?.phase === 'downloading' ? '正在下载依赖' : '正在校验与验证依赖'}{progress && progress.total_bytes > 0 ? `，${(progress.downloaded_bytes/1048576).toFixed(1)} / ${(progress.total_bytes/1048576).toFixed(1)} MiB` : ''}</p><Button variant="outline" disabled={runtime.canceling} onClick={() => void runtime.cancel()}><Square />取消下载</Button></div>}
        {other && <Button variant="outline" onClick={() => client.open(other.task_id)}>查看正在提取的视频</Button>}
        {task?.file_missing && <p className="text-xs text-muted-foreground">视频已移动或删除，已有文案仍可复制和导出。</p>}
      </div>
      <DialogFooter className="shrink-0 flex-wrap">
        <Button variant="outline" className="group/copy" data-copied={copied} aria-label={copied ? '已复制' : '复制全文'} disabled={pending || !text} onClick={() => void act(async () => {
          const scope = copyScope.current;
          setCopied(false); clearTimeout(copyResetTimer.current);
          await (asset ? api.copyAssetTranscript(asset.asset_id) : api.copyTranscript(task!.id));
          if (!scope.active) return;
          setCopied(true);
          copyResetTimer.current = setTimeout(() => setCopied(false), 1800);
        })}>
          <span aria-hidden="true" className="grid">
            <span className="col-start-1 row-start-1 inline-flex items-center justify-center gap-1.5 transition-opacity duration-250 ease-out group-data-[copied=true]/copy:opacity-0 motion-reduce:transition-none"><Copy />复制全文</span>
            <span className="col-start-1 row-start-1 inline-flex items-center justify-center gap-1.5 opacity-0 transition-opacity duration-250 ease-out group-data-[copied=true]/copy:opacity-100 motion-reduce:transition-none"><Check />已复制</span>
          </span>
        </Button>
        <Button variant="outline" disabled={pending || !text} onClick={() => void act(async () => { const path = await (asset ? api.exportAssetTranscript(asset.asset_id) : api.exportTranscript(task!.id)); if (path) setNotice(`已导出：${path}`); })}><Download />导出 TXT</Button>
        {running ? <Button variant="outline" disabled={pending} onClick={() => void act(async () => { await api.cancelTranscription(transcriptionId!, state.operation_id); })}><Square />取消提取</Button> :
          <Button disabled={pending || reading || !ready || runtime.busy || !!other || task?.file_missing} onClick={() => void act(async () => { client.merge(await (asset ? api.startAssetTranscription(asset.asset_id, !!state || !!cloudText) : api.startTranscription(task!.id, !!state))); })}>{state ? <RotateCcw /> : <FileText />}{state ? '重新提取' : '开始提取'}</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
