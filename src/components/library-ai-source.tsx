import type { AiSource } from '@/lib/library-api';
import { Badge } from '@/components/ui/badge';

export function AiSourceView({ source }: { source?: AiSource }) {
  if (!source) return null;
  return <details className="min-w-0 break-all text-xs text-muted-foreground"><summary><Badge variant="outline">AI 创建</Badge> {source.model_name}</summary>
    {source.citations.map((citation, index) => <blockquote className="library-note-citation" key={index}><p>{citation.quote}</p><p>资产 {citation.asset_id} / 文案 {citation.document_id}</p></blockquote>)}
  </details>;
}
