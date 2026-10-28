import { useEffect, useRef, useState } from 'react';
import { libraryApi, type LibraryClip, type LibraryNote, type TopicMemberView, type TopicMember } from '@/lib/library-api';
import { errorText } from '@/api';
import { LibraryError } from '@/components/library-organization';
import { LibraryVideo } from '@/components/library-video';
import { ComparisonSourceView } from '@/components/library-comparison-source';
import { CitationReader } from '@/components/library-writing';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';

const missing = () => {};
export function TopicSourceDialog({ entry, close }: { entry: Pick<TopicMemberView, 'title' | 'asset_title' | 'archived'> & { member: Pick<TopicMember, 'kind' | 'asset_id' | 'resource_id'> }; close: () => void }) {
  const [note, setNote] = useState<LibraryNote>();
  const [clip, setClip] = useState<LibraryClip>();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [citation, setCitation] = useState<LibraryNote['citations'][number]>();
  const focus = useRef(document.activeElement as HTMLElement);
  const member = entry.member;
  useEffect(() => {
    let disposed = false;
    async function read() {
      try {
        if (member.kind === 'note') {
          const value = await libraryApi.note(member.asset_id, member.resource_id);
          if (value.asset_id !== member.asset_id || value.id !== member.resource_id) throw Error('来源资产不匹配');
          if (!disposed) setNote(value);
        } else if (member.kind === 'clip') {
          const values = await libraryApi.clips(member.asset_id);
          const value = values.find(c => c.asset_id === member.asset_id && c.id === member.resource_id);
          if (!value) throw Error('片段已缺失');
          if (!disposed) setClip(value);
        }
      } catch (reason) { if (!disposed) setError(errorText(reason)); }
      finally { if (!disposed) setLoading(false); }
    }
    void read(); return () => { disposed = true; };
  }, [member]);
  return <Dialog open onOpenChange={open => { if (!open) close(); }}><DialogContent className="topic-source-dialog" onCloseAutoFocus={event => { event.preventDefault(); if (focus.current?.isConnected) focus.current.focus({ preventScroll: true }); }}><DialogHeader><DialogTitle>{entry.title || '未命名资料'}</DialogTitle></DialogHeader><p className="text-xs text-muted-foreground">{entry.asset_title}{entry.archived ? ' / 已移出知识库' : ''}</p>{loading && <p role="status">读取中…</p>}<LibraryError message={error} />
    {clip && <><p>{clip.start_ms}–{clip.end_ms} ms</p><LibraryVideo assetId={member.asset_id} fileId={clip.file_id} clipId={clip.id} onMissing={missing} /></>}
    {note && <><p className="text-xs text-muted-foreground">r{note.revision} {note.origin === 'ai' ? `AI / ${note.model_name ?? '模型未知'}` : '人工'}</p><pre className="topic-source-text">{note.body}</pre>{note.comparison?.points.flatMap((p, i) => p.sources.map(s => <ComparisonSourceView key={`${i}-${s.pin.asset_id}`} source={s} />))}{note.citations.map((c, i) => <blockquote className="library-note-citation" key={i}><p>{c.quote}</p><Button size="sm" variant="outline" onClick={() => setCitation(c)}>查看引用原文</Button></blockquote>)}</>}
    {citation && <CitationReader citation={citation} close={() => setCitation(undefined)} />}
  </DialogContent></Dialog>;
}
