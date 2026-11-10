import { useEffect, useState } from 'react';
import { Link } from 'lucide-react';
import { api, errorText, type AuthorBaselineAction, type AuthorBatch, type AuthorLink, type AuthorScan, type AuthorScanMode } from '@/api';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Spinner } from '@/components/ui/spinner';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

export function AuthorDownload({ author, updateScan, onSubmitted, onSubmitError }: {
  author: AuthorLink;
  updateScan: (authorId: string, scan: AuthorScan | null) => void;
  onSubmitted: (scanId: string, batch: AuthorBatch) => void;
  onSubmitError: (reason: unknown) => void;
}) {
  const [mode, setMode] = useState<AuthorScanMode | null>(null);
  const [action, setAction] = useState<'scan' | 'cancel' | 'submit' | null>(null);
  const [error, setError] = useState('');
  const [pollError, setPollError] = useState('');
  const { scan, author_id, canonical_url, default_mode } = author;
  const scanning = scan?.status === 'scanning';
  const scanId = scan?.id;
  const incremental = scan?.mode === 'incremental';
  const selectedMode = mode ?? (scan && (!scan.batch || scan.batch.baseline_only) ? scan.mode : default_mode);
  const scanMatchesMode = scan?.mode === selectedMode;

  useEffect(() => {
    if (!scanning || action || !scanId) return;
    let active = true;
    let timer: number | undefined;
    async function refresh() {
      try {
        const next = await api.authorScan(scanId);
        if (active) {
          updateScan(author_id, next);
          setPollError('');
          if (next?.status === 'scanning') {
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
    timer = window.setTimeout(() => void refresh(), 1000);
    return () => { active = false; window.clearTimeout(timer); };
  }, [scanning, scanId, action, author_id, updateScan]);

  async function run(operation: NonNullable<typeof action>, request: () => Promise<void>) {
    setAction(operation);
    setError('');
    try { await request(); }
    catch (reason) {
      setError(errorText(reason));
      if (operation === 'submit') onSubmitError(reason);
    }
    finally { setAction(null); }
  }

  function start() {
    if (action || scanning) return;
    void run('scan', async () => updateScan(author_id, await api.startAuthorScan(canonical_url, selectedMode)));
  }

  function confirm(result: AuthorScan, baselineAction?: AuthorBaselineAction) {
    void run('submit', async () => {
      const batch = await api.confirmAuthorScan(result.id, result.status === 'partial', baselineAction);
      if (batch.baseline_only) updateScan(author_id, { ...result, batch });
      else onSubmitted(result.id, batch);
    });
  }

  return (
    <Card size="sm">
      <CardHeader><CardTitle>{scan?.author_name || '抖音用户主页'}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <ToggleGroup type="single" value={selectedMode} variant="outline" size="sm" aria-label="抖音主页下载模式"
            disabled={scanning || !!action}
            onValueChange={value => { if (value === 'full' || value === 'incremental') setMode(value); }}>
            <ToggleGroupItem value="full">全量归档</ToggleGroupItem>
            <ToggleGroupItem value="incremental">增量下载</ToggleGroupItem>
          </ToggleGroup>
          <Button type="button" disabled={!!action || scanning} onClick={start}>
            {scanning || action === 'scan' ? <Spinner data-icon="inline-start" /> : <Link data-icon="inline-start" />}
            {scanning || action === 'scan' ? '扫描中…' : '扫描作品'}
          </Button>
        </div>
        {(error || pollError) && <p role="alert" className="text-xs text-destructive">{error || pollError}</p>}
        {scan && <>
          <p role="status" className="text-sm">
            {!scanMatchesMode && (incremental ? '增量下载，' : '全量归档，')}
            {scanning ? '正在扫描' : scan.status === 'completed' ? '已扫描到末页' : '扫描未完成'}
            {scan.pages > 0 && `，已读 ${scan.pages} 页，发现 ${scan.videos} 条视频、${scan.albums} 条图集`}
            {scan.skipped > 0 && `，跳过 ${scan.skipped} 条`}
          </p>
          {scan.error && <p role="alert" className="text-xs text-destructive">
            {scan.error === 'CANCELED' ? '扫描已停止' : errorText(scan.error)}
          </p>}
          {!scanning && scanMatchesMode && incremental && !scan.batch && scan.needs_baseline &&
            <p className="text-sm">{scan.status === 'completed' ? '首次增量扫描，请选择如何处理已找到的历史作品' : '扫描到末页后才能建立基线'}</p>}
          {!scanning && incremental && !scan.batch && !scan.needs_baseline && (scan.status === 'completed' || scan.pages > 0) &&
            <p className="text-sm">新增待下载 {scan.new_posts} 条，待补下载 {scan.retry_posts} 条，已有本地文件 {scan.already_local} 条</p>}
          {scan.batch?.baseline_only && <p className="text-sm">已建立基线</p>}
          <div className="flex flex-wrap gap-2">
            {scanning && <Button variant="outline" disabled={!!action}
              onClick={() => void run('cancel', async () => updateScan(author_id, await api.cancelAuthorScan(scan.id)))}>
              {action === 'cancel' ? '正在停止…' : '停止扫描'}
            </Button>}
            {!scanning && scanMatchesMode && !scan.batch && (scan.status === 'completed' || scan.videos + scan.albums > 0) &&
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
          </div>
        </>}
      </CardContent>
    </Card>
  );
}

export function PausedAuthorBatch() {
  const [scan, setScan] = useState<AuthorScan | null>(null);
  const [resuming, setResuming] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (resuming) return;
    let active = true;
    let timer: number | undefined;
    async function refresh() {
      try {
        const next = await api.authorScan();
        if (active) setScan(next?.pause_reason ? next : null);
      } catch {
        // Retry on the next poll and keep the last known batch state visible.
      } finally {
        if (active) timer = window.setTimeout(() => void refresh(), 1000);
      }
    }
    void refresh();
    return () => { active = false; window.clearTimeout(timer); };
  }, [resuming]);

  async function resume(id: string) {
    setResuming(true);
    setError('');
    try { setScan(await api.resumeAuthorBatch(id)); }
    catch (reason) { setError(errorText(reason)); }
    finally { setResuming(false); }
  }

  if (!scan?.batch || !scan.remaining || !scan.pause_reason) return null;
  return <Card size="sm" className="shrink-0">
    <CardContent className="flex flex-wrap items-center justify-between gap-2">
      <p role="alert" className="min-w-0 text-sm text-destructive">{error || errorText(scan.pause_reason)}</p>
      <Button size="sm" variant="outline" disabled={resuming} onClick={() => void resume(scan.id)}>
        {resuming ? '正在恢复…' : '恢复批次'}
      </Button>
    </CardContent>
  </Card>;
}
