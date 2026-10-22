import { useCallback, useEffect, useRef, useState } from 'react';
import { AlertCircle, FolderOpen, QrCode } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type Settings, type TdlStatus, type TdlLoginStatus } from '@/api';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogAction, AlertDialogCancel } from '@/components/ui/alert-dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardAction, CardContent, CardFooter } from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { InputGroup, InputGroupInput, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { Spinner } from '@/components/ui/spinner';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';

type TdlCardProps = { settings: Settings; saved: (settings: Settings) => void; busy: boolean; setBusy: (busy: boolean) => void };

export function useTdlDependency(savedPath: string | null | undefined, setBusy: (busy: boolean) => void) {
  const [status, setStatus] = useState<TdlStatus | null>(null);
  const [error, setError] = useState('');
  const [checking, setChecking] = useState(false);
  const request = useRef(0);
  const check = useCallback(async (path: string | null) => {
    const id = ++request.current;
    setChecking(true); setBusy(true); setError(''); setStatus(null);
    try { const result = await api.checkTdl(path); if (id === request.current) setStatus(result); }
    catch (e) { if (id === request.current) setError(errorText(e)); }
    finally { if (id === request.current) { setChecking(false); setBusy(false); } }
  }, [setBusy]);
  useEffect(() => { if (savedPath !== undefined) void check(savedPath); }, [savedPath, check]);
  return { status, error, checking, check, clear: () => { setStatus(null); setError(''); } };
}

export function TdlDependencyCard({ settings, saved, busy, setBusy, dependency }: TdlCardProps & { dependency: ReturnType<typeof useTdlDependency> }) {
  const [path, setPath] = useState(settings.tdl.path ?? '');
  const [error, setError] = useState('');
  const pending = useRef(false);
  const { status, checking } = dependency;
  useEffect(() => { setPath(settings.tdl.path ?? ''); }, [settings.tdl.path]);
  async function updatePath(chooseFile: boolean) {
    if (pending.current || busy || checking) return;
    pending.current = true;
    setBusy(true); setError('');
    try {
      const next = chooseFile ? await api.selectTdlExecutable() : path.trim() || null;
      if (chooseFile && next === null) return;
      const current = await api.saveTdlSettings({ enabled: settings.tdl.enabled, path: next });
      setPath(current.tdl.path ?? '');
      saved(current);
      toast.success('tdl 路径已保存。');
      // A changed path triggers the app's existing dependency effect. Explicitly
      // recheck unchanged selections too, without launching two version probes.
      if (current.tdl.path === settings.tdl.path) await dependency.check(current.tdl.path);
    }
    catch (e) { setError(errorText(e)); }
    finally { pending.current = false; setBusy(false); }
  }
  return <Card size="sm"><CardHeader><CardTitle>tdl</CardTitle><CardDescription>Telegram 原始视频下载依赖，需自行安装；通过 tdl version 检测，不限制版本。</CardDescription><CardAction><Badge variant={status ? 'success' : dependency.error ? 'warning' : 'muted'}>{checking ? '正在检测' : status ? '可用' : dependency.error ? '不可用' : '未检测'}</Badge></CardAction></CardHeader>
    <CardContent className="flex flex-col gap-4">
      <Field><FieldLabel htmlFor="tdl-path">tdl 可执行文件路径</FieldLabel><InputGroup><InputGroupInput id="tdl-path" value={path} disabled={busy || checking} onChange={e => { setPath(e.target.value); dependency.clear(); }} onKeyDown={e => { if (e.key === 'Enter' && !e.nativeEvent.isComposing) { e.preventDefault(); void updatePath(false); } }} placeholder="留空自动检测；也可填写完整绝对路径" /><InputGroupAddon align="inline-end"><Tooltip><TooltipTrigger asChild><InputGroupButton size="icon-xs" aria-label="选择 tdl 可执行文件" disabled={busy || checking} onClick={() => void updatePath(true)}><FolderOpen aria-hidden="true" /></InputGroupButton></TooltipTrigger><TooltipContent>选择 tdl 可执行文件</TooltipContent></Tooltip></InputGroupAddon></InputGroup><FieldDescription>点击右侧图标选择文件后自动保存并检测；手动输入或清空后按 Enter 保存并检测。自动查找 PATH、GOBIN、GOPATH/bin 和 Go 常见安装目录。账号登录及 HTTP 实验模式在“平台账号 → Telegram”中管理，检测成功不代表已登录。</FieldDescription></Field>
      {status && <p role="status" className="break-all text-sm">tdl {status.version}<br /><span className="text-xs text-muted-foreground">{status.path}</span></p>}
      {(error || dependency.error) && <Alert role="alert" variant="destructive"><AlertCircle /><AlertDescription>{error || dependency.error}</AlertDescription></Alert>}
    </CardContent>
  </Card>;
}

