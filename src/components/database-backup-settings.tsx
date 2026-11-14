import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Check } from 'lucide-react';
import { api, errorText, type BackupSettings, type DatabaseBackupSnapshot, type DatabaseBackupStatus } from '@/api';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Spinner } from '@/components/ui/spinner';
import { Progress } from '@/components/ui/progress';
import { bytes } from '@/lib/format-bytes';

type TelegramBinding = NonNullable<BackupSettings['telegram']>;
const phaseLabels = {
  preparing: '正在创建数据库快照', encrypting: '正在加密快照', uploading: '正在上传数据库备份',
  confirming: '正在确认远端备份', finishing: '正在整理旧快照', canceling: '正在取消备份',
};

export function DatabaseBackupSettings({ spaceId, remoteEnabled, disabled, telegram }: {
  spaceId: string;
  remoteEnabled: boolean;
  disabled: boolean;
  telegram?: TelegramBinding;
}) {
  const [status, setStatus] = useState<DatabaseBackupStatus>();
  const [busy, setBusy] = useState<'disable' | 'frequency' | 'start' | 'cancel' | null>(null);
  const [actionError, setActionError] = useState('');
  const [statusError, setStatusError] = useState('');
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [passwordOpen, setPasswordOpen] = useState(false);
  const restoreButton = useRef<HTMLButtonElement>(null);
  const passwordTrigger = useRef<HTMLElement | null>(null);
  const enabled = status?.enabled;
  const progress = status?.progress;
  const working = !!progress || !!(remoteEnabled && status?.queued);
  const percent = progress?.total_bytes ? Math.min(99, Math.floor(100 * progress.uploaded_bytes / progress.total_bytes)) : null;

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const next = await api.databaseBackupStatus(spaceId);
        if (disposed) return;
        setStatus(next);
        setStatusError('');
        if (next.progress || (remoteEnabled && next.enabled)) {
          timer = setTimeout(() => void refresh(), next.progress || next.queued ? 750 : 2000);
        }
      } catch (reason) {
        if (!disposed) {
          setStatusError(errorText(reason));
          timer = setTimeout(() => void refresh(), 2000);
        }
      }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [spaceId, remoteEnabled, enabled, busy]);

  async function action(kind: NonNullable<typeof busy>, work: () => Promise<void>) {
    setBusy(kind);
    setActionError('');
    try { await work(); }
    catch (reason) { setActionError(errorText(reason)); }
    finally { setBusy(null); }
  }

  const error = actionError || statusError || (status?.last_error && errorText(status.last_error));
  return <section className="backup-settings-actions mt-6 max-w-xl space-y-4" aria-label="数据库备份">
    <h3 className="text-sm font-medium">数据库备份</h3>
    <Field orientation="horizontal" className="max-w-fit gap-3">
      <Checkbox id="backup-database-enabled" className="size-5 border-[0.5px] border-ring" checked={enabled ?? false} disabled={disabled || !!busy || !status || working} aria-busy={busy === 'disable'}
        onCheckedChange={value => {
          if (value === true) {
            passwordTrigger.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
            setPasswordOpen(true);
          } else {
            void action('disable', async () => setStatus(await api.setDatabaseBackupEnabled(spaceId, false)));
          }
        }} />
      <FieldLabel htmlFor="backup-database-enabled">自动备份数据库</FieldLabel>
    </Field>
    {status && <Field orientation="horizontal" className="max-w-fit gap-3">
      <FieldLabel htmlFor="backup-database-frequency">备份频率</FieldLabel>
      <Select value={String(status.interval_hours)} disabled={!enabled || disabled || !!busy || working}
        onValueChange={value => void action('frequency', async () => setStatus(await api.setDatabaseBackupInterval(spaceId, Number(value))))}>
        <SelectTrigger id="backup-database-frequency" className="w-36" aria-busy={busy === 'frequency'}><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="1">每 1 小时</SelectItem>
          <SelectItem value="3">每 3 小时</SelectItem>
          <SelectItem value="6">每 6 小时</SelectItem>
          <SelectItem value="24">每 24 小时</SelectItem>
        </SelectContent>
      </Select>
    </Field>}
    {status && <div role="status" className="space-y-1 text-xs text-muted-foreground">
      <p>{status.last_success_at ? `上次成功备份 ${formatTime(status.last_success_at)}` : '本机暂无成功备份记录'}</p>
      {(working || status.last_speed_bps != null) && <p>上次平均上传速度 {status.last_speed_bps == null ? '暂无记录' : `${bytes(status.last_speed_bps)}/s`}</p>}
      {!working && enabled && <p>{!remoteEnabled ? '远端备份已暂停' : status.next_attempt_at ? `下次自动备份 ${formatTime(status.next_attempt_at)}` : '等待首次备份'}</p>}
      {!working && status.last_canceled && <p>上次备份已取消</p>}
    </div>}
    {working && <div className="space-y-2" role="status" aria-label="数据库备份进度">
      <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <Spinner aria-hidden="true" className="motion-reduce:animate-none" />
        <span>{progress ? phaseLabels[progress.phase] : '等待备份'}</span>
        {progress?.total_bytes != null && progress.phase !== 'finishing' && <span className="tabular-nums">{bytes(progress.uploaded_bytes)} / {bytes(progress.total_bytes)}</span>}
        {progress?.phase === 'uploading' && progress.speed_bps != null && <span className="tabular-nums">{bytes(progress.speed_bps)}/s</span>}
      </p>
      {percent != null && <Progress value={progress?.phase === 'finishing' ? 100 : percent} aria-label="快照上传进度" updateIntervalMs={250} />}
    </div>}
    {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    <div className="flex flex-wrap gap-2">
      <Button type="button" variant="outline" disabled={!enabled || disabled || !!busy || working || !remoteEnabled} aria-busy={busy === 'start'} onClick={() => void action('start', async () => {
        setStatus(await api.startDatabaseBackup(spaceId));
      })}>{busy === 'start' && <Spinner aria-hidden="true" />}立即备份</Button>
      {working && <Button type="button" variant="outline" disabled={!!busy || progress?.phase === 'canceling' || progress?.phase === 'finishing'} aria-busy={busy === 'cancel'} onClick={() => void action('cancel', async () => {
        setStatus(await api.cancelDatabaseBackup(spaceId));
      })}>{busy === 'cancel' && <Spinner aria-hidden="true" />}取消备份</Button>}
      {status?.generated_password && <BackupCopyButton label="复制备份密码" confirmation="备份密码已复制"
        disabled={disabled || !!busy} copy={() => api.copyDatabaseBackupPassword(spaceId)} />}
      <Button ref={restoreButton} type="button" variant="outline" disabled={disabled || !!busy || working} onClick={() => setRestoreOpen(true)}>恢复数据库</Button>
    </div>
    {status?.last_message_link && <div className="space-y-2">
      <p id="telegram-backup-link-purpose" className="text-xs text-muted-foreground">
        换机时，把此链接粘贴到「恢复数据库」并输入备份密码，应用会直接读取 Telegram 私有频道中的这份备份。
      </p>
      <BackupCopyButton label="复制 Telegram 数据库备份链接" confirmation="Telegram 数据库备份链接已复制"
        disabled={disabled || !!busy} describedBy="telegram-backup-link-purpose" copy={() => api.copyDatabaseBackupLink(spaceId)} />
    </div>}
    {passwordOpen && <SetRecoveryPasswordDialog hasBackup={status?.last_success_at != null} close={() => setPasswordOpen(false)}
      restoreFocus={() => (passwordTrigger.current?.isConnected ? passwordTrigger.current : restoreButton.current)?.focus()}
      save={async (password, generated) => {
        setStatus(await api.setDatabaseBackupEnabled(spaceId, true, password, generated));
        setActionError('');
        toast.success(generated ? '已开启自动备份，可在设置中复制备份密码' : '已开启数据库自动备份');
      }} />}
    {restoreOpen && <RestoreDatabaseDialog spaceId={spaceId} telegram={telegram} close={() => setRestoreOpen(false)}
      restoreFocus={() => restoreButton.current?.focus()} />}
  </section>;
}

