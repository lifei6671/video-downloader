import { useEffect, useState } from 'react';
import { api, errorText, type BackupAsset, type BackupConflict, type BackupLocalData } from '@/api';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Textarea } from '@/components/ui/textarea';
import { Spinner } from '@/components/ui/spinner';
import { toast } from 'sonner';

const names: Record<string, string> = { title: '标题', author: '作者', transcript: '文案', cover: '封面', source: '来源', original_filename: '文件名', duration_ms: '时长', width: '宽度', height: '高度', completed_at: '完成时间' };
export function BackupDataDialog({ asset, open, close, changed }: { asset: BackupAsset; open: boolean; close: () => void; changed: () => Promise<void> }) {
  const [data, setData] = useState<BackupLocalData | null>(null), [version, setVersion] = useState<BackupConflict | null>(null);
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [image, setImage] = useState('');
  useEffect(() => { if (!open) return; let live = true; setData(null); setVersion(null); setError(''); void api.backupLocalData(asset.space_id, asset.asset_id).then(v => { if (live) setData(v); }).catch(e => { if (live) setError(errorText(e)); }); return () => { live = false; }; }, [open, asset.space_id, asset.asset_id]);
  useEffect(() => { if (!version?.image) { setImage(''); return; } const url = URL.createObjectURL(new Blob([new Uint8Array(version.image)])); setImage(url); return () => URL.revokeObjectURL(url); }, [version]);
  async function read(v: BackupConflict) {
    setBusy(true); setError('');
    try { setVersion(await api.backupConflict(asset.space_id, asset.asset_id, v.id)); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  async function resolve(adopt: boolean) {
    if (!version) return;
    setBusy(true); setError('');
    try { await api.resolveBackupConflict(asset.space_id, asset.asset_id, version.id, adopt); await changed(); toast.success('选择已保存，等待同步确认'); close(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  return <Dialog open={open} onOpenChange={v => { if (!v) close(); }}><DialogContent className="flex max-h-[85dvh] flex-col sm:max-w-[720px]"><DialogHeader><DialogTitle>{asset.remote_deleted ? '本机待处理' : '冲突版本'}</DialogTitle><DialogDescription className="break-all">{asset.metadata.title || asset.original_filename}</DialogDescription></DialogHeader>
    <div className="min-h-0 space-y-4 overflow-y-auto pr-2">
      {error && <p role="alert" className="break-all text-sm text-destructive">{error}</p>}
      {!data && !error && <Spinner aria-label="正在读取本机数据" />}
      {data?.remote_deleted && <p className="text-sm text-muted-foreground">云端已删除，本机文案及冲突版本已保留。</p>}
      {data?.transcript && <Textarea aria-label="本机保留文案" readOnly value={data.transcript} className="min-h-40 text-sm" />}
      {data && <Button variant="outline" disabled={busy} onClick={() => { setBusy(true); void api.exportBackupLocalData(asset.space_id, asset.asset_id).then(path => { if (path) toast.success('本机数据已导出'); }).catch(e => setError(errorText(e))).finally(() => setBusy(false)); }}>导出本机数据</Button>}
      {data?.conflicts.map(v => <Button key={v.id} variant={version?.id === v.id ? 'secondary' : 'outline'} className="w-full justify-start" disabled={busy} onClick={() => void read(v)}>{names[v.field] || v.field}冲突版本</Button>)}
      {version && <div className="space-y-3"><p className="text-sm font-medium">{names[version.field] || version.field}版本</p>{image ? <img src={image} alt="冲突封面版本" className="max-h-72 max-w-full object-contain" /> : <Textarea aria-label="冲突版本内容" readOnly value={version.value} className="min-h-40 text-sm" />}
        {version.can_adopt && <div className="flex flex-wrap gap-2"><Button disabled={busy || data?.pending} onClick={() => void resolve(true)}>采用此版本</Button><Button variant="outline" disabled={busy || data?.pending} onClick={() => void resolve(false)}>保留当前版本</Button></div>}
        {data?.pending && !data.remote_deleted && <p className="text-xs text-muted-foreground">本机变更同步确认后可处理冲突。</p>}
      </div>}
      {busy && <Spinner aria-label="正在处理" />}
    </div>
  </DialogContent></Dialog>;
}
