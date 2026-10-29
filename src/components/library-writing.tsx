import { useEffect, useRef, useState } from 'react';
import { RotateCcw, Square } from 'lucide-react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { api, errorText, type VideoTranscript } from '@/api';
import { type LibraryAssetDetail, type LibraryCitation, type LibraryFile, type LibraryWritingSelection } from '@/lib/library-api';
import { readLibraryDocument, useLibraryWriting } from '@/hooks/use-library-writing';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Spinner } from '@/components/ui/spinner';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Progress } from '@/components/ui/progress';

function LibraryRecognition({ assetId, file, active, hasResult }: { assetId: string; file?: LibraryFile; active: boolean; hasResult: boolean }) {
  const [snapshot, setSnapshot] = useState<VideoTranscript | null>(null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const fileId = file?.id;
  const scope = useRef(0);
  useEffect(() => {
    scope.current++;
    setPending(false);
    if (!active || !fileId) return;
    let disposed = false; let stop: (() => void) | undefined;
    setSnapshot(null); setError('');
    const merge = (next: VideoTranscript) => { if (!disposed && next.asset_id === assetId && next.file_id === fileId) setSnapshot(old => !old || old.sequence <= next.sequence ? next : old); };
    void api.subscribeAssetTranscripts(merge).then(async unsubscribe => {
      if (disposed) { unsubscribe(); return; } stop = unsubscribe;
      const value = await api.assetTranscript(assetId, fileId); if (value) merge(value);
    }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; scope.current++; stop?.(); };
  }, [assetId, fileId, active]);
  const running = snapshot?.state === 'running';
  const recognized = snapshot?.phase === 'recognizing' && !!snapshot.total_segments;
  const percent = recognized ? Math.floor((snapshot!.completed_segments ?? 0) / snapshot!.total_segments! * 100) : null;
  async function act() {
    if (!fileId || pending) return;
    const token = scope.current;
    setPending(true); setError('');
    try {
      if (running) await api.cancelAssetTranscription(fileId, snapshot.operation_id);
      else { const next = await api.startAssetTranscription(assetId, fileId, true); if (token === scope.current) setSnapshot(old => !old || old.sequence <= next.sequence ? next : old); }
    } catch (reason) { if (token === scope.current) setError(errorText(reason)); }
    finally { if (token === scope.current) setPending(false); }
  }
  return <div className="library-recognition">
    <Button size="sm" variant="outline" disabled={pending || !file || !running && (file.missing || file.changed)} onClick={() => void act()}>{pending ? <Spinner /> : running ? <Square /> : <RotateCcw />}{running ? '取消提取' : snapshot?.has_result || hasResult ? '重新提取原文' : '提取原文'}</Button>
    {running && <div className="library-recognition-progress" role="status">{recognized && <Progress value={percent} aria-label="原文识别进度" />}<span>{recognized ? `已识别 ${snapshot!.completed_segments ?? 0} / ${snapshot!.total_segments} 段` : '正在提取原文…'}</span></div>}
    <LibraryError message={error || (snapshot?.error_code ? snapshot.state === 'canceled' ? '本次提取已取消' : errorText(snapshot.error_code) : '')} />
    {snapshot?.state === 'no_speech' && <p role="status" className="text-xs text-muted-foreground">未识别到语音，已有资料已保留。</p>}
  </div>;
}
export function CitationReader({ citation, close }: { citation: LibraryCitation; close: () => void }) {
  const [content, setContent] = useState<Awaited<ReturnType<typeof readLibraryDocument>>>();
  const [error, setError] = useState('');
  useEffect(() => { let disposed = false; void readLibraryDocument(citation.asset_id, citation.document_id).then(value => { if (!disposed) setContent(value); }).catch(reason => { if (!disposed) setError(errorText(reason)); }); return () => { disposed = true; }; }, [citation]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="library-citation-dialog"><DialogHeader><DialogTitle>引用原文</DialogTitle><DialogDescription className="sr-only">所引用文案的已保存版本</DialogDescription></DialogHeader><LibraryError message={error} />{content ? <><p className="text-xs text-muted-foreground">{content.document.summary.kind === 'recognized' ? '识别原文' : '整理稿'} v{content.document.summary.version}</p>{content.origin.source_changed && <p className="text-xs text-muted-foreground">来源文件已变化或缺失</p>}<Textarea readOnly aria-label="引用文案全文" value={content.document.text} /></> : !error && <Spinner />}</DialogContent></Dialog>;
}

export function LibraryWriting({ initialSelection, detail, file, active, navigation }: { initialSelection?: LibraryWritingSelection; detail: LibraryAssetDetail; file?: LibraryFile; active: boolean; navigation: LibraryEditorNavigation }) {
  const editor = useLibraryWriting(detail, active, navigation, initialSelection);
  const { state, setState } = editor;
  const [preview, setPreview] = useState(false);
  useEffect(() => { setPreview(false); }, [state.mode, state.id]);
  const disabled = state.loading || editor.saving;
  const original = state.mode === 'original';
  const notes = [...detail.notes].sort((a, b) => b.updated_at - a.updated_at);
  // The just-saved summary may arrive before the asset change event.
  if (state.note && !notes.some(note => note.id === state.note!.id)) notes.unshift({ ...state.note, characters: state.note.body.length, citation_count: state.note.citations.length });

  return <section className="library-writing" aria-label="素材文案" aria-busy={state.loading}>
    <Tabs className="library-writing-tabs" value={state.mode} onValueChange={mode => editor.select(mode as 'original' | 'summary')}>
      <TabsList aria-label="资料类型"><TabsTrigger value="original" disabled={editor.saving}>原始文案</TabsTrigger><TabsTrigger value="summary" disabled={editor.saving}>AI 总结</TabsTrigger></TabsList>
      <TabsContent value={state.mode} className="library-writing-panel">
        <div className="library-writing-toolbar">
          {!original && (notes.length > 1 || notes.length === 1 && !state.id) && <Select value={state.id ?? 'new'} disabled={disabled} onValueChange={id => editor.select('summary', id === 'new' ? null : id)}><SelectTrigger aria-label="选择总结内容"><SelectValue placeholder="选择内容" /></SelectTrigger><SelectContent><SelectItem value="new">新总结</SelectItem>{notes.map((note, index) => <SelectItem key={note.id} value={note.id}>{note.origin === 'ai' ? 'AI 总结' : ['copywriting', 'technique'].includes(note.kind) ? '总结' : '历史内容'} {index + 1}</SelectItem>)}</SelectContent></Select>}
          {!original && <Button size="sm" variant="ghost" disabled={disabled || !state.body} aria-pressed={preview} onClick={() => setPreview(value => !value)}>{preview ? '编辑' : '预览'}</Button>}
          {editor.dirty && <span role="status" className="text-xs text-muted-foreground">未保存</span>}
        </div>
        <LibraryError message={state.error} />
        {(state.conflict || editor.newer) && <Button variant="outline" size="sm" disabled={disabled} onClick={() => state.conflict ? void editor.reloadConflict() : editor.select(state.mode)}>读取最新内容</Button>}
        {state.needsRead && <Button size="sm" variant="outline" disabled={disabled} onClick={editor.reloadSaved}>重新读取已保存内容</Button>}
        {state.loading && <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><Spinner />正在读取内容…</p>}
        {!original && preview ? <div className="library-summary-preview" aria-label="AI 总结预览" tabIndex={0}><Markdown skipHtml remarkPlugins={[remarkGfm]} urlTransform={url => /^https?:\/\//i.test(url) ? url : undefined} components={{ a: ({ children }) => <span>{children}</span>, img: ({ alt }) => <span>{alt}</span> }}>{state.body}</Markdown></div> : <Textarea className="library-writing-text" aria-label={original ? '原始文案内容' : 'AI 总结内容'} disabled={disabled || state.needsRead} value={state.body} placeholder={original ? '提取视频文案后，可在此修改' : '填写 AI 对写作、表达或画面分镜手法的总结，支持 Markdown'} onChange={event => setState(old => ({ ...old, body: event.target.value }))} />}
        <div className="library-writing-actions">{original && <LibraryRecognition assetId={detail.asset.id} file={file} active={active} hasResult={detail.documents.some(d => d.kind === 'recognized' && d.file_id === file?.id)} />}<Button size="sm" disabled={disabled || !editor.canSave} onClick={() => void editor.save()}>{editor.saving && <Spinner />}{editor.saving ? '保存中…' : '保存'}</Button></div>
        {editor.recovered.length > 0 && <details className="library-recovered-drafts" open><summary>保留的未保存内容</summary>{editor.recovered.map((body, index) => <div key={index}><Textarea readOnly aria-label={`保留内容 ${index + 1}`} value={body} /><Button size="sm" variant="outline" onClick={() => editor.restore(body)}>恢复本次输入</Button></div>)}</details>}
      </TabsContent>
    </Tabs>
  </section>;
}