export function TdlAccountCard({ settings, saved, busy, setBusy }: TdlCardProps) {
  const [enabled, setEnabled] = useState(settings.tdl.enabled);
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [loggingIn, setLoggingIn] = useState(false);
  const [loginOpen, setLoginOpen] = useState(false);
  const [loginStatus, setLoginStatus] = useState<TdlLoginStatus | null>(null);
  const [password, setPassword] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [loginError, setLoginError] = useState('');
  const pendingLogin = useRef(false);
  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    const refresh = async () => {
      try {
        const currentLogin = await api.tdlLoginStatus();
        if (disposed) return;
        if (currentLogin) {
          if (!loggingIn) setLoginOpen(true);
          setBusy(true); setLoggingIn(true); setLoginStatus(currentLogin);
          timer = window.setTimeout(() => void refresh(), 250);
        }
        else {
          const current = await api.settings();
          if (!disposed) saved(current);
          if (loggingIn && !pendingLogin.current) {
            setBusy(false); setLoggingIn(false); setLoginOpen(false); setLoginStatus(null); setPassword(''); setSubmitting(false);
          } else if (loggingIn) timer = window.setTimeout(() => void refresh(), 250);
        }
      } catch (e) { if (!disposed) setError(errorText(e)); }
    };
    void refresh();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [saved, setBusy, loggingIn]);
  const changed = enabled !== settings.tdl.enabled;
  async function save() {
    setBusy(true); setError(''); setNotice('');
    try { saved(await api.saveTdlSettings({ enabled, path: settings.tdl.path })); setNotice('Telegram 配置已保存。'); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  async function login() {
    pendingLogin.current = true;
    setBusy(true); setLoggingIn(true); setLoginOpen(true); setLoginStatus(null); setLoginError(''); setPassword(''); setSubmitting(false); setError(''); setNotice('');
    try {
      const current = await api.loginTdl(settings.tdl.path);
      saved(current);
      setLoginOpen(false);
      setNotice('tdl 登录成功，解析和下载将使用映取独立会话。');
    } catch (e) { setError(errorText(e)); setLoginError(errorText(e)); }
    finally { pendingLogin.current = false; setBusy(false); setLoggingIn(false); setPassword(''); setSubmitting(false); setLoginStatus(null); }
  }
  async function cancelLogin() {
    try { await api.cancelTdlLogin(); }
    catch (e) { setError(errorText(e)); }
  }
  async function submitPassword(event: React.FormEvent) {
    event.preventDefault();
    if (!loginStatus || submitting) return;
    const value = password;
    setPassword(''); setSubmitting(true); setLoginError('');
    try { await api.submitTdlPassword(loginStatus.id, value); }
    catch (e) { setLoginError(errorText(e)); setSubmitting(false); }
  }
  function closeLogin() {
    setPassword('');
    if (loggingIn) void cancelLogin();
    setLoginOpen(false);
  }
  const qr = loginStatus?.qr;
  const disabled = busy || loggingIn;
  return <Card size="sm"><CardHeader><CardTitle className="flex items-center gap-2 font-semibold">Telegram<Badge variant="warning">实验模式</Badge></CardTitle><CardDescription>使用本机官方 tdl 扫码登录并完成两步验证。tdl 路径在“运行依赖”中配置。</CardDescription></CardHeader>
    <CardContent className="flex flex-col gap-4">
      <Alert role="note"><AlertCircle /><AlertDescription>tdl v0.20.4 的 HTTP 服务会监听所有网络接口，没有访问令牌，且可请求当前账号有权访问的其他消息。映取只请求本次的 127.0.0.1 端口和消息路径，但这不构成网络隔离。请仅在可信网络中启用，并使用系统防火墙限制访问；服务运行期间勿同时运行其他使用此 tdl 会话的命令。</AlertDescription></Alert>
      <p className="text-xs text-muted-foreground">{settings.tdl.session_directory ? '已配置映取独立会话；实际权限以解析和下载结果为准。' : '当前使用 tdl 默认会话；点击登录将创建映取独立会话，不覆盖现有登录。'}</p>
      <Field orientation="horizontal"><Checkbox id="tdl-enabled" checked={enabled} disabled={disabled} onCheckedChange={value => { if (value === true) setConfirm(true); else setEnabled(false); }} /><FieldLabel htmlFor="tdl-enabled">启用 Telegram HTTP 实验模式</FieldLabel></Field>
      {loggingIn && <p role="status" className="text-sm text-muted-foreground">正在登录，请在登录弹窗内扫码或完成两步验证。最长等待 5 分钟。</p>}
      {error && <Alert role="alert" variant="destructive"><AlertCircle /><AlertDescription>{error}</AlertDescription></Alert>}
      {notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
    </CardContent>
    <CardFooter className="flex-wrap justify-end gap-2">{loggingIn ? <Button variant="outline" onClick={() => void cancelLogin()}>取消登录</Button> : <Button variant="outline" disabled={busy || changed} onClick={() => void login()}><QrCode />{settings.tdl.session_directory ? '重新登录 tdl' : '登录 tdl'}</Button>}<Button disabled={disabled || !changed} onClick={() => void save()}>保存 Telegram 配置</Button>{changed && <p className="w-full text-right text-xs text-muted-foreground">登录前请先保存 Telegram 配置。</p>}</CardFooter>
    <AlertDialog open={confirm} onOpenChange={setConfirm}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>启用 Telegram 实验模式？</AlertDialogTitle><AlertDialogDescription>当前 tdl 服务会对所有网络接口监听，且缺少访问令牌和请求消息范围限制。其他可连接此端口的设备可能访问已登录账号有权限的媒体。随机端口、隐藏链接及访问 127.0.0.1 均不能保证隔离。请确认网络可信，并由系统防火墙限制外部访问。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel>暂不启用</AlertDialogCancel><AlertDialogAction onClick={() => setEnabled(true)}>我已了解风险，启用</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
    <Dialog open={loginOpen} onOpenChange={open => { if (!open) closeLogin(); }}><DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto" showCloseButton={false}><DialogHeader><DialogTitle>登录 Telegram</DialogTitle><DialogDescription>使用 Telegram 的“设置 → 设备 → 连接桌面设备”扫描二维码。本次创建独立会话，保留已有登录。</DialogDescription></DialogHeader>
      {loggingIn && (!loginStatus || loginStatus.phase === 'starting') && <p role="status" className="flex items-center gap-2"><Spinner />正在连接 Telegram…</p>}
      {loggingIn && qr && <div className="mx-auto w-full max-w-64 rounded-lg bg-white p-2"><svg role="img" aria-label="Telegram 登录二维码" className="block aspect-square w-full" viewBox={`0 0 ${qr.length} ${qr.length}`} shapeRendering="crispEdges"><rect width={qr.length} height={qr.length} fill="white" /><path fill="black" d={qr.flatMap((row, y) => row.flatMap((black, x) => black ? [`M${x} ${y}h1v1h-1z`] : [])).join('')} /></svg></div>}
      {loggingIn && loginStatus?.phase === 'password' && !submitting && <form id="tdl-password-form" onSubmit={event => void submitPassword(event)}><Field><FieldLabel htmlFor="tdl-2fa-password">两步验证密码</FieldLabel><Input autoFocus id="tdl-2fa-password" type="password" autoComplete="off" value={password} onChange={event => setPassword(event.target.value)} /><FieldDescription>输入你在 Telegram 设置的两步验证密码。密码仅临时传给本次 tdl 登录进程，不保存到配置或日志。</FieldDescription></Field></form>}
      {loggingIn && (submitting || loginStatus?.phase === 'verifying') && <p role="status" className="flex items-center gap-2"><Spinner />正在验证，请稍候…</p>}
      {!loggingIn && !loginError && <p role="status">tdl 登录成功，独立会话已保存。</p>}
      {loginError && <Alert role="alert" variant="destructive"><AlertCircle /><AlertDescription>{loginError}</AlertDescription></Alert>}
      <DialogFooter>{loggingIn && loginStatus?.phase === 'password' && <Button type="submit" form="tdl-password-form" disabled={submitting || !password}>验证并登录</Button>}{!loggingIn && loginError && <Button onClick={() => void login()}>重新扫码</Button>}<Button variant="outline" onClick={closeLogin}>{loggingIn ? '取消登录' : '关闭'}</Button></DialogFooter>
    </DialogContent></Dialog>
  </Card>;
}
