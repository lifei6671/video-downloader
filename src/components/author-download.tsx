import { useEffect, useState } from 'react';
import { api, errorText, type AuthorBatch, type AuthorDownloadMode, type AuthorLink, type AuthorScan } from '@/api';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Spinner } from '@/components/ui/spinner';

export function AuthorDownload({ author, updateScan, onDismiss, onSubmitted, onSubmitError }: {
  author: AuthorLink;
  updateScan: (scanId: string, scan: AuthorScan) => void;
  onDismiss: (scanId: string) => void;
  onSubmitted: (scanId: string, batch: AuthorBatch) => void;
  onSubmitError: (reason: unknown) => void;
}) {
  const [action, setAction] = useState<'reset' | 'continue' | 'cancel' | 'discard' | AuthorDownloadMode | null>(null);
  const [error, setError] = useState('');
  const [pollError, setPollError] = useState('');
  const { scan } = author;
  const scanning = scan?.status === 'scanning';
  const scanId = scan?.id;
  const showRecovery = scan?.status === 'partial' && !scan.batch;
  const hasWorks = !!scan && scan.videos + scan.albums > 0;
  const showDownload = !!scan && !scanning && !scan.batch && (scan.status === 'completed' || hasWorks);
  const showIncremental = showDownload && scan.pending_downloads > 0;

  useEffect(() => {
    if (!scanning || action || !scanId) return;
    const id = scanId;
    let active = true;
    let timer: number | undefined;
    async function refresh() {
      try {
        const next = await api.authorScan(id);
        if (active) {
          if (next) updateScan(id, next);
          setPollError('');
          if (!next || next.status === 'scanning') {
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
  }, [scanning, scanId, action, updateScan]);

  async function run(operation: NonNullable<typeof action>, request: () => Promise<void>) {
    setAction(operation);
    setError('');
    try { await request(); }
    catch (reason) {
      setError(errorText(reason));
      if (operation === 'full' || operation === 'incremental' || operation === 'discard') onSubmitError(reason);
    }
    finally { setAction(null); }
  }

  function recover(result: AuthorScan, reset: boolean) {
    void run(reset ? 'reset' : 'continue', async () => {
      updateScan(result.id, await api.recoverAuthorScan(result.id, reset));
    });
  }

  function confirm(result: AuthorScan, mode: AuthorDownloadMode) {
    void run(mode, async () => {
      const batch = await api.confirmAuthorScan(result.id, result.status === 'partial', mode);
      onSubmitted(result.id, batch);
    });
  }

  return (
    <Card size="sm">
      <CardHeader><CardTitle>{scan?.author_name || '抖音用户主页'}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        {(error || pollError) && <p role="alert" className="text-xs text-destructive">{error || pollError}</p>}
        {scan && <>
          <p role="status" className="flex flex-wrap items-center gap-x-2 text-sm">
            {scanning && <Spinner aria-hidden="true" className="size-4" />}
            <span>{scanning ? '正在扫描' : scan.status === 'completed' ? '已扫描到末页' : '扫描中断'}
            {scan.pages > 0 && `，已读 ${scan.pages} 页，发现 ${scan.videos} 条视频、${scan.albums} 条图集`}
            {scan.skipped > 0 && `，跳过 ${scan.skipped} 条`}</span>
          </p>
          {scan.error && <p role="alert" className="text-xs text-destructive">
            {scan.error === 'CANCELED' ? '扫描已停止' : errorText(scan.error)}
          </p>}
          {scanning && <Button variant="outline" disabled={!!action}
            onClick={() => void run('cancel', async () => updateScan(scan.id, await api.cancelAuthorScan(scan.id)))}>
            {action === 'cancel' ? '正在停止…' : '停止扫描'}
          </Button>}
          {showDownload && <p className="text-sm">已下载 {scan.already_local} 条，待下载 {scan.pending_downloads} 条</p>}
          {(showRecovery || showDownload) && <div className="flex items-center gap-2">
            {showRecovery && <>
              <Button variant="secondary" data-author-action="reset" disabled={!!action} onClick={() => recover(scan, true)}>
                {action === 'reset' ? '正在重置…' : '重置扫描'}
              </Button>
              <Button disabled={!!action} onClick={() => recover(scan, false)}>
                {action === 'continue' ? '正在恢复…' : '继续扫描'}
              </Button>
            </>}
            {showDownload && <>
              <Button variant="secondary" data-author-action="full" disabled={!!action || !hasWorks} onClick={() => confirm(scan, 'full')}>
                {action === 'full' ? '正在入队…' : '全量下载'}
              </Button>
              {showIncremental && <Button variant="secondary" data-author-action="incremental" disabled={!!action}
                onClick={() => confirm(scan, 'incremental')}>
                {action === 'incremental' ? '正在入队…' : '增量下载'}
              </Button>}
            </>}
            <Button variant="ghost" disabled={!!action} onClick={() => void run('discard', async () => {
              await api.discardAuthorScan(scan.id);
              onDismiss(scan.id);
            })}>{action === 'discard' ? '正在取消…' : '取消'}</Button>
          </div>}
          {showRecovery && (scan.resume_page
            ? <p className="text-xs text-muted-foreground">登录配置一致时，从第 {scan.resume_page} 页继续。</p>
            : scan.pages > 0 && <p className="text-xs text-muted-foreground">无法使用上次的翻页位置，继续将从第一页开始。</p>)}
          {showDownload && <p className="text-xs text-muted-foreground">
            {scan.status === 'partial' && '本次只处理已扫描到的作品。'}
            全量下载会覆盖已下载的视频或图集。
            {showIncremental && '增量下载只下载尚未保存到本地的作品。'}
          </p>}
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
