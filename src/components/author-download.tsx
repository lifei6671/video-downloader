import { useEffect, useState, type FormEvent, type RefObject } from 'react';
import { ClipboardPaste, Link } from 'lucide-react';
import { api, errorText, type AuthorBaselineAction, type AuthorScan, type AuthorScanMode } from '@/api';
import { ShareLinkInput } from '@/components/share-link-input';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Field, FieldLabel } from '@/components/ui/field';
import { InputGroup } from '@/components/ui/input-group';
import { Spinner } from '@/components/ui/spinner';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

export function AuthorDownload({ input, onChange, inputRef, paste, pasting, connected, openTasks }: {
  input: string;
  onChange: (value: string) => void;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  paste: () => void;
  pasting: boolean;
  connected: boolean;
  openTasks: () => void;
}) {
  const [scan, setScan] = useState<AuthorScan | null>(null);
  const [mode, setMode] = useState<AuthorScanMode | null>(null);
  const [loading, setLoading] = useState(true);
  const [action, setAction] = useState<'scan' | 'cancel' | 'submit' | 'resume' | null>(null);
  const [error, setError] = useState('');
  const [pollError, setPollError] = useState('');
  const scanning = scan?.status === 'scanning';
  const watching = scanning || Boolean(scan?.remaining && !scan.pause_reason);
  const scanId = scan?.id;
  const incremental = scan?.mode === 'incremental';
  const selectedMode = mode ?? scan?.mode ?? 'full';

  useEffect(() => {
    if (!connected || (!loading && !watching) || action) return;
    let active = true;
    let timer: number | undefined;
    async function refresh() {
      try {
        const next = await api.authorScan(scanId);
        if (active) {
          setScan(next);
          setPollError('');
          setLoading(false);
          if (next?.status === 'scanning' || (next?.remaining && !next.pause_reason)) {
            timer = window.setTimeout(() => void refresh(), 1000);
          }
        }
      } catch (reason) {
        if (active) {
          setPollError(errorText(reason));
          timer = window.setTimeout(() => void refresh(), 1500);
        }
      }
    }
    timer = window.setTimeout(() => void refresh(), loading ? 0 : 1000);
    return () => { active = false; window.clearTimeout(timer); };
  }, [connected, loading, watching, scanId, action]);

  async function run(operation: NonNullable<typeof action>, request: () => Promise<AuthorScan | null>) {
    setAction(operation);
    setError('');
    try { setScan(await request()); }
    catch (reason) { setError(errorText(reason)); }
    finally { setAction(null); }
  }

  function start(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!connected || loading || action || scanning || pasting) return;
    if (!input.trim()) { inputRef.current?.focus(); return; }
    void run('scan', () => api.startAuthorScan(input, selectedMode));
  }

  function confirm(result: AuthorScan, baselineAction?: AuthorBaselineAction) {
    void run('submit', async () => {
      await api.confirmAuthorScan(result.id, result.status === 'partial', baselineAction);
      return api.authorScan(result.id);
    });
  }

  return <>
    <Card size="sm" className="share-card">
      <CardHeader><CardTitle>抖音用户主页</CardTitle></CardHeader>
      <CardContent>
        <form onSubmit={start} className="space-y-3">
          <ToggleGroup type="single" value={selectedMode} variant="outline" size="sm" aria-label="抖音主页下载模式"
            disabled={scanning || !!action}
            onValueChange={value => { if (value === 'full' || value === 'incremental') setMode(value); }}>
            <ToggleGroupItem value="full">全量归档</ToggleGroupItem>
            <ToggleGroupItem value="incremental">增量下载</ToggleGroupItem>
          </ToggleGroup>
          <Field orientation="horizontal">
            <FieldLabel className="sr-only" htmlFor="video-input">抖音主页分享链接</FieldLabel>
            <InputGroup className="min-h-8 flex-1">
              <ShareLinkInput value={input} onChange={onChange} inputRef={inputRef}
                placeholder="粘贴抖音主页链接或分享口令…" disabled={scanning || action === 'scan'} />
            </InputGroup>
            <div className="flex shrink-0 flex-col gap-2">
              <Button type="button" variant="outline" disabled={!!action || pasting || scanning} onClick={paste}>
                {pasting ? <Spinner data-icon="inline-start" /> : <ClipboardPaste data-icon="inline-start" />}
                {pasting ? '粘贴中…' : '粘贴链接'}
              </Button>
              <Button type="submit" disabled={!connected || loading || !!action || pasting || scanning || !input.trim()}>
                {scanning || action === 'scan' ? <Spinner data-icon="inline-start" /> : <Link data-icon="inline-start" />}
                {scanning || action === 'scan' ? '扫描中…' : '扫描作品'}
              </Button>
            </div>
          </Field>
        </form>
        {(error || pollError) && <p role="alert" className="mt-3 text-xs text-destructive">{error || pollError}</p>}
      </CardContent>
    </Card>
    {scan && <Card size="sm">
      <CardHeader><CardTitle>{scan.author_name || '作品扫描'}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p role="status" className="text-sm">
          {scanning ? '正在扫描' : scan.status === 'completed' ? '已扫描到末页' : '扫描未完成'}
          {scan.pages > 0 && `，已读 ${scan.pages} 页，发现 ${scan.videos} 条视频、${scan.albums} 条图集`}
          {scan.skipped > 0 && `，跳过 ${scan.skipped} 条`}
        </p>
        {scan.error && <p role="alert" className="text-xs text-destructive">
          {scan.error === 'CANCELED' ? '扫描已停止' : errorText(scan.error)}
        </p>}
        {!scanning && incremental && !scan.batch && scan.needs_baseline &&
          <p className="text-sm">{scan.status === 'completed' ? '首次增量扫描，请选择如何处理已找到的历史作品' : '扫描到末页后才能建立基线'}</p>}
        {!scanning && incremental && !scan.batch && !scan.needs_baseline && (scan.status === 'completed' || scan.pages > 0) &&
          <p className="text-sm">新增待下载 {scan.new_posts} 条，待补下载 {scan.retry_posts} 条，已有本地文件 {scan.already_local} 条</p>}
        {scan.batch && <p className="text-sm">{scan.batch.baseline_only
          ? '已建立基线'
          : `已入队 ${scan.batch.queued} 条，跳过 ${scan.batch.skipped} 条，已完成 ${scan.completed} 条，失败或取消 ${scan.failed} 条`}</p>}
        {scan.pause_reason && <p role="alert" className="text-xs text-destructive">{errorText(scan.pause_reason)}</p>}
        <div className="flex flex-wrap gap-2">
          {scanning && <Button variant="outline" disabled={!!action}
            onClick={() => void run('cancel', () => api.cancelAuthorScan(scan.id))}>
            {action === 'cancel' ? '正在停止…' : '停止扫描'}
          </Button>}
          {!scanning && !scan.batch && (scan.status === 'completed' || scan.videos + scan.albums > 0) &&
            (scan.needs_baseline ? <>
              <Button disabled={!!action} onClick={() => confirm(scan, 'archive_history')}>
                {action === 'submit' ? '正在保存…' : '归档当前历史作品'}
              </Button>
              {scan.status === 'completed' && <Button variant="outline" disabled={!!action}
                onClick={() => confirm(scan, 'baseline_only')}>仅建立基线</Button>}
            </> : <Button disabled={!!action} onClick={() => confirm(scan)}>
              {action === 'submit' ? '正在保存…'
                : incremental && scan.new_posts + scan.retry_posts === 0 ? '确认增量下载'
                : scan.status === 'partial' ? '下载已找到的作品' : '加入下载队列'}
            </Button>)}
          {scan.pause_reason && <Button variant="outline" disabled={!!action}
            onClick={() => void run('resume', () => api.resumeAuthorBatch(scan.id))}>
            {action === 'resume' ? '正在恢复…' : '恢复批次'}
          </Button>}
          {scan.batch && !scan.batch.baseline_only && <Button variant="outline" onClick={openTasks}>查看下载任务</Button>}
        </div>
      </CardContent>
    </Card>}
  </>;
}