function BackupCopyButton({ label, confirmation, disabled, describedBy, copy }: {
  label: string;
  confirmation: string;
  disabled: boolean;
  describedBy?: string;
  copy: () => Promise<void>;
}) {
  const [phase, setPhase] = useState<'idle' | 'copying' | 'copied'>('idle');
  const copying = useRef(false);

  useEffect(() => {
    if (phase !== 'copied') return;
    const timeout = setTimeout(() => setPhase('idle'), 1000);
    return () => clearTimeout(timeout);
  }, [phase]);

  async function copyToClipboard() {
    if (copying.current) return;
    copying.current = true;
    setPhase('copying');
    try {
      await copy();
      setPhase('copied');
    } catch (reason) {
      setPhase('idle');
      toast.error(errorText(reason));
    } finally {
      copying.current = false;
    }
  }

  return <>
    <Button type="button" variant="outline" className="backup-copy-button"
      disabled={disabled} aria-busy={phase === 'copying'} aria-label={label} aria-describedby={describedBy}
      data-copy-phase={phase} onClick={() => void copyToClipboard()}>
      <span className="backup-action-label" aria-hidden="true">{label}</span>
      <span className="backup-action-pending" aria-hidden="true"><Spinner />复制中</span>
      <span className="backup-action-success" aria-hidden="true"><Check />已复制</span>
    </Button>
    <span role="status" className="sr-only">{phase === 'copied' ? confirmation : ''}</span>
  </>;
}

