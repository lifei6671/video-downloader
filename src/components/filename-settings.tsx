import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { RotateCcw } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorText, type Settings, type FilenamePreview } from '@/api';
import { displayPath } from '@/lib/path';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardAction, CardContent, CardFooter } from '@/components/ui/card';
import { Field, FieldGroup, FieldLabel, FieldDescription, FieldError } from '@/components/ui/field';
import { Input } from '@/components/ui/input';

const defaultTemplate = '{platform}/{user_name}-{content}';
const variables = [
  ['platform', '平台'], ['user_name', '作者昵称'], ['link_id', '作品 ID'],
  ['content', '作品描述'], ['current_time', '当前时间'], ['quality', '清晰度'],
];

export type FilenamePreviewCache = { template: string; preview: FilenamePreview };

export function FilenameSettings({ settings, saved, previewCache }: { settings: Settings; saved: (settings: Settings) => void; previewCache: RefObject<FilenamePreviewCache | null> }) {
  const savedTemplate = settings.filename_template ?? defaultTemplate;
  const cached = previewCache.current?.template === savedTemplate ? previewCache.current.preview : null;
  const [template, setTemplate] = useState(settings.filename_template ?? defaultTemplate);
  const [preview, setPreview] = useState<FilenamePreview | null>(cached);
  const [error, setError] = useState('');
  const [checking, setChecking] = useState(!cached);
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const caret = useRef<number | null>(null);
  const dirty = template !== (settings.filename_template ?? defaultTemplate);

  useLayoutEffect(() => {
    if (caret.current !== null) {
      input.current?.focus();
      input.current?.setSelectionRange(caret.current, caret.current);
      caret.current = null;
    }
  }, [template]);

  useEffect(() => {
    if (previewCache.current?.template === template) {
      setPreview(previewCache.current.preview);
      setError('');
      setChecking(false);
      return;
    }
    let active = true;
    setChecking(true);
    const timer = setTimeout(() => {
      api.previewFilename(template).then(value => {
        if (active) {
          setPreview(value); setError('');
          if (template === savedTemplate) previewCache.current = { template, preview: value };
        }
      }).catch(reason => {
        if (active) { setPreview(null); setError(errorText(reason)); }
      }).finally(() => { if (active) setChecking(false); });
    }, 150);
    return () => { active = false; clearTimeout(timer); };
  }, [template, savedTemplate, previewCache]);

  function change(value: string) {
    if (value === template) return;
    setTemplate(value);
    setChecking(true);
    setError('');
  }

  function insert(variable: string) {
    const start = input.current?.selectionStart ?? template.length;
    const end = input.current?.selectionEnd ?? start;
    const token = `{${variable}}`;
    const next = template.slice(0, start) + token + template.slice(end);
    if (next === template) {
      input.current?.focus();
      input.current?.setSelectionRange(start + token.length, start + token.length);
      return;
    }
    caret.current = start + token.length;
    change(next);
  }

  async function save() {
    if (saving || checking || error || !dirty) return;
    setSaving(true);
    try {
      const next = await api.filenameTemplate(template);
      if (preview) previewCache.current = { template, preview };
      saved(next);
      toast.success('文件命名规则已保存，仅影响新提交的下载');
    } catch (reason) { toast.error(errorText(reason)); }
    finally { setSaving(false); }
  }

  return <Card size="sm" className="shrink-0">
    <CardHeader>
      <CardTitle>文件命名规则</CardTitle>
      <CardDescription>使用变量组合下载文件的路径与名称。</CardDescription>
      <CardAction><Button variant="secondary" disabled={saving || template === defaultTemplate} onClick={() => change(defaultTemplate)}><RotateCcw data-icon="inline-start" />恢复默认</Button></CardAction>
    </CardHeader>
    <CardContent><FieldGroup className="gap-4">
      <Field data-invalid={!!error}>
        <FieldLabel htmlFor="filename-template" className="sr-only">文件命名规则</FieldLabel>
        <Input id="filename-template" ref={input} value={template} disabled={saving} maxLength={512} autoComplete="off" spellCheck={false} aria-invalid={!!error} aria-describedby="filename-template-help filename-template-error" onChange={event => change(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); void save(); } }} />
        <FieldDescription id="filename-template-help">用 / 分隔文件夹，扩展名自动保留。图集规则用于目录名称。</FieldDescription>
        <FieldError id="filename-template-error">{error}</FieldError>
      </Field>
      <Field>
        <FieldLabel>快捷插入变量</FieldLabel>
        <div className="flex flex-wrap gap-2">{variables.map(([variable, label]) => <Button key={variable} variant="outline" className="filename-variable flex-col items-start gap-0.5" disabled={saving} aria-label={`插入${label}`} onClick={() => insert(variable)}><span>{`{${variable}}`}</span><span className="text-xs font-normal text-muted-foreground">{label}</span></Button>)}</div>
      </Field>
      <Field>
        <FieldLabel id="filename-preview-label">实时预览</FieldLabel>
        <div role="status" aria-busy={checking} aria-labelledby="filename-preview-label" className="filename-preview min-h-12 rounded-md border-[0.5px] bg-surface-subtle p-3 break-all">{preview ? preview.segments.map((segment, index) => segment.variable ? <span key={index} data-filename-variable={segment.variable} title={`{${segment.variable}}`}>{segment.text}</span> : <span key={index}>{segment.text}</span>) : <span className="text-xs text-muted-foreground">{checking ? '正在预览…' : '请修正规则后预览'}</span>}</div>
        <FieldDescription className="break-all">{preview ? `示例路径：${displayPath(settings.download_directory).replace(/[\\/]$/, '')}/${preview.path}` : '预览使用示例作品'}</FieldDescription>
      </Field>
    </FieldGroup></CardContent>
    <CardFooter className="justify-end"><Button disabled={saving || checking || !!error || !dirty} onClick={() => void save()}>{saving ? '保存中…' : '保存命名规则'}</Button></CardFooter>
  </Card>;
}
