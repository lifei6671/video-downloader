import { useEffect, useRef, useState, type ReactNode } from 'react';
import { api, errorText, type BackupSettings, type TelegramStorageChannel, type TelegramStorageStatus } from '@/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Field, FieldLabel } from '@/components/ui/field';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { TelegramLoginQr } from '@/components/telegram-login-qr';
import { Spinner } from '@/components/ui/spinner';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';

export function TelegramStorageSettings({ space, name, connected, saving, actions }: { space?: BackupSettings; name?: string; connected?: (value: BackupSettings) => Promise<void>; saving?: (value: boolean) => void; actions?: ReactNode }) {
  const [status, setStatus] = useState<TelegramStorageStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [password, setPassword] = useState(''), [loginOpen, setLoginOpen] = useState(false);
  const [channels, setChannels] = useState<TelegramStorageChannel[]>([]), [channel, setChannel] = useState(space?.telegram?.channel_id ?? '');
  const [channelsLoaded, setChannelsLoaded] = useState(false), [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const live = useRef(true), revision = useRef(0);
  const autoChannels = useRef('');
  const channelInput = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { setChannel(space?.telegram?.channel_id ?? ''); }, [space?.space_id]);
  useEffect(() => {
    live.current = true;
    void api.telegramStorageStatus().then(value => { if (live.current) { setStatus(value); setLoginOpen(['qr', 'password'].includes(value.phase)); } })
      .catch(error => { if (live.current) setError(errorText(error)); });
    return () => { live.current = false; revision.current++; };
  }, []);
  useEffect(() => { if (!busy) input.current?.focus(); }, [status?.phase, busy]);
  useEffect(() => {
    if (status?.phase === 'ready') { setLoginOpen(false); setPassword(''); }
    else if (status?.phase === 'qr' || status?.phase === 'password') setLoginOpen(true);
  }, [status?.phase]);
  useEffect(() => {
    if (status?.phase !== 'ready') { autoChannels.current = ''; return; }
    if (space || busy || !status.operation_id) return;
    const identity = status.operation_id;
    if (autoChannels.current === identity) return;
    autoChannels.current = identity;
    void loadChannels();
  }, [status?.phase, status?.operation_id, busy, space?.space_id, space?.telegram_channel_title]);
  useEffect(() => {
    if (busy === 'cancel' || (!busy && !['connecting', 'qr', 'password'].includes(status?.phase ?? ''))) return;
    const current = revision.current;
    let stopped = false;
    const timer = setInterval(() => {
      void api.telegramStorageStatus().then(value => {
        if (!stopped && live.current && revision.current === current) setStatus(value);
      }).catch(() => {}); // The foreground request owns visible failures.
    }, 250);
    return () => { stopped = true; clearInterval(timer); };
  }, [busy, status?.phase]);
  async function refreshStatus(current: number) {
    try {
      const value = await api.telegramStorageStatus();
      if (live.current && current === revision.current) {
        setStatus(value);
        if (value.phase !== 'ready') { setChannels([]); setChannel(''); setChannelsLoaded(false); }
      }
    } catch { /* Keep the foreground result or error. */ }
  }
  async function run(kind: string, action: () => Promise<TelegramStorageStatus>) {
    if (busy) return;
    const current = ++revision.current;
    setBusy(kind); setError(''); setNotice(''); setPassword('');
    try {
      const value = await action();
      if (live.current && current === revision.current) {
        setStatus(value);
        if (value.phase !== 'ready') { setChannels([]); setChannel(''); setChannelsLoaded(false); }
        if (kind === 'resume' && value.phase === 'signed_out') setNotice('没有可恢复的登录，请扫码登录。');
      }
    } catch (error) {
      if (live.current && current === revision.current) setError(errorText(error));
    } finally {
      if (live.current && current === revision.current) {
        await refreshStatus(current);
        if (live.current && current === revision.current) setBusy(null);
      }
    }
  }
  async function cancel() {
    if (!status?.operation_id || busy === 'cancel') return;
    const current = ++revision.current;
    setBusy('cancel'); setPassword(''); setError('');
    try {
      const value = await api.telegramStorageCancel(status.operation_id);
      if (live.current && current === revision.current) { setStatus(value); setChannels([]); setChannel(''); }
    } catch (error) { if (live.current && current === revision.current) setError(errorText(error)); }
    finally { if (live.current && current === revision.current) setBusy(null); }
  }
  async function loadChannels() {
    if (!status?.operation_id || busy) return;
    const current = ++revision.current;
    setBusy('channels'); setError('');
    try {
      const values = await api.telegramStorageChannels(status.operation_id);
      if (live.current && current === revision.current) {
        setChannels(values); setChannel(previous => values.some(value => value.id === (space?.telegram?.channel_id ?? previous)) ? space?.telegram?.channel_id ?? previous : ''); setChannelsLoaded(true);
        if (!space) requestAnimationFrame(() => channelInput.current?.focus());
      }
    } catch (error) { if (live.current && current === revision.current) setError(errorText(error)); }
    finally {
      if (live.current && current === revision.current) {
        await refreshStatus(current);
        if (live.current && current === revision.current) setBusy(null);
      }
    }
  }
  async function connectLibrary(create: boolean) {
    if (busy || !(space?.telegram || status?.account && channel)) return;
    const started = performance.now();
    const current = ++revision.current;
    setBusy(create ? 'create' : 'connect'); saving?.(true); setError(''); setNotice('');
    try {
      let account = status?.account;
      if (space && phase !== 'ready') {
        const resumed = await api.telegramStorageResume();
        if (live.current && current === revision.current) setStatus(resumed);
        account = resumed.account;
      }
      if (!account) throw 'TELEGRAM_AUTH_REQUIRED';
      if (space?.telegram && account.id !== space.telegram.account_id) throw 'TELEGRAM_ACCOUNT_MISMATCH';
      const value = await api.saveBackupSettings({ cleanup_deleted_records: false, deleted_record_retention_days: 30, provider_type: 'telegram', profile: 'telegram', endpoint: '', bucket: '', prefix: '', region: 'auto', path_style: true, enabled: true, telegram: space?.telegram ?? { account_id: account.id, channel_id: channel }, display_name: name?.trim() || space?.display_name || channels.find(value => value.id === channel)?.title || '', create_library: create }, false);
      if (live.current && current === revision.current) await connected?.(value);
    } catch (error) { if (live.current && current === revision.current) setError(errorText(error)); }
    finally { await new Promise(resolve => setTimeout(resolve, Math.max(0, 300 - (performance.now() - started)))); saving?.(false); if (live.current && current === revision.current) { await refreshStatus(current); if (live.current) setBusy(null); } }
  }
  const id = status?.operation_id;
  const phase = status?.phase;
  const channelTitle = space?.telegram_channel_title || channels.find(value => value.id === space?.telegram?.channel_id)?.title;
  function startQr() {
    setStatus(previous => previous && { ...previous, phase: 'connecting', operation_id: null, qr: null, qr_expires_at: null, last_error: null });
    setLoginOpen(true);
    void run('qr', () => api.telegramStorageLoginQr());
  }
  function closeLogin() {
    if (busy && !id) return;
    setPassword(''); setLoginOpen(false);
    if (id && (busy || phase === 'qr' || phase === 'connecting' || phase === 'password')) void cancel();
  }
  return <section className="max-w-xl space-y-4" aria-label="Telegram 存储账号" aria-busy={!!busy}>
    {!status && !error && <p role="status" className="flex items-center gap-2 text-sm"><Spinner />正在读取账号…</p>}
    {status && !status.configured && <p role="status" className="text-sm text-muted-foreground">此构建尚未配置 Telegram 应用身份。</p>}
    {(status?.configured || space) && <>
      {(space || phase === 'ready' && status?.account) && <Field><FieldLabel>登录账号</FieldLabel><p className="text-sm break-words">{status?.account && (!space?.telegram || status?.account.id === space.telegram.account_id) ? status?.account.name || status?.account.id : space?.telegram?.account_id}</p></Field>}
      {space && <Field><FieldLabel>私有频道</FieldLabel><p className="text-sm break-words">{channelTitle || `频道 ${space.telegram?.channel_id}`}</p></Field>}
      {!space && phase === 'ready' && status?.account ? <>
        <div className="flex flex-wrap gap-2">{(!channelsLoaded || error) && !busy && <Button type="button" variant="outline" onClick={() => void loadChannels()}>重新读取频道</Button>}
          <Button type="button" variant="outline" disabled={!!busy} onClick={() => id && void run('logout', () => api.telegramStorageLogout(id))}>退出存储账号</Button></div>
        {channelsLoaded && channels.length === 0 && <p role="status" className="text-xs text-muted-foreground">未找到可访问的私有频道。</p>}
      </> : status?.configured && phase !== 'ready' && <div className="flex flex-wrap gap-2">
        {phase === 'signed_out' ? <>
          <Button type="button" disabled={!!busy} onClick={startQr}>扫码登录</Button>
          <Button type="button" variant="outline" disabled={!!busy} onClick={() => void run('resume', () => api.telegramStorageResume())}>恢复登录</Button>
          {!space && id && <Button type="button" variant="outline" disabled={!!busy} onClick={() => void run('logout', () => api.telegramStorageLogout(id))}>退出存储账号</Button>}
        </> : ['qr', 'password'].includes(phase ?? '') && <Button type="button" variant="outline" onClick={() => setLoginOpen(true)}>继续登录</Button>}
      </div>}
      {!space && <Field><FieldLabel id="telegram-storage-channel-label">私有频道<span aria-hidden="true" className="backup-required-mark text-destructive" /></FieldLabel><Select value={channel} disabled={!!busy || phase !== 'ready' || !channels.length} onValueChange={setChannel}>
        <SelectTrigger ref={channelInput} aria-labelledby="telegram-storage-channel-label"><SelectValue placeholder={busy === 'channels' ? '正在读取频道…' : '选择私有频道'} /></SelectTrigger>
        <SelectContent>{channels.map(value => <SelectItem key={value.id} value={value.id}>{value.title}{value.can_post ? '' : '（只读）'}</SelectItem>)}</SelectContent>
      </Select></Field>}
      {connected && <div className="flex flex-wrap gap-2">
        <Button type="button" className="min-w-28" aria-busy={busy === 'connect'} disabled={!!busy || !status?.configured || !space && (phase !== 'ready' || !channel)} onClick={() => void connectLibrary(false)}>{busy === 'connect' && <Spinner aria-hidden="true" />}{space ? '重新连接' : '保存配置'}</Button>
        {!space && <Button type="button" variant="outline" className="min-w-28" aria-busy={busy === 'create'} disabled={!!busy || phase !== 'ready' || !channel || !channels.find(value => value.id === channel)?.can_post} onClick={() => void connectLibrary(true)}>{busy === 'create' && <Spinner aria-hidden="true" />}创建备份库</Button>}
        {actions}
      </div>}
      {!loginOpen && (busy || phase === 'connecting') && <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><Spinner />{busy === 'channels' ? '正在读取频道…' : busy === 'create' ? '正在创建备份库…' : busy === 'connect' ? '正在连接备份库…' : busy === 'logout' ? '正在退出账号…' : busy === 'cancel' ? '正在取消…' : '正在连接 Telegram…'}</p>}
      <Dialog open={loginOpen} onOpenChange={open => { if (open) setLoginOpen(true); else closeLogin(); }}>
        <DialogContent className="tdl-login-dialog max-h-[85dvh] overflow-y-auto" showCloseButton={false} onEscapeKeyDown={event => { if (busy && !id) event.preventDefault(); }}>
          <DialogHeader><DialogTitle>登录 Telegram 存储</DialogTitle><DialogDescription>在手机 Telegram 的“设置 → 设备 → 连接桌面设备”中扫描二维码。</DialogDescription></DialogHeader>
          {phase === 'qr' && status?.qr && <TelegramLoginQr qr={status?.qr} />}
          {(phase === 'connecting' || (busy && phase !== 'qr' && phase !== 'ready')) && <p role="status" className="flex items-center gap-2"><Spinner />正在连接 Telegram…</p>}
          {phase === 'password' && !busy && <form id="telegram-storage-password-form" onSubmit={event => { event.preventDefault(); if (id) void run('password', () => api.telegramStoragePassword(id, password)); }}>
            <Field><FieldLabel htmlFor="telegram-storage-password">两步验证密码</FieldLabel><Input ref={input} autoFocus id="telegram-storage-password" type="password" autoComplete="off" required disabled={!!busy} value={password} onChange={event => setPassword(event.target.value)} /></Field>
          </form>}
          {(error || status?.last_error) && <p role="alert" className="text-sm text-destructive">{error || errorText(status?.last_error)}</p>}
          <DialogFooter>
            {phase === 'password' && <Button type="submit" form="telegram-storage-password-form" disabled={!!busy || !password}>验证并登录</Button>}
            {phase === 'signed_out' && !busy && <Button type="button" onClick={startQr}>重新扫码</Button>}
            <Button type="button" variant="outline" disabled={!!busy && !id || busy === 'cancel'} onClick={closeLogin}>{busy === 'cancel' ? '正在取消…' : phase === 'ready' || phase === 'signed_out' ? '关闭' : '取消登录'}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>}
    {!loginOpen && (error || status?.last_error) && <p role="alert" className="text-sm text-destructive">{error || errorText(status?.last_error)}</p>}
    {notice && <p role="status" className="text-xs text-muted-foreground">{notice}</p>}
  </section>;
}
