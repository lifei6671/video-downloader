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
  const id = profile?.id?.trim() || (video.platform === 'telegram' ? video.video_id.split('_')[0] : null);
  const handle = profile?.handle?.trim() || null;
  const legacyAuthor = video.author?.trim() || null;
  if (!id && !handle && !legacyAuthor) return null;
  const name = profile?.nickname?.trim() || handle || legacyAuthor || (id ? video.platform === 'telegram' ? 'Telegram 频道' : '作者未知' : null);
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

export function matchingAuthorRecords<T extends { video: Video | null }>(records: T[], identity: AuthorSearch): T[] {
  const matches = records.filter(record => matchesAuthor(record.video, identity));
  if (identity.id) return matches;
  const knownIds = new Set(matches.map(record => authorSearch(record.video)?.id).filter(Boolean));
  return knownIds.size > 1 ? matches.filter(record => !authorSearch(record.video)?.id) : matches;
}
