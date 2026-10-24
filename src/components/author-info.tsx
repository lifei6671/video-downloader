import { useEffect, useState } from 'react';
import { api, type Task } from '@/api';
import { Avatar, AvatarImage, AvatarFallback } from '@/components/ui/avatar';
import { Button } from '@/components/ui/button';
import { cn } from 'cn';

export function AuthorInfo({ task, compact = false, showId = true, showAvatar = true, onAuthorClick }: { task: Task; compact?: boolean; showId?: boolean; showAvatar?: boolean; onAuthorClick?: () => void }) {
  const profile = task.video?.author_profile;
  const [source, setSource] = useState<string>();
  useEffect(() => {
    let disposed = false;
    let url: string | undefined;
    setSource(undefined);
    if (showAvatar && profile?.avatar_cached) void api.authorAvatar(task.id).then(data => {
      if (disposed) return;
      const body = new Uint8Array(data);
      url = URL.createObjectURL(new Blob([body], { type: body[0] === 0xff ? 'image/jpeg' : body[0] === 0x89 ? 'image/png' : 'image/webp' }));
      setSource(url);
    }).catch(() => { /* The official Avatar fallback covers missing local bytes. */ });
    return () => { disposed = true; if (url) URL.revokeObjectURL(url); };
  }, [task.id, task.cached_at, profile?.id, profile?.avatar_cached, showAvatar]);
  if (!task.video) return null;
  const telegram = task.video.platform === 'telegram';
  const authorId = profile?.id?.trim() || (telegram ? task.video.video_id.split('_')[0] : undefined);
  const id = showId ? authorId : undefined;
  const name = profile?.nickname?.trim() || (!showId && authorId) || profile?.handle || task.video.author || (telegram ? 'Telegram 频道' : '作者未知');
  return <div data-author-info="" role="group" aria-label={telegram ? '频道信息' : '作者信息'} className="flex min-w-0 items-center gap-2">
    {showAvatar && <Avatar size={compact ? 'sm' : 'default'}>
      <AvatarImage src={source} alt={`${name}的头像`} />
      <AvatarFallback>{Array.from(name)[0]}</AvatarFallback>
    </Avatar>}
    <div className="flex min-w-0 flex-1 items-baseline gap-2">
      {onAuthorClick ? <Button variant="link" title={name} aria-label={`查看${name}的作品`} onClick={onAuthorClick} className={cn('h-auto min-w-0 justify-start p-0', id && 'max-w-[40%] shrink-0', compact ? 'text-xs font-normal' : 'text-sm font-medium')}><span className="truncate">{name}</span></Button>
        : <span title={name} className={cn('truncate', id && 'max-w-[40%] shrink-0', compact ? 'text-xs' : 'text-sm font-medium')}>{name}</span>}
      {id && <span title={`ID: ${id}`} className="max-w-full truncate text-xs text-muted-foreground">ID: {id}</span>}
    </div>
  </div>;
}
