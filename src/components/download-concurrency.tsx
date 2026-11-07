import { useEffect, useState } from 'react';
import { api, errorText, type Settings } from '@/api';
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/ui/field';
import { Slider } from '@/components/ui/slider';

export function DownloadConcurrency({ value, disabled, saved }: { value: number; disabled: boolean; saved: (settings: Settings) => void }) {
  const [draft, setDraft] = useState(value);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { setDraft(value); }, [value]);

  async function commit(values: number[]) {
    const next = values[0];
    if (disabled || saving || next === value) return;
    setSaving(true);
    setError('');
    try {
      saved(await api.concurrency(next));
    } catch (reason) {
      setDraft(value);
      setError(errorText(reason));
    } finally { setSaving(false); }
  }

  return <Field className="download-concurrency" aria-busy={saving} data-invalid={!!error}>
    <div className="flex items-center justify-between gap-3">
      <FieldLabel id="concurrency-label">同时下载数量</FieldLabel>
      <span className="text-sm font-medium text-primary" aria-live="polite">{draft} 个</span>
    </div>
    <div className="download-concurrency-stops" aria-hidden="true">{[1,2,3,4,5,6,7,8,9,10].map(count => <span key={count} style={{ left: `${(count - 1) * 100 / 9}%` }} data-selected={draft === count}>{count}{count === 5 ? '（默认）' : ''}</span>)}</div>
    <Slider min={1} max={10} step={1} value={[draft]} disabled={disabled || saving} onValueChange={values => { setDraft(values[0]); setError(''); }} onValueCommit={values => void commit(values)} aria-labelledby="concurrency-label" aria-describedby={error ? 'concurrency-help concurrency-error' : 'concurrency-help'} aria-invalid={!!error} aria-valuetext={`${draft} 个任务`} />
    <FieldDescription id="concurrency-help">最多同时下载的任务数；降低上限不会中断已开始的下载。</FieldDescription>
    {error && <FieldError id="concurrency-error">{error}</FieldError>}
  </Field>;
}
