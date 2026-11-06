import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { AlertCircle, Bug, Check, Code2, Copy, Download, ExternalLink, FolderOpen, Github, Images, Info, Link, PackageOpen, RefreshCw, Square } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card';
import { Separator } from '@/components/ui/separator';
import { Accordion, AccordionItem, AccordionTrigger, AccordionContent } from '@/components/ui/accordion';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Spinner } from '@/components/ui/spinner';
import { Progress } from '@/components/ui/progress';
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { api, errorText, type ProjectPage, type UpdateCheck, type UpdateDownload } from '@/api';
const ReleaseNotes = lazy(() => import('@/components/release-notes').then(module => ({ default: module.ReleaseNotes })));

const brandIcon = new URL('../../src-tauri/icons/128x128.png', import.meta.url).href;
const engineInfo = '引擎 0.7.5，yt-dlp 2026.08.19，DTK 5.1.3';
export const aboutVersionInfo = `映取 v${__APP_VERSION__}\n${engineInfo}\nWindows / macOS`;
const features = [
  { Icon: Link, title: '从分享链接开始', description: '粘贴抖音、TikTok 或 YouTube 链接，先看预览，再选择视频规格。' },
  { Icon: Images, title: '视频与图集，一起收藏', description: '保存视频与封面；抖音、TikTok 图集原图优先，保留图片原格式。' },
  { Icon: FolderOpen, title: '作品与记录，留在本机', description: '在下载任务中查看进度、搜索记录，直接打开本地文件夹。' },
];
const projectLinks: { page: ProjectPage; Icon: typeof Github; title: string; description: string }[] = [
  { page: 'source', Icon: Github, title: '项目源码', description: 'lifei6671 / video-downloader' },
  { page: 'issues', Icon: Bug, title: '问题反馈', description: '报告问题，或分享你的建议' },
  { page: 'license', Icon: Code2, title: '开源许可证', description: 'Apache License 2.0' },
];
const credits = [
  { page: 'credit-ytdlp', name: 'yt-dlp', usage: '视频解析与媒体下载', license: 'Unlicense' },
  { page: 'credit-dtk', name: 'Douyin_TikTok_Download_API', usage: 'DTK 原生签名与图集解析模块', license: 'Apache-2.0' },
  { page: 'credit-tauri', name: 'Tauri', usage: '桌面窗口与系统集成', license: 'MIT / Apache-2.0' },
  { page: 'credit-react', name: 'React', usage: '界面与交互', license: 'MIT' },
  { page: 'credit-shadcn', name: 'shadcn/ui', usage: '基础界面控件', license: 'MIT' },
  { page: 'credit-lucide', name: 'Lucide', usage: '界面图标', license: 'ISC' },
] as const;
const downloadPhases = { preparing: '正在获取安装包信息…', downloading: '正在下载安装包', verifying: '正在校验安装包…', canceling: '正在取消…', completed: '下载完成，已通过校验', canceled: '下载已取消', failed: '安装包下载失败' };
const megabytes = (value: number) => `${(value / 1024 / 1024).toFixed(1)} MB`;