function SetRecoveryPasswordDialog({ hasBackup, save, close, restoreFocus }: {
  hasBackup: boolean;
  save: (password: string, generated: boolean) => Promise<void>;
  close: () => void;
  restoreFocus: () => void;
}) {
  const [password, setPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState<'generate' | 'manual' | null>(null);
  const [error, setError] = useState('');
  const matches = password === confirmation;
  const valid = password.trim() && [...password].length >= 12 && [...password].length <= 128;

  async function submit(source: NonNullable<typeof busy>) {
    setBusy(source);
    setError('');
    try {
      const generated = source === 'generate';
      const value = generated ? btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(18)))).replaceAll('+', '-').replaceAll('/', '_') : password;
      await save(value, generated);
      close();
    }
    catch (reason) { setError(errorText(reason)); }
    finally { setBusy(null); }
  }

  return <Dialog open onOpenChange={open => { if (!open && !busy) close(); }}>
    <DialogContent className="backup-settings-dialog" showCloseButton={!busy}
      onEscapeKeyDown={event => { if (busy) event.preventDefault(); }}
      onInteractOutside={event => { if (busy) event.preventDefault(); }}
      onCloseAutoFocus={event => { event.preventDefault(); restoreFocus(); }}>
      <DialogHeader>
        <DialogTitle>设置数据库恢复密码</DialogTitle>
        <DialogDescription>请妥善保存恢复密码。<strong>密码丢失后，无法从备份恢复数据。</strong>{hasBackup && '如使用新密码，已有备份仍需要原密码。'}</DialogDescription>
      </DialogHeader>
      <form className="space-y-4" onSubmit={event => { event.preventDefault(); if (!busy && valid && matches) void submit('manual'); }}>
        <Field>
          <FieldLabel htmlFor="database-new-password">恢复密码</FieldLabel>
          <Input id="database-new-password" type="password" value={password} autoComplete="new-password" spellCheck={false}
            aria-describedby="database-password-length" disabled={!!busy} onChange={event => { setPassword(event.target.value); setError(''); }} />
          <FieldDescription id="database-password-length">12 至 128 个字符</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="database-confirm-password">确认恢复密码</FieldLabel>
          <Input id="database-confirm-password" type="password" value={confirmation} autoComplete="new-password" spellCheck={false}
            disabled={!!busy} onChange={event => { setConfirmation(event.target.value); setError(''); }} />
        </Field>
        {(error || (confirmation && !matches)) && <p role="alert" className="text-xs text-destructive">{error || '两次输入的密码不一致'}</p>}
        <DialogFooter>
          <Button type="button" variant="outline" disabled={!!busy} onClick={close}>取消</Button>
          <Button type="button" variant="outline" disabled={!!busy} aria-busy={busy === 'generate'} onClick={() => void submit('generate')}>{busy === 'generate' && <Spinner aria-hidden="true" />}自动生成</Button>
          <Button type="submit" disabled={!!busy || !valid || !matches} aria-busy={busy === 'manual'}>{busy === 'manual' && <Spinner aria-hidden="true" />}启用数据库备份</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>;
}

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString();
}

export function TelegramDatabaseRestore({ telegram, disabled }: { telegram: TelegramBinding | null; disabled: boolean }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  return <div className="backup-settings-actions">
    <Button ref={button} type="button" variant="outline" disabled={disabled || !telegram} onClick={() => setOpen(true)}>恢复数据库</Button>
    {open && telegram && <RestoreDatabaseDialog spaceId={null} telegram={telegram} close={() => setOpen(false)} restoreFocus={() => button.current?.focus()} />}
  </div>;
}

