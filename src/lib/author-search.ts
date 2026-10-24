import type { Video } from '../api';

export interface AuthorSearch {
  platform: string;
  id: string | null;
  handle: string | null;
  legacyAuthor: string | null;
  name: string;
}

export function authorSearch(video: Video | null): AuthorSearch | null {
  if (!video) return null;
  const profile = video.author_profile;
  const id = profile?.id || (video.platform === 'telegram' ? video.video_id.split('_')[0] : null);
  const handle = profile?.handle || null;
  const legacyAuthor = video.author || null;
  if (!id && !handle && !legacyAuthor) return null;
  const name = profile?.nickname || handle || legacyAuthor || (id ? video.platform === 'telegram' ? 'Telegram 频道' : '作者未知' : null);
  return name ? { platform: video.platform, id, handle, legacyAuthor, name } : null;
}

export function matchesAuthor(video: Video | null, query: AuthorSearch): boolean {
  const candidate = authorSearch(video);
  if (!candidate || candidate.platform !== query.platform) return false;
  if (candidate.id && query.id) return candidate.id === query.id;
  if (candidate.handle && query.handle) return candidate.handle === query.handle;
  // Old records only stored `author`. Never let that fallback override two known IDs.
  return !!candidate.legacyAuthor && (candidate.legacyAuthor === query.legacyAuthor
    || candidate.legacyAuthor === query.handle)
    || !!query.legacyAuthor && query.legacyAuthor === candidate.handle;
}
