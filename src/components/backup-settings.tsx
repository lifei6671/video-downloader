import { useEffect, useRef, useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type BackupSettings, type BackupUpdate } from '@/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { Spinner } from '@/components/ui/spinner';
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Checkbox } from '@/components/ui/checkbox';

export function BackupSettingsForm({ saved }: { saved: (value: BackupSettings) => void }) {
  const formRef = useRef<HTMLFormElement>(null);
  const [form, setForm] = useState<BackupUpdate>({ endpoint: '', bucket: '', prefix: 'yingqu/', role: 'reader', enabled: true });
  const [key, setKey] = useState(''), [secret, setSecret] = useState('');
  const [showKey, setShowKey] = useState(false), [showSecret, setShowSecret] = useState(false);
  const [savedTarget, setSavedTarget] = useState('');
  const target = (value: BackupUpdate) => JSON.stringify([value.endpoint, value.bucket, value.prefix]);
  const [credentialsChanged, setCredentialsChanged] = useState(false);
  const [busy, setBusy] = useState<'save' | 'test' | null>(null), [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const value = await api.backupSettings();
        if (!live) return;
        if (value) {
          setForm(value); setSavedTarget(target(value));
          if (value.credentials_configured) {
            const credentials = await api.backupCredentials(value.space_id);
            if (!live) return;
            setKey(credentials.access_key_id); setSecret(credentials.secret_access_key);
          }
        }
      } catch (error) { if (live) toast.error(errorText(error)); }
      finally { if (live) setLoaded(true); }
    })();
    return () => { live = false; };
  }, []);
  async function run(kind: 'save' | 'test') {
    if (busy || !loaded || !formRef.current?.reportValidity()) return;
    const started = performance.now();
    setBusy(kind);
    try {
      if (kind === 'test') {
        await api.testBackupConnection({ ...form, credentials: { access_key_id: key, secret_access_key: secret } });
        toast.success('连接正常');
      } else {
        const value = await api.saveBackupSettings({ ...form, credentials: credentialsChanged || target(form) !== savedTarget ? { access_key_id: key, secret_access_key: secret } : undefined });
        setForm(value); setSavedTarget(target(value)); setCredentialsChanged(false); setShowKey(false); setShowSecret(false); saved(value);
        toast.success('备份设置已保存');
      }
    } catch (error) { toast.error(errorText(error)); }
    finally {
      // Keep quick saves visible long enough for the button's fade, without delaying the request.
      if (kind === 'save') await new Promise(resolve => setTimeout(resolve, Math.max(0, 300 - (performance.now() - started))));
      setBusy(null);
    }
  }
  const disabled = !!busy || !loaded;
  return <form ref={formRef} className="backup-settings-form max-w-xl space-y-5" aria-label="远端备份设置" aria-busy={!!busy} onSubmit={event => { event.preventDefault(); void run('save'); }}>
    <FieldGroup className="gap-4">
      <Field orientation="horizontal"><FieldLabel htmlFor="backup-enabled">启用远端备份</FieldLabel><Checkbox id="backup-enabled" checked={form.enabled} disabled={disabled} onCheckedChange={enabled => setForm({ ...form, enabled: enabled === true })} /></Field>
      <Field><FieldLabel id="backup-role-label">本机角色</FieldLabel><Select value={form.role} disabled={disabled} onValueChange={role => setForm({ ...form, role: role as BackupUpdate['role'] })}><SelectTrigger aria-labelledby="backup-role-label"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="primary">主设备（上传与删除）</SelectItem><SelectItem value="reader">恢复设备（只读）</SelectItem></SelectContent></Select></Field>
      {(['endpoint', 'bucket', 'prefix'] as const).map(name => <Field key={name}><FieldLabel htmlFor={`backup-${name}`}>{({ endpoint: 'R2 Endpoint', bucket: 'Bucket', prefix: 'Prefix' })[name]}</FieldLabel><Input id={`backup-${name}`} required disabled={disabled} autoComplete="off" spellCheck={false} value={form[name]} placeholder={name === 'endpoint' ? 'https://<account-id>.r2.cloudflarestorage.com' : undefined} onChange={e => setForm({ ...form, [name]: e.target.value })} /></Field>)}
      <Field><FieldLabel htmlFor="backup-key">Access Key ID</FieldLabel><InputGroup><InputGroupInput id="backup-key" type={showKey ? 'text' : 'password'} autoComplete="new-password" spellCheck={false} disabled={disabled} value={key} onChange={e => { setKey(e.target.value); setCredentialsChanged(true); }} /><InputGroupAddon align="inline-end"><InputGroupButton size="icon-xs" disabled={disabled} aria-label={showKey ? '隐藏 Access Key ID' : '显示 Access Key ID'} aria-pressed={showKey} onClick={() => setShowKey(!showKey)}>{showKey ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup></Field>
      <Field><FieldLabel htmlFor="backup-secret">Secret Access Key</FieldLabel><InputGroup><InputGroupInput id="backup-secret" type={showSecret ? 'text' : 'password'} autoComplete="new-password" spellCheck={false} disabled={disabled} value={secret} onChange={e => { setSecret(e.target.value); setCredentialsChanged(true); }} /><InputGroupAddon align="inline-end"><InputGroupButton size="icon-xs" disabled={disabled} aria-label={showSecret ? '隐藏 Secret Access Key' : '显示 Secret Access Key'} aria-pressed={showSecret} onClick={() => setShowSecret(!showSecret)}>{showSecret ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup></Field>
    </FieldGroup>
    <div className="flex gap-2"><Button type="submit" className="backup-save-button" disabled={disabled} aria-busy={busy === 'save'} aria-label={busy === 'save' ? '保存中…' : '保存设置'}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'save'}>保存设置</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'save'}><Spinner aria-hidden="true" />保存中…</span></Button><Button type="button" variant="outline" className="min-w-28" disabled={disabled || !key || !secret} aria-busy={busy === 'test'} onClick={() => void run('test')}>{busy === 'test' && <Spinner aria-hidden="true" />}{busy === 'test' ? '测试中…' : '测试连接'}</Button></div>
  </form>;
}
