import { useLayoutEffect, useRef } from 'react';
import type { Task } from '@/api';
import { AuthorAvatar, hasAuthorAvatar } from '@/components/author-info';
import { authorSearch, type AuthorSearch } from '@/lib/author-search';

const countFormat = new Intl.NumberFormat('zh-CN');
const compactCountFormat = new Intl.NumberFormat('zh-CN', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const counts = [['follower_count', '粉丝'], ['following_count', '关注'], ['likes_count', '获赞']] as const;
const labels = [['region', '地区'], ['ip_location', 'IP 属地']] as const;

function formatFollowers(count: number) {
  if (count >= 100_000_000) return `${compactCountFormat.format(count / 100_000_000)}亿`;
  if (count >= 10_000_000) return `${compactCountFormat.format(count / 10_000_000)}千万`;
  if (count >= 10_000) return `${compactCountFormat.format(count / 10_000)}万`;
  return countFormat.format(count);
}

export function AuthorProfileHeader({ identity, source, tasks, platformName }: {
  identity: AuthorSearch;
  source: Task;
  tasks: Task[];
  platformName: string;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useLayoutEffect(() => { heading.current?.focus({ preventScroll: true }); }, []);

  // Weak matches can still appear as old records, but cannot supply another author's profile.
  const authorId = identity.id ?? tasks.map(task => authorSearch(task.video)?.id).find(Boolean);
  const profileTasks = tasks.filter(task => !authorId || authorSearch(task.video)?.id === authorId);
  const current = profileTasks.find(task => task.id === source.id) ?? profileTasks[0];
  const profile = current?.video?.author_profile;
  const name = profile?.nickname?.trim() || identity.name;
  const handle = profile?.handle?.trim() || identity.handle;
  const bio = profile?.bio?.trim() || profileTasks.find(task => task.video?.author_profile?.bio?.trim())?.video?.author_profile?.bio?.trim()
    || source.video?.author_profile?.bio?.trim();
  const avatar = profile?.avatar_cached ? current : profileTasks.find(task => task.video?.author_profile?.avatar_cached)
    ?? (current && hasAuthorAvatar(current) ? current : profileTasks.find(hasAuthorAvatar));
  const storedProfile = profileTasks.find(task => task.video?.author_profile?.profile_updated_at != null)?.video?.author_profile;
  const profiles = storedProfile ? [storedProfile] : [profile, ...profileTasks.map(task => task.video?.author_profile)];
  const age = profiles.find(item => item?.age != null)?.age;

  return <section aria-labelledby="author-profile-name" className="author-profile-header">
    <AuthorAvatar task={avatar} name={name} className="size-16" />
    <div className="min-w-0">
      <h1 ref={heading} id="author-profile-name" tabIndex={-1} className="text-xl font-semibold leading-7 break-words outline-none">{name}</h1>
      <dl className="author-profile-metadata">
        <div><dt className="sr-only">平台</dt><dd>{platformName}</dd></div>
        {handle && <div><dt className="sr-only">账号名</dt><dd>{handle.startsWith('@') ? handle : `@${handle}`}</dd></div>}
        {identity.platform !== 'douyin' && authorId && <div><dt>{identity.platform === 'telegram' ? '频道 ID' : '作者 ID'}</dt><dd>{authorId}</dd></div>}
        {counts.map(([field, label]) => {
          const count = profiles.find(item => item?.[field] != null)?.[field];
          return count == null ? null : <div key={field}><dt>{label}</dt><dd className="tabular-nums">{field === 'follower_count' ? formatFollowers(count) : countFormat.format(count)}</dd></div>;
        })}
        {age != null && <div><dt>年龄</dt><dd className="tabular-nums">{age} 岁</dd></div>}
        {labels.map(([field, label]) => {
          const value = profiles.find(item => item?.[field]?.trim())?.[field]?.trim();
          return value ? <div key={field}><dt>{label}</dt><dd>{value}</dd></div> : null;
        })}
      </dl>
      {bio && <p className="author-profile-bio">{bio}</p>}
    </div>
  </section>;
}