function RestoreDatabaseDialog({ spaceId, close, restoreFocus, telegram }: { spaceId: string | null; close: () => void; restoreFocus: () => void; telegram?: TelegramBinding }) {
  const [key, setKey] = useState('');
  const [messageLink, setMessageLink] = useState('');
  const [snapshots, setSnapshots] = useState<DatabaseBackupSnapshot[]>();
  const [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [staged, setStaged] = useState(false);
  const submitting = useRef(false);
  const locked = busy || staged;
  const canSubmit = key.trim() && (!telegram || messageLink.trim()) && (!snapshots?.length || selected);
  const snapshot = snapshots?.[0];
  const submitLabel = staged ? '正在退出…' : snapshot ? '恢复并退出' : telegram ? '验证快照' : '查找备份';
  const pendingLabel = snapshot ? '正在恢复…' : telegram ? '验证中…' : '查找中…';
  const notice = error || (snapshots?.length === 0 ? '未找到匹配的数据库备份，请检查所选空间和恢复密码。' : '');

  function clearResults() {
    setSnapshots(undefined); setSelected(''); setError('');
  }

  async function submit() {
    if (!canSubmit || staged || submitting.current) return;
    submitting.current = true;
    setBusy(true);
    try {
      if (!snapshots?.length) {
        const found = telegram ? [await api.inspectTelegramDatabaseBackup(telegram, messageLink, key)] : await api.listDatabaseBackups(spaceId!, key);
        setError('');
        setSnapshots(found);
        setSelected(found[0]?.id ?? '');
      } else if (selected) {
        await api.restoreDatabaseBackup(spaceId, telegram ? messageLink : selected, key, telegram);
        setKey('');
        setStaged(true);
      }
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  }

  return <Dialog open onOpenChange={open => { if (!open && !locked) close(); }}>
    <DialogContent className="backup-settings-dialog" onEscapeKeyDown={event => { if (locked) event.preventDefault(); }}
      onCloseAutoFocus={event => { event.preventDefault(); restoreFocus(); }}
      onInteractOutside={event => { if (locked) event.preventDefault(); }}>
      <DialogHeader>
        <DialogTitle>恢复数据库</DialogTitle>
        <DialogDescription>确认后映取会退出。重新打开时，所选快照会替换当前数据库，并保留恢复前的本机副本。</DialogDescription>
      </DialogHeader>
      <form className="space-y-4" onSubmit={event => { event.preventDefault(); void submit(); }}>
        {telegram && <Field>
          <FieldLabel htmlFor="database-snapshot-message">Telegram 数据库备份链接</FieldLabel>
          <Input id="database-snapshot-message" type="text" value={messageLink} autoComplete="off" spellCheck={false}
            placeholder="https://t.me/c/…" aria-describedby="database-message-help" maxLength={256} readOnly={locked}
            onChange={event => { setMessageLink(event.target.value); clearResults(); }} />
          <FieldDescription id="database-message-help">粘贴从备份设置复制的链接，也可以在 Telegram 私有频道里找到含 yingqu-database.age 文件的消息，复制该消息的链接。</FieldDescription>
        </Field>}
        <Field>
          <FieldLabel htmlFor="database-recovery-key">恢复密码</FieldLabel>
          <Input id="database-recovery-key" type="password" value={key} autoComplete="off" spellCheck={false}
            aria-describedby="database-password-help" readOnly={locked} onChange={event => { setKey(event.target.value); clearResults(); }} />
          <FieldDescription id="database-password-help">输入创建这份备份时使用的密码。</FieldDescription>
        </Field>
        {!telegram && snapshot && <Field>
          <FieldLabel htmlFor="database-recovery-snapshot">备份时间</FieldLabel>
          <Select value={selected} onValueChange={setSelected} disabled={locked}>
            <SelectTrigger id="database-recovery-snapshot" className="w-full"><SelectValue /></SelectTrigger>
            <SelectContent position="popper">{snapshots?.map(snapshot => <SelectItem key={snapshot.id} value={snapshot.id}>
              {formatTime(snapshot.created_at)} ({(snapshot.bytes / (1024 * 1024)).toFixed(1)} MiB)
            </SelectItem>)}</SelectContent>
          </Select>
        </Field>}
        <div className="min-h-[2lh] text-xs leading-relaxed">
          {notice && <p role="alert" className="text-destructive">{notice}</p>}
          {telegram && snapshot && <p role="status" className="text-muted-foreground">备份时间 {formatTime(snapshot.created_at)}</p>}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" aria-disabled={locked} onClick={() => { if (!locked) close(); }}>取消</Button>
          <Button type="submit" className="backup-restore-submit" disabled={!locked && !canSubmit}
            aria-label={submitLabel} aria-disabled={locked} aria-busy={busy}>
            <span className="backup-action-label" aria-hidden="true">{submitLabel}</span>
            <span className="backup-action-pending" aria-hidden="true"><Spinner />{pendingLabel}</span>
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>;
}