export function About({ busy, openPage, copyVersion }: { busy: boolean; openPage: (page: ProjectPage, releaseTag?: string) => void; copyVersion: () => void }) {
  const [update, setUpdate] = useState<{ status: 'idle' | 'checking' } | { status: 'checked'; result: UpdateCheck } | { status: 'error'; message: string }>({ status: 'idle' });
  const [updateOpen, setUpdateOpen] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState('');
  const [download, setDownload] = useState<UpdateDownload | null>(null);
  const [downloadReady, setDownloadReady] = useState(false);
  const [downloadStatusError, setDownloadStatusError] = useState('');
  const [starting, setStarting] = useState(false);
  const downloading = !!download && ['preparing', 'downloading', 'verifying', 'canceling'].includes(download.phase);
  const checkButton = useRef<HTMLButtonElement>(null);
  function acceptDownload(incoming: UpdateDownload | null) {
    if (incoming) setDownload(previous => !previous || incoming.revision >= previous.revision ? incoming : previous);
  }
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void (async () => {
      const unlisten = await api.subscribeUpdateDownload(status => { if (!disposed) acceptDownload(status); });
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      const status = await api.updateDownload();
      if (!disposed) { acceptDownload(status); setDownloadReady(true); }
    })().catch(error => { if (!disposed) setDownloadStatusError(errorText(error)); });
    return () => { disposed = true; stop?.(); };
  }, []);
  const checking = update.status === 'checking';
  async function checkUpdates() {
    if (checking || opening || downloading || starting) return;
    setUpdateOpen(false);
    setOpenError('');
    toast.dismiss('update-check');
    setUpdate({ status: 'checking' });
    try {
      const result = await api.checkForUpdates();
      setUpdate({ status: 'checked', result });
      setUpdateOpen(result.has_update);
      if (!result.has_update) toast.info(result.current_version === result.latest_version ? '当前已是最新版本' : '当前没有可用更新', { id: 'update-check', description: `当前版本 v${result.current_version}，最新正式版本 v${result.latest_version}` });
    }
    catch (error) { setUpdate({ status: 'error', message: errorText(error) }); }
  }
  const release = downloading && download ? download.release : update.status === 'checked' ? update.result : download?.release ?? null;
  const currentDownload = download?.release.release_tag === release?.release_tag ? download : null;
  const percent = currentDownload?.total_bytes ? Math.floor(currentDownload.downloaded_bytes / currentDownload.total_bytes * 100) : null;
  const published = release?.published_at ? new Date(release.published_at) : null;
  async function openDownload() {
    if (!release || opening) return;
    setOpening(true);
    setOpenError('');
    try { await api.openProjectPage('releases', release.release_tag); }
    catch (error) { setOpenError(errorText(error)); }
    finally { setOpening(false); }
  }
  async function openReleaseNoteLink(url: string) {
    setOpenError('');
    try { await api.openReleaseNoteLink(url); }
    catch { setOpenError(errorText('OPEN_LINK_FAILED')); }
  }
  async function startDownload() {
    if (!release || starting || downloading || opening || !downloadReady) return;
    setStarting(true); setOpenError('');
    try { acceptDownload(await api.startUpdateDownload(release.release_tag)); }
    catch (error) { setOpenError(errorText(error)); }
    finally { setStarting(false); }
  }
  async function cancelDownload() {
    if (!currentDownload || currentDownload.phase === 'canceling') return;
    setOpenError('');
    try { acceptDownload(await api.cancelUpdateDownload(currentDownload.id)); }
    catch (error) { setOpenError(errorText(error)); }
  }
  async function openInstaller() {
    if (!currentDownload || currentDownload.phase !== 'completed' || opening) return;
    setOpening(true); setOpenError('');
    try { await api.openUpdateInstaller(currentDownload.id); }
    catch (error) { setOpenError(errorText(error)); }
    finally { setOpening(false); }
  }
  return <div className="mx-auto flex w-full max-w-[1040px] flex-col gap-4">
    <Card className="[--card-spacing:20px]">
      <CardHeader className="flex flex-row items-center gap-4">
        <img src={brandIcon} alt="映取图标" className="size-20 shrink-0 object-contain" />
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2"><h2 className="text-xl font-semibold tracking-tight">映取</h2><Badge variant="outline">v{__APP_VERSION__}</Badge></div>
          <p className="text-sm">把喜欢的作品，留在本地。</p>
          <CardDescription>面向 Windows 与 macOS 的轻量桌面下载器。</CardDescription>
        </div>
      </CardHeader>
      <CardFooter className="flex flex-wrap justify-between gap-3 border-t">
        <div className="flex flex-wrap gap-2">{['抖音', 'TikTok', 'YouTube', 'Telegram'].map(platform => <Badge key={platform} variant="secondary" title={platform === 'Telegram' ? 'Telegram 实验模式' : undefined}>{platform}</Badge>)}</div>
        <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy} onClick={copyVersion}><Copy data-icon="inline-start" />复制版本信息</Button>{download && <Button variant="outline" onClick={() => { setUpdate({ status: 'checked', result: download.release }); setUpdateOpen(true); }}><Download data-icon="inline-start" />{downloading ? '查看下载进度' : download.phase === 'completed' ? '查看安装包' : '查看更新下载'}</Button>}<Button ref={checkButton} aria-busy={checking} aria-disabled={opening || downloading || starting} disabled={busy || checking} onClick={() => void checkUpdates()}>{checking ? <Spinner aria-hidden="true" data-icon="inline-start" /> : <RefreshCw data-icon="inline-start" />}{checking ? '正在检查…' : '检查更新'}</Button></div>
      </CardFooter>
    </Card>

    <div aria-live="polite" className="empty:hidden">
      {update.status === 'error' && <Alert variant="destructive"><AlertCircle /><AlertDescription className="flex flex-col gap-2"><span>检查更新失败：{update.message}</span><Button variant="outline" size="sm" className="self-start" disabled={busy} onClick={() => openPage('releases')}>查看发行版本<ExternalLink data-icon="inline-end" /></Button></AlertDescription></Alert>}
    </div>

    {release?.has_update && <Dialog open={updateOpen} onOpenChange={setUpdateOpen}>
      <DialogContent showCloseButton={false} className="update-dialog flex max-h-[85dvh] flex-col sm:max-w-[640px]" onCloseAutoFocus={event => { event.preventDefault(); checkButton.current?.focus(); }}>
        <DialogHeader className="shrink-0">
          <DialogTitle className="text-xl font-semibold leading-snug">发现新版本 v{release.latest_version}</DialogTitle>
          <DialogDescription>当前版本 v{release.current_version}{published && !Number.isNaN(published.getTime()) ? `，发布于 ${published.toLocaleDateString('zh-CN')}` : ''}</DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto pr-3">
          <h3 className="shrink-0 text-sm font-medium">更新说明</h3>
          <Suspense fallback={<div role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner aria-hidden="true" />正在加载更新说明…</div>}><ReleaseNotes content={release.release_notes} openLink={url => void openReleaseNoteLink(url)} /></Suspense>
          <p className="shrink-0 break-all text-xs text-muted-foreground">{release.installer ? `已匹配当前系统：${currentDownload?.filename ?? release.installer.name}，${megabytes(currentDownload?.total_bytes ?? release.installer.size)}` : '此版本没有可验证的适用安装包，可前往发行页面手动下载。'}</p>
          {currentDownload && <div className="flex shrink-0 flex-col gap-2 rounded-md border p-3" aria-label="安装包下载状态">
            <p className="flex items-center gap-2 text-sm" role="status">{downloading ? <Spinner aria-hidden="true" /> : currentDownload.phase === 'completed' ? <Check aria-hidden="true" className="size-4" /> : null}{downloadPhases[currentDownload.phase]}</p>
            {['downloading', 'verifying', 'canceling', 'completed'].includes(currentDownload.phase) && <><Progress aria-label="安装包下载进度" value={percent} /><p className="text-xs tabular-nums text-muted-foreground">{megabytes(currentDownload.downloaded_bytes)} / {currentDownload.total_bytes == null ? '大小未知' : megabytes(currentDownload.total_bytes)}{percent == null ? '' : `，${percent}%`}</p></>}
            {currentDownload.phase === 'completed' && <p className="text-xs text-muted-foreground">点击“打开安装包”继续安装，软件不会自动退出。</p>}
            {currentDownload.error && <Alert variant="destructive" role="alert"><AlertCircle /><AlertDescription>{errorText(currentDownload.error)}</AlertDescription></Alert>}
          </div>}
          {(openError || downloadStatusError) && <Alert variant="destructive" className="shrink-0" role="alert"><AlertCircle /><AlertDescription>{openError || downloadStatusError}</AlertDescription></Alert>}
          <Button variant="link" size="sm" className="shrink-0 self-start px-0" disabled={busy || opening} onClick={() => void openDownload()}>前往发行页面<ExternalLink data-icon="inline-end" /></Button>
        </div>
        <DialogFooter className="shrink-0 flex-wrap">
          <DialogClose asChild><Button variant="outline">{downloading ? '后台下载' : '稍后再说'}</Button></DialogClose>
          {downloading ? <Button variant="outline" disabled={currentDownload?.phase === 'canceling' || starting} onClick={() => void cancelDownload()}><Square data-icon="inline-start" />{currentDownload?.phase === 'canceling' ? '正在取消…' : '取消下载'}</Button> : currentDownload?.phase === 'completed' ? <><Button variant="outline" disabled={opening || starting || !downloadReady} onClick={() => void startDownload()}>重新下载</Button><Button disabled={busy || opening || starting} onClick={() => void openInstaller()}>{opening ? <Spinner aria-hidden="true" data-icon="inline-start" /> : <PackageOpen data-icon="inline-start" />}{opening ? '正在打开…' : '打开安装包'}</Button></> : <Button disabled={busy || opening || starting || !downloadReady || !release.installer} onClick={() => void startDownload()}>{starting ? <Spinner aria-hidden="true" data-icon="inline-start" /> : <Download data-icon="inline-start" />}{starting ? '正在准备…' : currentDownload ? '重新下载' : '下载更新'}</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>}

    <div className="grid items-stretch gap-4 min-[1100px]:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
      <Card size="sm" className="[--card-spacing:16px]">
        <CardHeader><CardTitle>为本地收藏而做</CardTitle><CardDescription>从确认作品，到找到保存的文件。</CardDescription></CardHeader>
        <CardContent><ul className="flex flex-col gap-4">{features.map(({ Icon, title, description }) => <li key={title} className="flex gap-3"><Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-primary" /><div className="flex min-w-0 flex-col gap-1"><p className="text-sm font-medium">{title}</p><p className="text-xs leading-relaxed text-muted-foreground">{description}</p></div></li>)}</ul></CardContent>
      </Card>
      <Card size="sm" className="[--card-spacing:16px]">
        <CardHeader><CardTitle>项目与支持</CardTitle><CardDescription>在系统默认浏览器中打开。</CardDescription></CardHeader>
        <CardContent className="flex flex-col gap-1">{projectLinks.map(({ page, Icon, title, description }) => <Button key={page} variant="ghost" className="project-support-link w-full justify-start gap-3" disabled={busy} onClick={() => openPage(page)}><Icon data-icon="inline-start" /><span className="flex min-w-0 flex-1 flex-col gap-1 text-left"><span>{title}</span><span className="truncate text-xs font-normal text-muted-foreground">{description}</span></span><ExternalLink data-icon="inline-end" /></Button>)}</CardContent>
      </Card>
    </div>

    <Card size="sm" className="[--card-spacing:16px]">
      <CardHeader><CardTitle>开源致谢</CardTitle><CardDescription>感谢这些项目，让映取得以专注于日常收藏。</CardDescription></CardHeader>
      <CardContent className="flex flex-col gap-3">
        <dl className="grid gap-x-6 gap-y-3 min-[1100px]:grid-cols-2">{credits.map(credit => <div key={credit.name} className="flex min-w-0 items-start justify-between gap-2"><div className="min-w-0"><dt><Button variant="link" className="h-auto min-w-0 justify-start whitespace-normal p-0 text-left" aria-label={`${credit.name} GitHub 仓库`} disabled={busy} onClick={() => openPage(credit.page)}><span className="break-all">{credit.name}</span><ExternalLink aria-hidden="true" className="size-3" /></Button></dt><dd className="mt-1 text-xs text-muted-foreground">{credit.usage}</dd></div><Badge variant="outline" className="shrink-0">{credit.license}</Badge></div>)}</dl>
        <Separator />
        <Accordion type="single" collapsible><AccordionItem value="versions" className="border-0"><AccordionTrigger className="py-1 text-xs">组件版本与本机工具</AccordionTrigger><AccordionContent className="pb-0 pt-2"><div className="flex flex-col gap-2 text-xs text-muted-foreground"><p>内置解析组件：{engineInfo}</p><p>YouTube 解析需本机 Node.js 22+ 或 Deno 2.3+；高清音视频合并使用 FFmpeg，可在系统设置的运行依赖中检测并下载缺失工具。</p></div></AccordionContent></AccordionItem></Accordion>
      </CardContent>
      <CardFooter className="flex flex-wrap justify-between gap-2 border-t"><p className="text-xs text-muted-foreground">各组件遵循其各自的开源许可证。</p><Button variant="link" size="sm" disabled={busy} onClick={() => openPage('credits')}>查看完整来源与许可<ExternalLink data-icon="inline-end" /></Button></CardFooter>
    </Card>
    <p className="flex items-start justify-center gap-2 pb-2 text-xs leading-relaxed text-muted-foreground"><Info aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />下载文件、历史记录和设置保存在本机，无需注册映取账号。</p>
  </div>;
}
