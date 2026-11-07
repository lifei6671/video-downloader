import type { ReactNode } from 'react';
import { Check, CircleX } from 'lucide-react';
import { cn } from 'cn';
import { errorText, type ResolvePhase, type Task } from '@/api';
import { Card, CardAction, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Spinner } from '@/components/ui/spinner';
import { Progress } from '@/components/ui/progress';

const phaseStep: Record<ResolvePhase, number> = {
  checking_link: 0, checking_cache: 0, starting_engine: 0,
  loading_configuration: 0, resolving_link: 0, connecting_account: 0, fetching_metadata: 0,
  selecting_formats: 1,
  loading_preview: 2, loading_author: 2, finalizing: 2, cache_hit: 2, completed: 2,
};
const steps = ['读取作品信息', '整理可用规格', '准备解析结果'];

export function ResolveProgressCard({ task, source, error, actions }: { task?: Task; source: string; error?: string; actions?: ReactNode }) {
  const progress = task?.resolve_progress;
  const failed = !!error || task?.status === 'failed';
  const canceled = task?.status === 'canceled';
  const stopped = failed || canceled;
  const currentStep = progress ? phaseStep[progress.phase] : 0;
  const status = failed ? '解析失败' : canceled ? '解析已取消' : task?.canceling ? '正在取消解析' : '正在解析';
  const message = error || (failed && task?.error ? errorText(task.error) : '');
  let host = '';
  try { host = new URL(task?.video?.canonical_url ?? task?.telegram_link ?? source.match(/https?:\/\/[^\s<>]+/i)?.[0] ?? '').hostname; } catch { /* The input may not contain a valid URL. */ }
  return <Card size="sm" className="[--card-spacing:16px]" data-slot="resolve-progress">
    <CardHeader className="items-center">
      <CardTitle className="flex min-w-0 items-center gap-3">
        {host && <span className="truncate font-mono font-normal">{host}</span>}
        <span className="flex shrink-0 items-center gap-2 text-xs font-normal text-muted-foreground"><span className={cn('size-1.5 rounded-full bg-primary', stopped && 'bg-muted-foreground')} aria-hidden="true" />{status}</span>
      </CardTitle>
      {!stopped && actions && <CardAction>{actions}</CardAction>}
    </CardHeader>
    <CardContent className="flex flex-col gap-4">
      <Progress value={(currentStep + 1) * 100 / steps.length} aria-label="解析阶段" aria-valuetext={`第 ${currentStep + 1} 步，共 ${steps.length} 步`} className="h-0.5" />
      <span role="status" aria-live="polite" aria-atomic="true" data-resolve-phase={progress?.phase ?? 'submitting'} className="sr-only">{status}，{steps[currentStep]}</span>
      <ol aria-label="解析步骤" className="flex flex-col gap-1">
        {steps.map((label, index) => {
          const active = index === currentStep;
          const done = index < currentStep && !!progress?.steps.some(phase => phaseStep[phase] === index);
          return <li key={label} data-resolve-step={index} data-state={active ? 'active' : done ? 'done' : 'pending'} aria-current={active ? 'step' : undefined} className={cn('flex min-h-9 items-center gap-3 text-muted-foreground', active && 'font-medium text-foreground')}>
            <span className="w-5 shrink-0 font-mono text-xs font-normal text-muted-foreground">{String(index + 1).padStart(2, '0')}</span>
            <span className="flex-1">{label}</span>
            {active ? stopped ? <CircleX className="size-4 shrink-0" aria-hidden="true" /> : <Spinner className="size-4 shrink-0 text-primary" aria-hidden="true" /> : done ? <Check className="size-4 shrink-0" aria-hidden="true" /> : <span className="mx-1.5 size-1 shrink-0 rounded-full bg-muted-foreground/40" aria-hidden="true" />}
          </li>;
        })}
      </ol>
      {task?.status === 'parsing' && task.error === 'TELEGRAM_RECONNECTING' && <p role="status" className="text-xs text-muted-foreground">网络连接中断，正在重连</p>}
      {message && <Alert variant="destructive"><AlertDescription>{message}</AlertDescription></Alert>}
    </CardContent>
    {stopped && actions && <CardFooter className="justify-end py-2">{actions}</CardFooter>}
  </Card>;
}
