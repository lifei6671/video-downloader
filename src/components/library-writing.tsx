import { useEffect, useRef, useState } from 'react';
import { Copy, Download, Plus, RotateCcw, Square } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type VideoTranscript } from '@/api';
import { libraryApi, type LibraryAssetDetail, type LibraryCitation, type LibraryFile, type LibraryExportFormat } from '@/lib/library-api';
import { ComparisonSourceView } from '@/components/library-comparison-source';
import { readLibraryDocument, useLibraryWriting } from '@/hooks/use-library-writing';
import type { LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Spinner } from '@/components/ui/spinner';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem, SelectGroup } from '@/components/ui/select';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Progress } from '@/components/ui/progress';
import { LibraryAnalysisEditor } from '@/components/library-analysis-editor';
import { Input } from '@/components/ui/input';
import { LibraryTranscriptReader } from '@/components/library-transcript-reader';
import type { LibraryVideoHandle } from '@/components/library-video';

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
    <Button size="sm" variant="outline" disabled={pending || !file || !running && file.missing} onClick={() => void act()}>{pending ? <Spinner /> : running ? <Square /> : <RotateCcw />}{running ? '取消提取' : snapshot?.has_result || hasResult ? '重新提取原文' : '提取原文'}</Button>
    {running && <div className="library-recognition-progress" role="status">{recognized && <Progress value={percent} aria-label="原文识别进度" />}<span>{recognized ? `已识别 ${snapshot!.completed_segments ?? 0} / ${snapshot!.total_segments} 段` : '正在提取原文…'}</span></div>}
    <LibraryError message={error || (snapshot?.error_code ? snapshot.state === 'canceled' ? '本次提取已取消' : errorText(snapshot.error_code) : '')} />
    {snapshot?.state === 'no_speech' && <p role="status" className="text-xs text-muted-foreground">未识别到语音，已有资料已保留。</p>}
  </div>;
}
const noteNames: Record<string, string> = { comparison: '比较笔记', note: '笔记', inspiration: '灵感', copywriting: '文案学习', technique: '制作手法', copywriting_analysis: '文案拆解', technique_analysis: '制作手法拆解' };
export function CitationReader({ citation, close }: { citation: LibraryCitation; close: () => void }) {
  const [content, setContent] = useState<Awaited<ReturnType<typeof readLibraryDocument>>>();
  const [error, setError] = useState('');
  useEffect(() => { let disposed = false; void readLibraryDocument(citation.asset_id, citation.document_id).then(value => { if (!disposed) setContent(value); }).catch(reason => { if (!disposed) setError(errorText(reason)); }); return () => { disposed = true; }; }, [citation]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="library-citation-dialog"><DialogHeader><DialogTitle>引用原文</DialogTitle><DialogDescription className="sr-only">所引用文案的已保存版本</DialogDescription></DialogHeader><LibraryError message={error} />{content ? <><p className="text-xs text-muted-foreground">{content.document.summary.kind === 'recognized' ? '识别原文' : '整理稿'} v{content.document.summary.version}</p>{content.origin.source_changed && <p className="text-xs text-muted-foreground">来源文件已变化或缺失</p>}<Textarea readOnly aria-label="引用文案全文" value={content.document.text} /></> : !error && <Spinner />}</DialogContent></Dialog>;
}

export function LibraryWriting({ detail, file, active, navigation, onClipCitation, player, ready, onJump, onSelectParagraphs, onPlayClip }: { detail: LibraryAssetDetail; file?: LibraryFile; active: boolean; navigation: LibraryEditorNavigation; onClipCitation?: (citation: LibraryCitation) => void; player: React.RefObject<LibraryVideoHandle | null>; ready: number; onJump: (positionMs: number) => Promise<void>; onSelectParagraphs: (citation: LibraryCitation, range: { start_ms: number; end_ms: number }) => void; onPlayClip: (clip: import('@/lib/library-api').LibraryClip) => void }) {
  const editor = useLibraryWriting(detail, active, navigation);
  const { state, setState } = editor;
  const text = useRef<HTMLTextAreaElement>(null);
  const [selectedQuote, setSelectedQuote] = useState('');
  const [fullText, setFullText] = useState(false);
  const [citation, setCitation] = useState<LibraryCitation | null>(null);
  const [actionError, setActionError] = useState('');
  const [exporting, setExporting] = useState(false);
  const [exportFormat, setExportFormat] = useState<LibraryExportFormat>('txt');
  const [exportPath, setExportPath] = useState('');
  useEffect(() => { setSelectedQuote(''); setActionError(''); setExportPath(''); setFullText(false); }, [state.id, state.mode]);
  const documents = detail.documents.filter(d => d.kind === state.mode).sort((a, b) => b.version - a.version);
  if (state.document && state.mode === 'edited' && !documents.some(d => d.id === state.document!.summary.id) && state.document.summary.kind === 'edited') documents.unshift(state.document.summary);
  const originFile = detail.files.find(f => f.id === state.origin?.summary.file_id);
  const originalSource = state.origin?.source;
  const sourceChanged = state.origin?.source_changed || originFile?.missing || originFile?.changed || !!(originalSource && originFile && (
    originFile.path !== originalSource.path || originFile.size !== originalSource.size || originFile.mtime_ns != null && originFile.mtime_ns !== originalSource.mtime_ns
  ));
  const newer = state.mode === 'notes' ? !!state.note && detail.notes.some(note => note.id === state.id && note.revision > state.note!.revision) : !!state.id && documents.some(d => d.version > (state.document?.summary.version ?? 0));
  async function copy(body: string) { try { await navigator.clipboard.writeText(body); toast.success('已复制'); } catch (reason) { setActionError(errorText(reason)); } }
  async function exportDocument() {
    if (!state.id || exporting || editor.dirty) return;
    setExporting(true); setActionError(''); setExportPath('');
    try { const path = await libraryApi.exportContent(detail.asset.id, { kind: state.mode === 'notes' ? 'note' : 'document', id: state.id }, exportFormat); if (path) setExportPath(path); toast.info(path ? `已导出：${path}` : '已取消导出'); }
    catch (reason) { setActionError(errorText(reason)); }
    finally { setExporting(false); }
  }
  const disabled = state.loading || editor.saving || exporting;
  const timed = state.mode === 'recognized' && !!state.document?.timing?.segments.length;
  const syncAvailable = !disabled && active && !!file && !file.missing && !file.changed && !sourceChanged && !!state.document?.source && state.document.summary.file_id === file.id;

  return <section className="library-writing" aria-label="文案与笔记" aria-busy={state.loading}>
    <Tabs className="library-writing-tabs" value={state.mode} onValueChange={mode => {
      if (mode === 'notes') editor.select('notes', detail.notes[0]?.id ?? null);
      else editor.select(mode as 'recognized' | 'edited', detail.documents.filter(d => d.kind === mode).sort((a, b) => b.version - a.version)[0]?.id ?? null);
    }}><TabsList aria-label="资料类型"><TabsTrigger value="recognized" disabled={editor.saving || exporting}>识别原文</TabsTrigger><TabsTrigger value="edited" disabled={editor.saving || exporting}>整理稿</TabsTrigger><TabsTrigger value="notes" disabled={editor.saving || exporting}>笔记</TabsTrigger></TabsList><TabsContent value={state.mode} className="library-writing-panel">
    <div className="library-writing-toolbar">
      {state.mode === 'notes' ? <><Select value={state.id ?? 'new'} disabled={disabled} onValueChange={id => editor.select('notes', id === 'new' ? null : id)}><SelectTrigger aria-label="选择笔记"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="new">新笔记</SelectItem>{detail.notes.map((note, index) => <SelectItem key={note.id} value={note.id}>{noteNames[note.kind] ?? note.kind} {index + 1} r{note.revision}</SelectItem>)}</SelectContent></Select><Button size="sm" variant="ghost" disabled={disabled} onClick={() => editor.createNote()}><Plus />新建笔记</Button></> : <>
        <Select value={state.id ?? 'new'} disabled={disabled} onValueChange={id => editor.select(state.mode as 'recognized' | 'edited', id === 'new' ? null : id)}><SelectTrigger aria-label="文案版本"><SelectValue /></SelectTrigger><SelectContent>{state.mode === 'edited' && <SelectItem value="new">新整理稿</SelectItem>}{state.mode === 'recognized' && !documents.length && <SelectItem value="new">暂无识别原文</SelectItem>}{documents.map(document => <SelectItem key={document.id} value={document.id}>v{document.version}{document.model_name ? ` ${document.model_name}` : ''}</SelectItem>)}</SelectContent></Select>
        {state.mode === 'recognized' && <Button size="sm" variant="ghost" disabled={disabled || !state.document} onClick={editor.createDraft}>创建整理稿</Button>}
      </>}
      {editor.dirty && <span role="status" className="text-xs text-muted-foreground">未保存</span>}
    </div>
    <LibraryError message={state.error || actionError} />
    {state.conflict ? <Button variant="outline" size="sm" disabled={disabled} onClick={() => void editor.reloadConflict()}>重新读取</Button> : newer && <Button size="sm" variant="outline" disabled={disabled} onClick={() => void editor.reloadConflict()}>读取最新内容</Button>}
    {state.needsRead && <Button size="sm" variant="outline" disabled={disabled} onClick={editor.reloadSaved}>重新读取已保存内容</Button>}
    {state.loading && <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><Spinner />正在读取内容…</p>}
    {state.mode !== 'notes' && state.origin && <div className="library-document-source">
      <span>{state.mode === 'edited' ? '来源原文：' : ''}{state.origin.summary.kind === 'recognized' ? state.origin.summary.model_name ?? '模型未知' : '来源未知'}{state.origin.summary.backend ? ` / ${state.origin.summary.backend}` : ''}</span>
      {sourceChanged && <span role="status">来源文件已变化或缺失，以下为已保存版本。</span>}
      <details><summary>来源信息</summary><p>{state.origin.source?.path ?? '来源文件未知'}</p><p>{state.origin.model_revision ?? '模型版本未知'}</p></details>
    </div>}
    {state.mode === 'notes' && <div className="library-writing-toolbar"><Select value={state.kind} disabled={disabled || !!state.id || !!state.analysis} onValueChange={editor.changeNoteKind}><SelectTrigger aria-label="笔记类型"><SelectValue /></SelectTrigger><SelectContent>{!noteNames[state.kind] && <SelectItem value={state.kind}>{state.kind}</SelectItem>}{Object.entries(noteNames).filter(([id]) => !!state.id || id !== 'comparison').map(([id, name]) => <SelectItem key={id} value={id}>{name}</SelectItem>)}</SelectContent></Select><span className="text-xs text-muted-foreground">创建来源：{state.id && !state.note ? state.loading ? '读取中…' : '未知' : state.noteOrigin === 'ai' ? `AI${state.modelName ? ` / ${state.modelName}` : ''}` : '人工'}</span></div>}
    {state.mode === 'notes' && state.analysis && !state.id && <div className="library-analysis-selects"><label>内容来源<Select value={state.noteOrigin} disabled={disabled} onValueChange={value => setState(old => ({ ...old, noteOrigin: value as 'human' | 'ai' }))}><SelectTrigger aria-label="拆解内容来源"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="human">人工</SelectItem><SelectItem value="ai">AI 结果（手动录入）</SelectItem></SelectContent></Select></label>{state.noteOrigin === 'ai' && <label>模型名称<Input aria-label="拆解模型名称" value={state.modelName} maxLength={60} disabled={disabled} onChange={event => setState(old => ({ ...old, modelName: event.target.value }))} /></label>}</div>}
    {state.note?.analysis?.human_revised && <p role="status" className="text-xs text-muted-foreground">已人工修订，保留 AI 创建来源</p>}
    {timed && <Button size="sm" variant="ghost" onClick={() => { setFullText(value => !value); setSelectedQuote(''); }}>{fullText ? '同步阅读' : '阅读全文'}</Button>}
    {state.mode === 'notes' && state.comparison && state.note?.comparison ? <div className="comparison-notes"><label>标题<Input aria-label="比较标题" disabled={disabled} value={state.comparison.title} onChange={e => setState(old => ({ ...old, comparison: { ...old.comparison!, title: e.target.value } }))} /></label>{state.note.comparison.points.map((p, i) => <div className="comparison-point" key={i}><Textarea aria-label={`比较结论 ${i + 1}`} disabled={disabled} value={state.comparison!.point_texts[i]} onChange={e => setState(old => ({ ...old, comparison: { ...old.comparison!, point_texts: old.comparison!.point_texts.map((t, j) => j === i ? e.target.value : t) } }))} />{p.sources.map(s => <ComparisonSourceView key={s.pin.asset_id} source={s} />)}</div>)}</div> : state.mode === 'notes' && state.analysis ? <LibraryAnalysisEditor assetId={detail.asset.id} content={state.analysis} saved={state.note?.analysis} disabled={disabled} resourcesRevision={detail} onChange={analysis => setState(old => ({ ...old, analysis }))} onPlay={onPlayClip} /> : timed && !fullText && state.document ? <LibraryTranscriptReader key={state.document.summary.id} document={state.document} available={syncAvailable} unavailableReason={sourceChanged ? '来源文件已变化或缺失，无法同步播放' : file?.id !== state.document.summary.file_id ? '请选择文案对应的文件版本' : '来源文件不可用'} player={player} ready={ready} onJump={onJump} onSelect={onSelectParagraphs} /> : <Textarea ref={text} className="library-writing-text" aria-label={state.mode === 'recognized' ? '识别原文全文' : state.mode === 'edited' ? '整理稿内容' : '笔记内容'} readOnly={state.mode === 'recognized'} disabled={disabled || state.mode !== 'recognized' && !!state.id && !state.document && !state.note} value={state.body} placeholder={state.mode === 'recognized' ? '暂无识别原文' : state.mode === 'edited' ? '输入整理稿内容' : '记录学习心得'} onChange={event => setState(old => ({ ...old, body: event.target.value }))} onSelect={() => { const el = text.current; setSelectedQuote(el && state.mode !== 'notes' ? el.value.slice(el.selectionStart, el.selectionEnd) : ''); }} />}
    {state.mode !== 'notes' && selectedQuote && state.document && !editor.dirty && <Button size="sm" variant="outline" disabled={disabled} onClick={() => editor.createNote({ asset_id: detail.asset.id, document_id: state.document!.summary.id, quote: selectedQuote })}>引用所选文字写笔记</Button>}
    {onClipCitation && state.mode !== 'notes' && selectedQuote.trim() && state.document && !editor.dirty && <Button size="sm" variant="outline" disabled={disabled || !file || file.missing || file.changed || !!sourceChanged || !!originFile && originFile.id !== file.id} onClick={() => onClipCitation({ asset_id: detail.asset.id, document_id: state.document!.summary.id, quote: selectedQuote })}>引用所选文字收藏片段</Button>}
    {state.mode === 'notes' && state.citations.map((entry, index) => <blockquote className="library-note-citation" key={index}><p>{entry.quote}</p><Button size="sm" variant="ghost" onClick={() => setCitation(entry)}>查看引用原文</Button></blockquote>)}
    <div className="library-writing-actions"><Button variant="outline" size="sm" disabled={disabled || !state.body || (!!state.analysis || !!state.comparison) && (editor.dirty || state.needsRead)} onClick={() => void copy(state.body)}><Copy />复制全文</Button><Select value={exportFormat} disabled={disabled || editor.dirty} onValueChange={value => setExportFormat(value as LibraryExportFormat)}><SelectTrigger size="sm" aria-label="导出格式"><SelectValue /></SelectTrigger><SelectContent><SelectGroup><SelectItem value="txt">TXT</SelectItem><SelectItem value="markdown">Markdown</SelectItem><SelectItem value="json">JSON</SelectItem></SelectGroup></SelectContent></Select><Button variant="outline" size="sm" disabled={disabled || !state.id || editor.dirty} onClick={() => void exportDocument()}>{exporting ? <Spinner /> : <Download />}导出 {exportFormat === 'markdown' ? 'Markdown' : exportFormat.toUpperCase()}</Button>{state.mode !== 'recognized' && <Button size="sm" disabled={disabled || !editor.canSave} onClick={() => void editor.save()}>{editor.saving && <Spinner />}{editor.saving ? '保存中…' : '保存'}</Button>}</div>
    {editor.recovered.length > 0 && <details className="library-recovered-drafts"><summary>保留的未保存内容</summary>{editor.recovered.map((body, index) => <div key={index}><Textarea readOnly aria-label={`保留内容 ${index + 1}`} value={body} /><Button size="sm" variant="outline" onClick={() => void copy(body)}>复制保留内容 {index + 1}</Button></div>)}</details>}
    {exportPath && <p role="status" className="library-transfer-path">已导出：{exportPath}</p>}
    {state.mode === 'recognized' && <LibraryRecognition assetId={detail.asset.id} file={file} active={active} hasResult={detail.documents.some(d => d.kind === 'recognized' && d.file_id === file?.id)} />}
    {citation && <CitationReader citation={citation} close={() => setCitation(null)} />}
    </TabsContent></Tabs>
  </section>;
}
