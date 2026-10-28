import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, Copy, ExternalLink, FolderOpen, Play, RotateCcw } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText } from '@/api';
import { displayPath } from '@/lib/path';
import { libraryApi, type LibraryAssetDetail, type LibraryCategory, type LibraryFile, type LibraryTag, type LibraryClip, type LibraryCitation, type IdeaReferenceInput } from '@/lib/library-api';
import { LibraryCover } from '@/components/library-cover';
import { LibraryWriting } from '@/components/library-writing';
import { useLibraryDetailNavigation, type LibraryEditorNavigation } from '@/components/library-editor-navigation';
import { LibraryClips, type ClipDraft } from '@/components/library-clips';
import { LibraryKeyframes } from '@/components/library-keyframes';
import { useLibraryKeyframes } from '@/hooks/use-library-keyframes';
import { LibraryVideo, type LibraryVideoHandle } from '@/components/library-video';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Spinner } from '@/components/ui/spinner';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';

const platformNames: Record<string, string> = { douyin: '抖音', tiktok: 'TikTok', youtube: 'YouTube', telegram: 'Telegram' };
const filename = (file: LibraryFile) => displayPath(file.path).split(/[\\/]/).pop() || '未命名文件';
function size(bytes: number) { return bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`; }
function duration(ms: number | null) { return ms == null ? '未知' : `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`; }
function FileStatus({ file }: { file: LibraryFile }) { return <Badge variant="outline">{file.missing ? '文件缺失' : file.changed ? '文件已变化' : '文件可用'}</Badge>; }

export function LibraryAssetDetailView({ onCreateIdea, backLabel = '返回列表', assetId, entryId = 0, initialFileId, initialError, active, revision, categories, tags, back, organize, navigation }: {
  onCreateIdea?: (input: IdeaReferenceInput) => void; backLabel?: string; assetId: string; entryId?: number; initialFileId?: string; initialError?: string; active: boolean; revision: number; categories: LibraryCategory[]; tags: LibraryTag[];
  navigation: LibraryEditorNavigation; back: () => void; organize: (trigger: HTMLElement) => void;
}) {
  const [detail, setDetail] = useState<LibraryAssetDetail>();
  const [selectedFile, setSelectedFile] = useState(initialFileId ?? '');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const visited = useRef(false);
  const [visitError, setVisitError] = useState('');
  async function recordVisit() {
    try { await libraryApi.visit(assetId); setVisitError(''); }
    catch (reason) { setVisitError(`访问记录保存失败：${errorText(reason)}`); }
  }
  useEffect(() => { visited.current = false; }, [active, assetId]);
  const [actionError, setActionError] = useState(initialError ? errorText(initialError) : '');
  useEffect(() => { if (initialFileId) setSelectedFile(initialFileId); setActionError(initialError ? errorText(initialError) : ''); }, [entryId, initialFileId, initialError]);
  const [refresh, setRefresh] = useState(0);
  const frames = useLibraryKeyframes(assetId, active, revision + refresh);
  const [playback, setPlayback] = useState<{ key: string; clipId?: string; positionMs?: number; attempt: number }>();
  const [clipDraft, setClipDraft] = useState<ClipDraft | null>(null);
  const [playerReady, setPlayerReady] = useState(0);
  const player = useRef<LibraryVideoHandle>(null);
  const editors = useLibraryDetailNavigation(navigation);
  const onPlayerReady = useCallback(() => setPlayerReady(value => value + 1), []);
  const [busy, setBusy] = useState(false);
  const [relinking, setRelinking] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const refreshFileState = useCallback(() => setRefresh(value => value + 1), []);
  const heading = useRef<HTMLHeadingElement>(null);
  const media = useRef<HTMLDivElement>(null);
  useEffect(() => { if (active) heading.current?.focus(); }, [active, entryId]);
  useEffect(() => { if (!active) setPlayback(undefined); }, [active]);
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    setLoading(true); setError('');
    void libraryApi.asset(assetId).then(next => {
      if (disposed) return;
      setDetail(next);
      if (!visited.current) { visited.current = true; void recordVisit(); }
      setSelectedFile(old => next.files.some(file => file.id === old) ? old : (next.files.find(file => !file.missing) ?? next.files[0])?.id ?? '');
    }).catch(reason => { if (!disposed) { setError(errorText(reason)); if (String(reason) === 'ASSET_NOT_FOUND') { setDetail(undefined); setPlayback(undefined); } } })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [assetId, active, revision, refresh]);
  const file = detail?.files.find(file => file.id === selectedFile);
  async function external(action: 'open' | 'reveal') {
    if (!file || file.missing || busy || relinking) return;
    setBusy(true); setActionError(''); if (action === 'open') { media.current?.querySelector('video')?.pause(); setPlayback(undefined); }
    try { await libraryApi[action](assetId, file.id); toast.success(action === 'open' ? '已使用系统播放器打开' : '已定位文件'); }
    catch (reason) { setActionError(errorText(reason)); setRefresh(value => value + 1); }
    finally { setBusy(false); }
  }
  async function relink() {
    if (!file || relinking || busy) return;
    media.current?.querySelector('video')?.pause(); setPlayback(undefined); setRelinking(true); setActionError('');
    try {
      const result = await api.relinkLibraryFile(assetId, file.id);
      if (result) { toast.success('文件已重新关联'); setRefresh(value => value + 1); }
      else toast.info('未更改文件关联');
    } catch (reason) { if (String(reason) === 'CANCELED') toast.info('已取消关联'); else setActionError(errorText(reason)); }
    finally { setRelinking(false); setCanceling(false); }
  }
  const fileKey = file ? JSON.stringify([file.id, file.path, file.size, file.mtime_ns, file.missing, file.changed]) : '';
  const playing = playback?.key === fileKey;
  function startPlayer(clip?: LibraryClip) {
    const selected = clip ? detail?.files.find(value => value.id === clip.file_id) : file;
    if (!selected) return;
    setSelectedFile(selected.id);
    setPlayback({ key: JSON.stringify([selected.id, selected.path, selected.size, selected.mtime_ns, selected.missing, selected.changed]), clipId: clip?.id, attempt: Date.now() });
  }
  function ensurePlayer() { if (!playing || playback?.clipId) startPlayer(); }
  function citeClip(citation: LibraryCitation) { editors.clips.request(() => { ensurePlayer(); setClipDraft({ citation, sequence: Date.now() }); }); }
  async function jumpToParagraph(positionMs: number) {
    if (!file || file.missing || file.changed) throw Error('来源文件不可用');
    if (playing && !playback?.clipId && player.current?.clock()) await player.current.seek(positionMs);
    else setPlayback({ key: fileKey, positionMs, attempt: Date.now() });
  }
  function selectParagraphs(citation: LibraryCitation, range: { start_ms: number; end_ms: number }) {
    editors.clips.request(() => { ensurePlayer(); setClipDraft({ citation, range, sequence: Date.now() }); });
  }

  return <section className="library-asset-detail" aria-label="视频资料详情" aria-busy={loading} hidden={!active}>
    <header className="library-detail-header"><Button size="sm" variant="ghost" disabled={relinking} onClick={back}><ArrowLeft />{backLabel}</Button>{onCreateIdea && <Button size="sm" variant="outline" disabled={!detail || loading || relinking} onClick={() => onCreateIdea({ kind: 'video', asset_id: assetId, resource_id: assetId })}>写选题</Button>}<Button size="sm" variant="ghost" aria-label="刷新视频资料" disabled={loading} onClick={() => setRefresh(value => value + 1)}><RotateCcw /></Button></header>
    <h2 ref={heading} tabIndex={-1} className="library-detail-title">{detail?.asset.title || (loading ? '正在读取视频资料…' : '视频资料')}</h2>
    <LibraryError message={actionError || error} />
    {visitError && <div><LibraryError message={visitError} /><Button variant="outline" size="sm" onClick={() => void recordVisit()}>重试保存访问记录</Button></div>}
    {loading && <div role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><Spinner />正在读取视频资料…</div>}
    {error && <Button size="sm" className="self-start" variant="outline" onClick={() => setRefresh(value => value + 1)}>重新读取资料</Button>}
    {detail && <div className="library-detail-columns">
      <div ref={media} className="library-detail-media">
        {detail.files.length > 0 && <label className="library-detail-file-picker">文件版本<Select value={selectedFile} disabled={relinking || busy} onValueChange={id => navigation.request(() => { setSelectedFile(id); setPlayback(undefined); setActionError(''); })}><SelectTrigger aria-label="文件版本"><SelectValue /></SelectTrigger><SelectContent>{detail.files.map((entry, index) => <SelectItem key={entry.id} value={entry.id}>版本 {index + 1}：{filename(entry)}{entry.missing ? '（缺失）' : entry.changed ? '（已变化）' : ''}</SelectItem>)}</SelectContent></Select></label>}
        {file && !file.missing && playing && active ? <LibraryVideo ref={player} key={fileKey + playback?.attempt} assetId={assetId} fileId={file.id} clipId={playback?.clipId} positionMs={playback?.positionMs} onMissing={refreshFileState} onReady={onPlayerReady} /> : <div className="library-detail-poster"><LibraryCover asset={detail.asset} className="library-detail-cover" />{(!file || file.missing) && <p>{file ? '本地文件缺失' : '暂无关联文件'}</p>}</div>}
        {file && <>
          <div className="library-file-actions"><Button size="sm" disabled={file.missing || busy || relinking} onClick={() => startPlayer()}><Play />{playing ? '重新播放' : '播放视频'}</Button><Button size="sm" variant="outline" disabled={file.missing || busy || relinking} onClick={() => void external('open')}><ExternalLink />系统播放器</Button><Button size="sm" variant="outline" disabled={file.missing || busy || relinking} onClick={() => void external('reveal')}><FolderOpen />定位文件</Button>{busy && <Spinner aria-label="正在打开" />}</div>
          <div className="library-file-heading"><FileStatus file={file} /><Button size="sm" variant="ghost" disabled={relinking || busy} onClick={() => navigation.request(() => { void relink(); })}>{relinking && <Spinner />}{relinking ? '关联中…' : '重新关联文件'}</Button>{relinking && <Button size="sm" variant="outline" disabled={canceling} onClick={() => { setCanceling(true); void api.cancelLibraryImport().catch(reason => { setActionError(errorText(reason)); setCanceling(false); }); }}>{canceling ? '取消中…' : '取消关联'}</Button>}</div>
          {file.changed && <p className="text-xs text-muted-foreground">文件已变化，以下规格为登记时信息。</p>}
          <dl className="library-detail-fields"><dt>文件路径</dt><dd>{displayPath(file.path)}</dd><dt>时长</dt><dd>{duration(file.duration_ms)}</dd><dt>分辨率</dt><dd>{file.width && file.height ? `${file.width} × ${file.height}` : '未知'}</dd><dt>文件大小</dt><dd>{size(file.size)}</dd></dl>
        </>}
        <LibraryClips onCreateIdea={onCreateIdea} assetId={assetId} file={file} active={active} revision={revision + refresh} player={player} ready={playerReady} draft={clipDraft} setDraft={setClipDraft} ensurePlayer={ensurePlayer} play={startPlayer} navigation={editors.clips} framesBusy={frames.busy} extractFrames={(clip, count) => { void frames.start({ kind: 'clip', clip_id: clip.id, count }); }} />
        {active && <LibraryKeyframes assetId={assetId} files={detail.files} file={file} player={player} state={frames} />}
      </div>
      <div className="library-detail-information">
        <LibraryWriting key={entryId} detail={detail} file={file} active={active} navigation={editors.writing} onClipCitation={citeClip} player={player} ready={playerReady} onJump={jumpToParagraph} onSelectParagraphs={selectParagraphs} onPlayClip={clip => navigation.request(() => startPlayer(clip))} />
        <section><h3>来源信息</h3><dl className="library-detail-fields"><dt>作者</dt><dd>{detail.asset.author || '作者未知'}</dd><dt>来源</dt><dd>{detail.asset.platform ? platformNames[detail.asset.platform] ?? '平台未知' : '本地导入'}</dd>{detail.asset.video_id && <><dt>作品 ID</dt><dd>{detail.asset.video_id}</dd></>}<dt>收录时间</dt><dd>{new Date(detail.asset.created_at * 1000).toLocaleString('zh-CN')}</dd></dl>
          {detail.asset.canonical_url && <div className="library-detail-source"><p>{detail.asset.canonical_url}</p><Button size="icon-sm" variant="ghost" aria-label="复制来源链接" onClick={() => { void navigator.clipboard.writeText(detail.asset.canonical_url!).then(() => toast.success('来源链接已复制')).catch(reason => setActionError(errorText(reason))); }}><Copy /></Button></div>}
          {detail.asset.archived_at != null && <Badge variant="outline">已移出知识库</Badge>}
        </section>
        <section><div className="library-section-heading"><h3>分类与标签</h3><Button size="sm" variant="ghost" onClick={event => organize(event.currentTarget)}>修改</Button></div><div className="library-detail-labels">{detail.category_ids.map(id => <Badge key={id} variant="secondary">{categories.find(category => category.id === id)?.name ?? '分类未加载'}</Badge>)}{detail.tag_ids.map(id => <Badge key={id} variant="outline">{tags.find(tag => tag.id === id)?.name ?? '标签未加载'}</Badge>)}{!detail.category_ids.length && !detail.tag_ids.length && <p className="text-muted-foreground">尚未分类或添加标签</p>}</div></section>
        <section><h3>资料摘要</h3><dl className="library-detail-fields"><dt>文案</dt><dd>{detail.documents.length} 个版本</dd><dt>笔记</dt><dd>{detail.notes.length} 篇</dd></dl>{detail.documents.length > 0 && <ul className="library-document-summaries" aria-label="文案版本摘要">{detail.documents.map(document => <li key={document.id}><span>{document.kind === 'recognized' ? '识别原文' : '整理稿'} v{document.version}</span><span>{document.characters} 字</span><span>{document.model_name || (document.kind === 'recognized' ? '模型未知' : '人工整理')}</span></li>)}</ul>}</section>
      </div>
    </div>}
  </section>;
}
