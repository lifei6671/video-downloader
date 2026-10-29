import { useCallback, useEffect, useState } from 'react';
import { Archive, FolderOpen, RotateCcw } from 'lucide-react';
import { errorText } from '@/api';
import { libraryApi, type LibraryBackupInfo } from '@/lib/library-api';
import type { LibraryTransferClient } from '@/hooks/use-library-transfer';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Progress } from '@/components/ui/progress';
import { Spinner } from '@/components/ui/spinner';
import { displayPath } from '@/lib/path';

const bytes = (value: number) => value < 1048576 ? `${(value / 1024).toFixed(1)} KB` : value < 1073741824 ? `${(value / 1048576).toFixed(1)} MB` : `${(value / 1073741824).toFixed(2)} GB`;
const phases: Record<string, string> = { preparing: '准备中', copying: '复制文件', validating: '校验资料', committing: '完成保存' };
export function LibraryBackupSettings({ active, client }: { active: boolean; client: LibraryTransferClient }) {
  const [includeMedia, setIncludeMedia] = useState(false);
  const [info, setInfo] = useState<LibraryBackupInfo>();
  const [error, setError] = useState('');
  const [subscriptionError, setSubscriptionError] = useState('');
  const [connection, setConnection] = useState(0);
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  const snapshot = client.snapshot;
  const running = snapshot?.state === 'running';
  const busy = client.picking || running;
  const scopeIncludesMedia = running && snapshot.kind === 'backup' ? snapshot.include_media : includeMedia;
  useEffect(() => {
    if (!active) return;
    setSubscriptionError('');
    let disposed = false; let stop: (() => void) | undefined;
    void libraryApi.subscribe(() => { if (!disposed) refresh(); }).then(unsubscribe => { if (disposed) unsubscribe(); else { stop = unsubscribe; refresh(); } })
      .catch(reason => { if (!disposed) setSubscriptionError(errorText(reason)); });
    window.addEventListener('focus', refresh);
    return () => { disposed = true; stop?.(); window.removeEventListener('focus', refresh); };
  }, [active, refresh, connection]);
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void libraryApi.backupInfo().then(value => { if (!disposed) { setInfo(value); setError(''); } }).catch(reason => { if (!disposed) setError(errorText(reason)); });
    return () => { disposed = true; };
  }, [active, revision, snapshot?.state]);
  return <div className="library-backup-settings" hidden={!active}>
    <LibraryError message={client.error || error || subscriptionError} />
    {(client.error || error || subscriptionError) && <Button size="sm" variant="outline" onClick={() => { client.reconnect(); setConnection(value => value + 1); refresh(); }}>重新读取备份状态</Button>}
    <section aria-label="素材库备份" className="library-backup-section">
      <div className="library-section-heading"><h3>备份</h3><Button variant="ghost" size="icon-sm" aria-label="刷新备份范围" disabled={busy} onClick={refresh}><RotateCcw /></Button></div>
      <FieldGroup><Field><FieldLabel>备份范围</FieldLabel><ToggleGroup type="single" variant="outline" value={scopeIncludesMedia ? 'media' : 'material'} disabled={busy} onValueChange={value => { if (value) setIncludeMedia(value === 'media'); }} aria-label="备份范围"><ToggleGroupItem value="material">仅资料备份</ToggleGroupItem><ToggleGroupItem value="media">包含视频</ToggleGroupItem></ToggleGroup></Field></FieldGroup>
      {info ? <div className="library-backup-summary"><p>{info.assets} 个视频资料，{info.documents} 篇文案，{info.notes} 条笔记，{info.topics ?? 0} 个专题，{info.ideas ?? 0} 个选题</p>{scopeIncludesMedia && <p>可用视频 {info.available_files}/{info.files} 个，约 {bytes(info.media_bytes)}</p>}</div> : !error && <p role="status" className="text-xs text-muted-foreground">正在读取备份范围…</p>}
      <Button size="sm" className="self-start" disabled={busy || !info || !!error} onClick={() => void client.start('backup', includeMedia)}>{client.pickingKind === 'backup' ? <Spinner /> : <Archive />}选择位置并备份</Button>
    </section>
    <section aria-label="素材库恢复" className="library-backup-section">
      <h3>恢复</h3>
      <p className="text-xs text-muted-foreground">{info && !info.empty ? '素材库已有资料，请在空素材库中恢复。' : info?.busy ? '请先结束下载、导入或文案提取，再恢复素材库。' : '仅支持空素材库，恢复时保留原有版本与引用。'}</p>
      <Button size="sm" variant="outline" className="self-start" disabled={busy || !info?.empty || info.busy || !!error} onClick={() => void client.start('restore')}>{client.pickingKind === 'restore' ? <Spinner /> : <FolderOpen />}选择备份包并恢复</Button>
    </section>
    {client.notice && <p role="status" className="text-xs text-muted-foreground">{client.notice}</p>}
    {snapshot && <section aria-label="备份恢复操作" className="library-backup-section" aria-live="polite">
      <h3>{snapshot.kind === 'backup' ? '备份' : '恢复'}{running ? '进行中' : snapshot.state === 'completed' ? '完成' : snapshot.state === 'canceled' ? '已取消' : '失败'}</h3>
      {running ? <><p className="text-xs text-muted-foreground">{snapshot.canceling ? '正在取消…' : phases[snapshot.phase] ?? '处理中'}</p><Progress value={snapshot.total_files > 0 ? Math.min(100, snapshot.completed_files / snapshot.total_files * 100) : undefined} aria-label="文件处理进度" /><p className="text-xs text-muted-foreground">{snapshot.completed_files}/{snapshot.total_files} 个文件，已处理 {bytes(snapshot.bytes)}</p><Button size="sm" variant="outline" className="self-start" disabled={client.canceling || snapshot.canceling || snapshot.phase === 'committing'} onClick={() => void client.cancel()}>取消操作</Button></> : snapshot.state === 'failed' ? <LibraryError message={errorText(snapshot.error)} /> : snapshot.state === 'completed' && snapshot.result ? <><p className="text-xs">{snapshot.result.assets} 个视频资料{snapshot.kind === 'restore' ? `，${snapshot.result.documents} 篇文案，${snapshot.result.notes} 条笔记` : `，${snapshot.result.files} 个视频文件`}，{snapshot.result.topics ?? 0} 个专题，{snapshot.result.ideas ?? 0} 个选题</p><p className="text-xs text-muted-foreground">{snapshot.result.missing_media} 个文件未随包提供{snapshot.kind === 'restore' && snapshot.result.missing_media > 0 ? '，可在素材库中重新关联' : ''}</p></> : null}
      <p className="library-transfer-path">{displayPath(snapshot.path)}</p>
    </section>}
    <details className="library-backup-explanation"><summary>备份说明</summary><p>资料备份用于迁移素材库，默认不含大视频。请完整保留备份目录。</p><p>升级前的数据库备份仅用于本机版本回退，可能包含应用设置与凭据，不作为资料迁移包。</p></details>
  </div>;
}
