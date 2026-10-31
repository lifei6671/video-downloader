import { useEffect, useRef, useState } from 'react';
import { Eye, EyeOff, Plus, Power, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type BackupSettings, type BackupUpdate } from '@/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { Spinner } from '@/components/ui/spinner';
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Checkbox } from '@/components/ui/checkbox';
import { Tabs, TabsList, TabsTrigger, TabsContent, settingsTabsListClassName, settingsTabsTriggerClassName } from '@/components/ui/tabs';
import { Badge } from '@/components/ui/badge';
import { Tooltip, TooltipTrigger, TooltipContent } from '@/components/ui/tooltip';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from '@/components/ui/alert-dialog';

type Editor = { form: BackupUpdate; key: string; secret: string; credentialsChanged: boolean; savedTarget: string };
type RenameEditor = { id: string; value: string; saving: boolean; error: string };

export function BackupSettingsForm({ saved }: { saved: (value: BackupSettings | null) => void }) {
  const formRef = useRef<HTMLFormElement>(null);
  const [form, setForm] = useState<BackupUpdate>(draft());
  const [spaces, setSpaces] = useState<BackupSettings[]>([]), [selected, setSelected] = useState('new');
  const [current, setCurrent] = useState<string | null>(null);
  const drafts = useRef(new Map<string, Editor>());
  const [renaming, setRenaming] = useState<RenameEditor | null>(null);
  const renameRef = useRef<RenameEditor | null>(null);
  const renameInput = useRef<HTMLInputElement>(null);
  const renameFocused = useRef<string | null>(null);
  const tabRefs = useRef(new Map<string, HTMLButtonElement>());
  const selectedRef = useRef(selected); selectedRef.current = selected;
  const [enabled, setEnabled] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [actionError, setActionError] = useState('');
  const [errors, setErrors] = useState<Record<string, string>>({});
  const selectedSpace = spaces.find(s => s.space_id === selected);
  const [key, setKey] = useState(''), [secret, setSecret] = useState('');
  const [showKey, setShowKey] = useState(false), [showSecret, setShowSecret] = useState(false);
  const [savedTarget, setSavedTarget] = useState('');
  const target = (value: BackupUpdate) => JSON.stringify([value.provider_type, value.profile, value.endpoint, value.bucket, value.prefix, value.region, value.path_style]);
  const retention = (value: BackupUpdate) => JSON.stringify([value.cleanup_deleted_records ?? false, value.deleted_record_retention_days ?? 30]);
  const [credentialsChanged, setCredentialsChanged] = useState(false);
  const [busy, setBusy] = useState<'save' | 'test' | 'load' | 'switch' | 'toggle' | 'retry' | 'delete' | null>(null), [loaded, setLoaded] = useState(false);
  function validate() {
    const inputs = Array.from(formRef.current?.querySelectorAll<HTMLInputElement>('input') ?? []);
    const invalid = inputs.filter(input => !input.validity.valid);
    setErrors(Object.fromEntries(invalid.map(input => [input.id, input.validity.valueMissing ? '请填写此项' : input.id === 'backup-retention-days' ? '请输入1到3650之间的整数' : '请输入有效的地址'])));
    invalid[0]?.focus();
    return invalid.length === 0;
  }
  function validation(id: string) { return { 'aria-invalid': !!errors[id], 'aria-describedby': errors[id] ? `${id}-error` : undefined }; }
  function fieldError(id: string) { return errors[id] && <FieldError id={`${id}-error`}>{errors[id]}</FieldError>; }
  function renameState(value: RenameEditor | null) { renameRef.current = value; setRenaming(value); }
  useEffect(() => {
    if (renaming && !renaming.saving && !busy && selectedRef.current === renaming.id) {
      renameInput.current?.focus();
      if (renameFocused.current !== renaming.id) { renameInput.current?.select(); renameFocused.current = renaming.id; }
    }
  }, [renaming?.id, renaming?.saving, busy]);
  function startRename(space: BackupSettings) {
    if (!loaded || (busy && busy !== 'load') || renameRef.current?.saving || selected !== space.space_id || renameRef.current?.id === space.space_id) return;
    renameFocused.current = null;
    renameState({ id: space.space_id, value: spaceName(space) || spaceLabel(space), saving: false, error: '' });
  }
  function cancelRename() {
    const edit = renameRef.current;
    if (!edit || edit.saving) return;
    renameState(null);
    requestAnimationFrame(() => tabRefs.current.get(edit.id)?.focus());
  }
  async function commitRename(returnFocus = false) {
    const edit = renameRef.current;
    if (!edit || edit.saving || (busy && busy !== 'load')) return;
    const space = spaces.find(s => s.space_id === edit.id);
    if (!space) return;
    const name = edit.value.trim();
    if (name === spaceName(space) || (!spaceName(space) && name === spaceLabel(space))) {
      renameState(null);
      if (returnFocus) requestAnimationFrame(() => tabRefs.current.get(edit.id)?.focus());
      return;
    }
    renameState({ ...edit, saving: true, error: '' });
    try {
      const value = await api.renameBackupSpace(edit.id, name);
      setSpaces(all => all.map(s => s.space_id === edit.id ? { ...s, display_name: value.display_name } : s));
      const cached = drafts.current.get(edit.id);
      if (cached) drafts.current.set(edit.id, { ...cached, form: { ...cached.form, display_name: value.display_name } });
      if (selectedRef.current === edit.id) setForm(previous => ({ ...previous, display_name: value.display_name }));
      if (current === edit.id) saved(value);
      renameState(null); toast.success('空间名称已保存');
      if (returnFocus && selectedRef.current === edit.id) requestAnimationFrame(() => tabRefs.current.get(edit.id)?.focus());
    } catch (error) { renameState({ ...edit, saving: false, error: errorText(error) }); }
  }
  function editor(): Editor { return { form, key, secret, credentialsChanged, savedTarget }; }
  function remember() {
    if (!selectedSpace || credentialsChanged || target(form) !== savedTarget || form.enabled !== selectedSpace.enabled || retention(form) !== retention(selectedSpace) || spaceName(form) !== spaceName(selectedSpace)) drafts.current.set(selected, editor());
    else drafts.current.delete(selected);
  }
  function restore(value: Editor) {
    setForm(value.form); setKey(value.key); setSecret(value.secret); setCredentialsChanged(value.credentialsChanged); setSavedTarget(value.savedTarget);
    setShowKey(false); setShowSecret(false);
    setErrors({}); setActionError('');
  }
  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const [active, all, syncEnabled] = await Promise.all([api.backupSettings(), api.backupSpaces(), api.backupSyncEnabled()]);
        if (live) setSpaces(all);
        if (!live) return;
        setCurrent(active?.space_id ?? null);
        setEnabled(syncEnabled);
        const value = active ?? all[0];
        if (value) {
          setForm(update(value)); setSelected(value.space_id); setSavedTarget(target(value));
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
  const currentSpace = spaces.find(space => space.space_id === current);
  // Read local worker status while connecting or cleanup is enabled; keep form and credentials intact.
  useEffect(() => {
    if (!loaded || busy || !enabled || !(currentSpace?.connection_pending || currentSpace?.cleanup_deleted_records)) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const [active, all] = await Promise.all([api.backupSettings(), api.backupSpaces()]);
        if (!live) return;
        setSpaces(all); saved(active);
        if (active?.connection_pending || active?.cleanup_deleted_records) timer = setTimeout(() => void refresh(), active.connection_pending ? 1000 : 5000);
      } catch (error) {
        if (live) { setActionError(errorText(error)); timer = setTimeout(() => void refresh(), 5000); }
      }
    }
    timer = setTimeout(() => void refresh(), currentSpace.connection_pending ? 1000 : 5000);
    return () => { live = false; clearTimeout(timer); };
  }, [loaded, busy, enabled, current, currentSpace?.connection_pending, currentSpace?.cleanup_deleted_records, saved]);
  async function run(kind: 'save' | 'test') {
    if (busy || !loaded || !validate()) return;
    const started = performance.now();
    setBusy(kind);
    setActionError('');
    try {
      if (kind === 'test') {
        await api.testBackupConnection({ ...form, credentials: { access_key_id: key, secret_access_key: secret } });
        toast.success('连接正常');
      } else {
        const credentials = { access_key_id: key, secret_access_key: secret };
        const value = await api.saveBackupSettings({ ...form, credentials: credentialsChanged || target(form) !== savedTarget ? credentials : undefined }, false);
        const [all, active] = await Promise.all([api.backupSpaces(), api.backupSettings()]);
        drafts.current.delete(selected); drafts.current.delete(value.space_id);
        setSpaces(all); setCurrent(active?.space_id ?? null); setSelected(value.space_id);
        restore({ form: update(value), key, secret, savedTarget: target(value), credentialsChanged: false });
        if (active) saved(active);
        toast.success('备份设置已保存');
      }
    } catch (error) { setActionError(errorText(error)); toast.error(errorText(error)); }
    finally {
      // Keep quick saves visible long enough for the button's fade, without delaying the request.
      if (kind === 'save') await new Promise(resolve => setTimeout(resolve, Math.max(0, 300 - (performance.now() - started))));
      setBusy(null);
    }
  }
  function changeTarget(value: BackupUpdate) {
    if (target(value) === target(form)) return;
    if (selected !== 'new') remember();
    setSelected('new'); restore({ form: value, key: '', secret: '', credentialsChanged: true, savedTarget: '' });
  }
  async function choose(id: string) {
    if (!id || id === selected || busy || !loaded) return;
    const existing = spaces.find(s => s.space_id === id);
    if (id !== 'new' && !existing) return;
    remember();
    setSelected(id);
    const cached = drafts.current.get(id);
    if (cached) { restore(cached); return; }
    if (!existing) { restore({ form: draft(), key: '', secret: '', credentialsChanged: false, savedTarget: '' }); return; }
    restore({ form: update(existing), key: '', secret: '', credentialsChanged: false, savedTarget: target(existing) });
    setBusy('load');
    try {
      const credentials = existing.credentials_configured ? await api.backupCredentials(id) : null;
      setKey(credentials?.access_key_id ?? ''); setSecret(credentials?.secret_access_key ?? '');
    } catch (e) { toast.error(errorText(e)); } finally { setBusy(null); }
  }
  async function switchSpace() {
    if (busy || !loaded || !enabled || !selectedSpace || target(form) !== target(selectedSpace)) return;
    if (dirty) return;
    const started = performance.now();
    setBusy('switch');
    try {
      const value = await api.selectBackupSpace(selected);
      await visibleFeedback(started);
      setSpaces(await api.backupSpaces()); setCurrent(value.space_id); drafts.current.delete(selected);
      restore({ form: update(value), key, secret, credentialsChanged: false, savedTarget: target(value) });
      saved(value); toast.success('默认备份空间已切换');
      requestAnimationFrame(() => formRef.current?.querySelector<HTMLButtonElement>('button[type="submit"]')?.focus({ preventScroll: true }));
    } catch (e) { await visibleFeedback(started); setActionError(errorText(e)); toast.error(errorText(e)); } finally { setBusy(null); }
  }
  async function toggleEnabled(retry = false) {
    if (disabled) return;
    const started = performance.now();
    let confirmedEnabled: boolean | undefined;
    setBusy(retry ? 'retry' : 'toggle'); setActionError('');
    try {
      const value = await api.setBackupSyncEnabled(retry || !enabled);
      confirmedEnabled = value;
      await visibleFeedback(started);
      const [active, all] = await Promise.all([api.backupSettings(), api.backupSpaces()]);
      setEnabled(value); setSpaces(all); saved(active);
      if (!retry) toast.success(value ? '远端备份已启用' : '远端备份已停用');
    } catch (error) { await visibleFeedback(started); if (confirmedEnabled !== undefined) setEnabled(confirmedEnabled); setActionError(errorText(error)); toast.error(errorText(error)); }
    finally { setBusy(null); }
  }
  async function deleteSpace(confirmed = false) {
    if (disabled || !selectedSpace) return;
    const started = performance.now();
    setBusy('delete'); setActionError('');
    try {
      if (!confirmed) {
        const usage = await api.backupSpaceUsage(selected);
        if (usage.assets || usage.operations) { await visibleFeedback(started); setConfirmDelete(true); return; }
      }
      await api.deleteBackupSpace(selected, confirmed);
      await visibleFeedback(started);
      setConfirmDelete(false);
      const [active, all] = await Promise.all([api.backupSettings(), api.backupSpaces()]);
      drafts.current.delete(selected); setSpaces(all); setCurrent(active?.space_id ?? null); saved(active);
      const next = active ?? all[0];
      setSelected(next?.space_id ?? 'new');
      restore(next ? drafts.current.get(next.space_id) ?? { form: update(next), key: '', secret: '', credentialsChanged: false, savedTarget: target(next) } : { form: draft(), key: '', secret: '', credentialsChanged: false, savedTarget: '' });
      if (next?.credentials_configured && !drafts.current.has(next.space_id)) {
        const credentials = await api.backupCredentials(next.space_id);
        setKey(credentials.access_key_id); setSecret(credentials.secret_access_key);
      }
      setConfirmDelete(false); toast.success('备份配置已删除');
      requestAnimationFrame(() => next ? tabRefs.current.get(next.space_id)?.focus() : formRef.current?.querySelector<HTMLInputElement>('input')?.focus({ preventScroll: true }));
    } catch (error) {
      await visibleFeedback(started);
      if (String(error).includes('BACKUP_SPACE_REFERENCED')) setConfirmDelete(true);
      else { setActionError(errorText(error)); toast.error(errorText(error)); }
    } finally { setBusy(null); }
  }
  const saveLabel = selected === 'new' ? '新建配置' : '保存配置';
  const disabled = !!busy || !loaded || !!renaming?.saving;
  const tabsDisabled = !loaded || (!!busy && busy !== 'load') || !!renaming?.saving;
  const dirty = !selectedSpace || target(form) !== savedTarget || credentialsChanged || retention(form) !== retention(selectedSpace) || spaceName(form) !== spaceName(selectedSpace);
  const switchReason = !enabled ? '请先启用远端备份' : !selectedSpace || target(form) !== target(selectedSpace) ? '请先保存配置' : spaceName(form) !== spaceName(selectedSpace) ? '请先保存名称' : selected === current && selectedSpace.enabled ? '当前默认备份空间' : dirty ? '请先保存配置' : '';
  return <div className="backup-settings-actions flex min-h-0 min-w-0 flex-1 flex-col gap-4">
    <header className="flex shrink-0 flex-col gap-1"><div className="flex items-center justify-between gap-4"><h2 className="text-sm font-semibold">远端备份</h2><Button type="button" className="backup-save-button backup-feedback-button" variant={enabled ? 'secondary' : 'default'} disabled={disabled} aria-label={enabled ? '停用远端备份' : '启用远端备份'} aria-pressed={enabled} aria-busy={busy === 'toggle'} onClick={() => void toggleEnabled()}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'toggle'}><Power />{enabled ? '停用远端备份' : '启用远端备份'}</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'toggle'}><Spinner />{enabled ? '正在停用…' : '正在启用…'}</span></Button></div><p className="text-xs text-muted-foreground">备份视频与文案，视频按需下载。</p></header>
    <div className="backup-settings-scroll min-h-0 overflow-y-auto p-1 pr-3"><div className="flex min-w-0 flex-col gap-5">
    {actionError && <p role="alert" className="text-xs text-destructive">{actionError}</p>}
    {enabled && currentSpace?.connection_pending && <div className="flex items-center gap-2 text-xs text-muted-foreground">{currentSpace.last_error ? <><p role="alert" className="text-destructive">{errorText(currentSpace.last_error)}</p><Button type="button" variant="outline" size="sm" className="backup-save-button backup-feedback-button" disabled={disabled} aria-label="重新连接" aria-busy={busy === 'retry'} onClick={() => void toggleEnabled(true)}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'retry'}>重新连接</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'retry'}><Spinner />正在重试…</span></Button></> : <p role="status" className="flex items-center gap-2"><Spinner aria-hidden="true" />正在连接备份存储…</p>}</div>}
    <Tabs value={selected} onValueChange={id => void choose(id)} className="min-w-0 gap-5">
    <TabsList variant="line" aria-label="备份空间" className={`${settingsTabsListClassName} max-w-full overflow-x-auto overflow-y-hidden`}>
      {spaces.map(space => {
        const label = spaceLabel(space);
        const editing = renaming?.id === space.space_id;
        return <Tooltip key={space.space_id} open={editing ? !!renaming.error : undefined}><TooltipTrigger asChild><span className="relative inline-flex h-full shrink-0" onDoubleClick={() => startRename(space)}>
          <TabsTrigger ref={node => { if (node) tabRefs.current.set(space.space_id, node); else tabRefs.current.delete(space.space_id); }} value={space.space_id} disabled={tabsDisabled || editing} aria-hidden={editing || undefined} aria-label={label} aria-keyshortcuts="F2" onKeyDown={event => { if (event.key === 'F2') { event.preventDefault(); event.stopPropagation(); startRename(space); } }} className={`${settingsTabsTriggerClassName} max-w-64 gap-2${editing ? ' invisible' : ''}`}><span className="truncate">{label}</span>{space.space_id === current && <Badge variant="muted" className="shrink-0">默认</Badge>}</TabsTrigger>
          {editing && <><Input ref={renameInput} className="backup-space-name-editor" aria-label="编辑空间名称" aria-busy={renaming.saving} aria-invalid={!!renaming.error} aria-describedby={renaming.error ? 'backup-space-rename-error' : undefined} maxLength={64} value={renaming.value} disabled={(!!busy && busy !== 'load') || renaming.saving} onChange={event => renameState({ ...renaming, value: event.target.value, error: '' })} onBlur={() => void commitRename()} onDoubleClick={event => event.stopPropagation()} onKeyDown={event => { event.stopPropagation(); if (event.nativeEvent.isComposing) return; if (event.key === 'Enter') { event.preventDefault(); void commitRename(true); } else if (event.key === 'Escape') { event.preventDefault(); cancelRename(); } }} />{renaming.saving && <Spinner aria-label="正在保存名称" className="absolute right-2 top-1/2 size-3 -translate-y-1/2" />}{renaming.error && <span id="backup-space-rename-error" role="alert" className="sr-only">{renaming.error}</span>}</>}
        </span></TooltipTrigger><TooltipContent>{editing && renaming.error ? renaming.error : label}</TooltipContent></Tooltip>;
      })}
      <TabsTrigger value="new" disabled={disabled} className={`${settingsTabsTriggerClassName} gap-1.5`}><Plus aria-hidden="true" />新建空间</TabsTrigger>
    </TabsList>
    <TabsContent value={selected}><form ref={formRef} noValidate onInput={event => { const input = event.target as HTMLInputElement; if (errors[input.id]) setErrors(previous => { const next = { ...previous }; delete next[input.id]; return next; }); }} className="backup-settings-form max-w-xl space-y-5" aria-label="远端备份设置" aria-busy={!!busy} onSubmit={event => { event.preventDefault(); void run('save'); }}>
    <FieldGroup className="gap-4">
      {selected === 'new' && <Field><FieldLabel htmlFor="backup-space-name">空间名称</FieldLabel><Input id="backup-space-name" maxLength={64} disabled={disabled} value={form.display_name ?? ''} placeholder="例如：个人备份" onChange={e => setForm({ ...form, display_name: e.target.value })} /></Field>}
      <Field><FieldLabel id="backup-provider-label">备份提供商<Required /></FieldLabel><Select value={form.profile} disabled={disabled} onValueChange={profile => { if (profile === 'r2' || profile === 's3') changeTarget({ ...form, profile, provider_type: 's3', region: profile === 'r2' ? 'auto' : 'us-east-1' }); }}><SelectTrigger aria-labelledby="backup-provider-label"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="r2">Cloudflare R2</SelectItem><SelectItem value="s3">通用 S3</SelectItem><SelectItem value="telegram" disabled>Telegram（计划支持）</SelectItem></SelectContent></Select></Field>
      {(['endpoint', 'bucket', 'prefix'] as const).map(name => <Field key={name}><FieldLabel htmlFor={`backup-${name}`}>{({ endpoint: form.profile === 'r2' ? 'R2 Endpoint' : 'S3 Endpoint', bucket: 'Bucket', prefix: 'Prefix' })[name]}<Required /></FieldLabel><Input id={`backup-${name}`} {...validation(`backup-${name}`)} type={name === 'endpoint' ? 'url' : 'text'} required disabled={disabled} autoComplete="off" spellCheck={false} value={form[name]} placeholder={name === 'endpoint' ? 'https://<account-id>.r2.cloudflarestorage.com' : undefined} onChange={e => changeTarget({ ...form, [name]: e.target.value })} />{fieldError(`backup-${name}`)}</Field>)}
      {form.profile === 's3' && <><Field><FieldLabel htmlFor="backup-region">Region<Required /></FieldLabel><Input id="backup-region" {...validation('backup-region')} value={form.region} required disabled={disabled} onChange={e => changeTarget({ ...form, region: e.target.value })} />{fieldError('backup-region')}</Field><Field orientation="horizontal"><FieldLabel htmlFor="backup-path-style">使用路径样式</FieldLabel><Checkbox id="backup-path-style" checked={form.path_style} disabled={disabled} onCheckedChange={v => changeTarget({ ...form, path_style: v === true })} /></Field></>}
      <Field><FieldLabel htmlFor="backup-key">Access Key ID<Required /></FieldLabel><InputGroup><InputGroupInput id="backup-key" required {...validation('backup-key')} type={showKey ? 'text' : 'password'} autoComplete="new-password" spellCheck={false} disabled={disabled} value={key} onChange={e => { setKey(e.target.value); setCredentialsChanged(true); }} /><InputGroupAddon align="inline-end"><InputGroupButton size="icon-xs" disabled={disabled} aria-label={showKey ? '隐藏 Access Key ID' : '显示 Access Key ID'} aria-pressed={showKey} onClick={() => setShowKey(!showKey)}>{showKey ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup>{fieldError('backup-key')}</Field>
      <Field><FieldLabel htmlFor="backup-secret">Secret Access Key<Required /></FieldLabel><InputGroup><InputGroupInput id="backup-secret" required {...validation('backup-secret')} type={showSecret ? 'text' : 'password'} autoComplete="new-password" spellCheck={false} disabled={disabled} value={secret} onChange={e => { setSecret(e.target.value); setCredentialsChanged(true); }} /><InputGroupAddon align="inline-end"><InputGroupButton size="icon-xs" disabled={disabled} aria-label={showSecret ? '隐藏 Secret Access Key' : '显示 Secret Access Key'} aria-pressed={showSecret} onClick={() => setShowSecret(!showSecret)}>{showSecret ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon></InputGroup>{fieldError('backup-secret')}</Field>
      <Field orientation="horizontal"><FieldLabel htmlFor="backup-cleanup-records">自动清理已删除记录</FieldLabel><Checkbox id="backup-cleanup-records" checked={form.cleanup_deleted_records ?? false} disabled={disabled} onCheckedChange={value => setForm({ ...form, cleanup_deleted_records: value === true })} /></Field>
      {form.cleanup_deleted_records && <Field><FieldLabel htmlFor="backup-retention-days">保留天数<Required /></FieldLabel><Input id="backup-retention-days" type="number" min={1} max={3650} step={1} required disabled={disabled} {...validation('backup-retention-days')} value={form.deleted_record_retention_days ?? ''} onChange={event => setForm({ ...form, deleted_record_retention_days: event.target.value === '' ? undefined : Number(event.target.value) })} />{fieldError('backup-retention-days')}</Field>}
    </FieldGroup>
    {selectedSpace?.last_error && !(enabled && selected === current && currentSpace?.connection_pending) && <p role="alert" className="text-sm text-destructive">{errorText(selectedSpace.last_error)}</p>}
    <div className="flex flex-wrap gap-2">{selectedSpace && selected !== current && <Tooltip><TooltipTrigger asChild><span className="inline-flex"><Button type="button" className="backup-save-button backup-feedback-button" variant="outline" disabled={disabled || !!switchReason} aria-label="设为默认" aria-busy={busy === 'switch'} onClick={() => void switchSpace()}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'switch'}>设为默认</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'switch'}><Spinner />设置中…</span></Button></span></TooltipTrigger><TooltipContent>{switchReason || '设为默认备份渠道'}</TooltipContent></Tooltip>}<Button type="submit" className="backup-save-button" disabled={disabled} aria-busy={busy === 'save'} aria-label={busy === 'save' ? '保存中…' : saveLabel}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'save'}>{saveLabel}</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'save'}><Spinner aria-hidden="true" />保存中…</span></Button><Button type="button" variant="outline" className="min-w-28" disabled={disabled || !key || !secret} aria-busy={busy === 'test'} onClick={() => void run('test')}>{busy === 'test' && <Spinner aria-hidden="true" />}{busy === 'test' ? '测试中…' : '测试连接'}</Button>{selectedSpace && <Button type="button" className="backup-save-button backup-feedback-button" variant="destructive" disabled={disabled} aria-label="删除配置" aria-busy={busy === 'delete'} onClick={() => void deleteSpace()}><span aria-hidden="true" className="backup-save-label" data-visible={busy !== 'delete'}><Trash2 />删除配置</span><span aria-hidden="true" className="backup-save-progress" data-visible={busy === 'delete'}><Spinner />删除中…</span></Button>}</div>
  </form></TabsContent>
  <AlertDialog open={confirmDelete} onOpenChange={open => { if (!busy) setConfirmDelete(open); }}><AlertDialogContent className="backup-settings-dialog"><AlertDialogHeader><AlertDialogTitle>删除备份配置？</AlertDialogTitle><AlertDialogDescription>此配置仍被视频或同步任务使用。删除后，关联视频将无法通过此配置同步。本机视频、远端文件和未同步文案会保留。</AlertDialogDescription></AlertDialogHeader>{actionError && <p role="alert" className="text-xs text-destructive">{actionError}</p>}<AlertDialogFooter><AlertDialogCancel disabled={disabled}>取消</AlertDialogCancel><AlertDialogAction disabled={disabled} onClick={event => { event.preventDefault(); void deleteSpace(true); }}>{busy === 'delete' && <Spinner />}确认删除</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </Tabs></div></div></div>;
}

function draft(): BackupUpdate { return { cleanup_deleted_records: false, deleted_record_retention_days: 30, display_name: '', provider_type: 's3', profile: 'r2', endpoint: '', bucket: '', prefix: 'yingqu/', region: 'auto', path_style: true, enabled: true }; }
function update(v: BackupSettings): BackupUpdate { const { provider_type, profile, endpoint, bucket, prefix, region, path_style, enabled } = v; return { cleanup_deleted_records: v.cleanup_deleted_records ?? false, deleted_record_retention_days: v.deleted_record_retention_days ?? 30, display_name: v.display_name ?? '', provider_type, profile, endpoint, bucket, prefix, region, path_style, enabled }; }
function spaceName(v: BackupUpdate) { return v.display_name?.trim() ?? ''; }
function spaceLabel(v: BackupSettings) { return spaceName(v) || `${v.profile === 'r2' ? 'R2' : 'S3'} / ${v.bucket}`; }

function Required() { return <span aria-hidden="true" className="backup-required-mark text-destructive" />; }

function visibleFeedback(started: number) { return new Promise<void>(resolve => setTimeout(resolve, Math.max(0, 300 - (performance.now() - started)))); }
