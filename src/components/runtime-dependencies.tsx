import { useEffect, useRef, useState, type ReactNode } from 'react';
import { AlertCircle, Check, Download, FolderOpen, RefreshCw, Square } from 'lucide-react';
import { api, errorText, type RuntimeProgress, type RuntimeStatus, type RuntimeTool } from '@/api';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardAction, CardContent, CardFooter } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Progress } from '@/components/ui/progress';
import { Spinner } from '@/components/ui/spinner';

const descriptions = {
  ffmpeg: { title: 'FFmpeg', text: '合并 YouTube 分离的音频与视频，下载包含 FFprobe。', download: 'FFmpeg 6.1.1' },
  javascript: { title: 'JavaScript 运行时', text: '用于 YouTube 解析，Node.js 22+ 或 Deno 2.3+ 满足其一即可。', download: 'Deno 2.9.7' },
};
const phases = { downloading: '正在下载', verifying: '正在校验与解压', installing: '正在验证可执行版本' };
const megabytes = (value: number) => (value / 1024 / 1024).toFixed(1) + ' MB';

export function useRuntimeDependencies() {
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState<RuntimeTool['id'] | null>(null);
  const [progress, setProgress] = useState<RuntimeProgress | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [canceling, setCanceling] = useState(false);
  const initialized = useRef(false);
  const busy = checking || installing !== null;
  async function check() {
    setChecking(true); setError(''); setNotice('');
    try { setStatus(await api.checkRuntimeDependencies()); }
    catch (e) { setError(String(e)); }
    finally { setChecking(false); }
  }
  useEffect(() => {
    if (!initialized.current) { initialized.current = true; void check(); }
  }, []);
  async function install(tool: RuntimeTool['id']) {
    setInstalling(tool); setError(''); setNotice(''); setProgress(null);
    try {
      setStatus(await api.installRuntimeDependency(tool, setProgress));
      setNotice('依赖已安装并重新检测，新任务可直接使用。');
    } catch (e) { setError(String(e)); }
    finally { setInstalling(null); setCanceling(false); }
  }
  async function cancel() {
    setCanceling(true);
    try { await api.cancelRuntimeDependency(); setError('CANCELED'); }
    catch (e) { setError(String(e)); }
    finally { setCanceling(false); }
  }
  return { status, checking, installing, progress, error, notice, canceling, busy, check, install, cancel };
}

export function RuntimeDependencies({ runtime, children }: { runtime: ReturnType<typeof useRuntimeDependencies>; children?: ReactNode }) {
  const { status, checking, installing, progress, error, notice, canceling, busy, check, install, cancel } = runtime;
  return <div className="flex min-w-0 flex-col gap-4" aria-busy={busy}>
    <div className="flex flex-wrap items-center justify-between gap-3"><p className="text-xs text-muted-foreground">优先使用系统已有工具；仅在缺失或版本不满足时下载。</p><Button variant="outline" disabled={busy} onClick={() => void check()}>{checking ? <Spinner aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}{checking ? '正在检测…' : '重新检测'}</Button></div>
    {error && <Alert variant={error === 'CANCELED' ? 'default' : 'destructive'} role="alert"><AlertCircle aria-hidden="true" /><AlertDescription>{errorText(error)}</AlertDescription></Alert>}
    {notice && <Alert role="status"><Check aria-hidden="true" /><AlertDescription>{notice}</AlertDescription></Alert>}
    {error === 'DEPENDENCY_BUSY' && <Button variant="outline" disabled={canceling} onClick={() => void cancel()}><Square aria-hidden="true" />取消正在进行的依赖操作</Button>}
    {status?.tools.map(tool => {
      const info = descriptions[tool.id], working = installing === tool.id;
      const percent = working && progress ? Math.floor(progress.downloaded_bytes / progress.total_bytes * 100) : null;
      return <Card key={tool.id} size="sm"><CardHeader><CardTitle>{info.title}</CardTitle><CardDescription>{info.text}</CardDescription><CardAction><Badge variant={tool.available ? 'success' : 'warning'}>{tool.available ? '可用' : '缺失或版本不满足'}</Badge></CardAction></CardHeader>
        <CardContent className="flex min-w-0 flex-col gap-3">{tool.available ? <><p className="text-sm">{tool.provider === 'node' ? 'Node.js' : tool.provider === 'deno' ? 'Deno' : 'FFmpeg'} {tool.version}<span className="ml-3 text-xs text-muted-foreground">{tool.source === 'managed' ? '应用专属目录' : '系统已安装'}</span></p><p className="break-all text-xs text-muted-foreground">{tool.path}</p></> : <p className="text-xs text-muted-foreground">{tool.can_install ? `可下载 ${info.download}，不会修改系统环境变量。` : '当前架构暂无自动下载包；自行安装后可重新检测。'}</p>}
          {working && <div className="flex flex-col gap-2" role="status"><Progress aria-label={`${info.title} 下载进度`} value={percent} /><p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><Spinner aria-hidden="true" /><span>{progress ? phases[progress.phase] : '正在检测本机环境…'}</span>{progress && <span>{megabytes(progress.downloaded_bytes)} / {megabytes(progress.total_bytes)}</span>}</p></div>}
        </CardContent>{(!tool.available || working) && <CardFooter className="justify-end gap-2">{working ? <Button variant="outline" disabled={canceling} onClick={() => void cancel()}><Square aria-hidden="true" />{canceling ? '正在取消…' : '取消下载'}</Button> : <Button disabled={busy || !tool.can_install} onClick={() => void install(tool.id)}><Download aria-hidden="true" />下载缺失依赖</Button>}</CardFooter>}
      </Card>;
    })}
    {children}
    {!status && !checking && !error && <p className="text-xs text-muted-foreground">检测后显示依赖版本与实际可执行路径。</p>}
    {status && <p className="flex items-start gap-2 break-all text-xs text-muted-foreground"><FolderOpen className="size-4 shrink-0" aria-hidden="true" />下载位置：{status.directory}<span className="shrink-0">{status.platform}</span></p>}
    <Alert role="note"><AlertCircle aria-hidden="true" /><AlertDescription>Python、yt-dlp、DTK 已随软件分发，通过软件更新升级。运行依赖下载使用系统网络及系统代理，不使用平台 Cookie 或平台代理配置。</AlertDescription></Alert>
  </div>;
}
