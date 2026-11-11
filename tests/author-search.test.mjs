import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mergeTasks } from '../src/api.ts';
import { authorSearch, matchingAuthorRecords, matchesAuthor } from '../src/lib/author-search.ts';

const video = (platform, id, nickname = '同名作者', author = 'account') => ({platform, video_id:'123', author,
  author_profile:{id, nickname, handle:author, kind:'user'}});

test('author search uses platform and ID, not nicknames, substrings or mutable handles', () => {
  const query = authorSearch(video('douyin', '123'));
  assert(matchesAuthor(video('douyin', '123', '新昵称', 'new-handle'), query));
  assert(!matchesAuthor(video('douyin', '1234'), query));
  assert(!matchesAuthor(video('tiktok', '123'), query));
  assert(!matchesAuthor(video('douyin', '456'), query));
  assert(!matchesAuthor(null, query));
});

test('legacy author strings remain searchable without overriding known identities', () => {
  const query = authorSearch(video('douyin', '123'));
  assert(matchesAuthor({platform:'douyin',video_id:'456',author:'account'}, query));
  assert(!matchesAuthor({platform:'douyin',video_id:'456',author:'other-account'}, query));
  assert.equal(authorSearch({platform:'douyin',video_id:'456'}), null);
});

test('Telegram groups messages by channel instead of message ID', () => {
  const query = authorSearch({platform:'telegram', video_id:'100_9'});
  assert(matchesAuthor({platform:'telegram',video_id:'100_10'}, query));
  assert(!matchesAuthor({platform:'telegram',video_id:'101_9'}, query));
});

test('a legacy account does not combine multiple confirmed IDs on its profile', () => {
  const legacy = { video: { platform:'douyin', video_id:'456', author:'account' } };
  const first = { video: video('douyin', '123') };
  const second = { video: video('douyin', '456') };
  const identity = authorSearch(legacy.video);
  assert.deepEqual(matchingAuthorRecords([legacy, first], identity), [legacy, first]);
  assert.deepEqual(matchingAuthorRecords([legacy, first, second], identity), [legacy]);
});

test('nickname-only metadata without an identity is not an actionable author search', () => {
  assert.equal(authorSearch({platform:'youtube',video_id:'123',
    author_profile:{id:null,handle:null,nickname:'只有昵称',kind:'channel'}}), null);
});
test('an older list snapshot cannot restore an author field cleared in the same second', () => {
  const work = (id, revision, age) => ({
    id, revision, created_at: 1,
    video: {platform: 'douyin', author_profile: {id: 'same-author', profile_updated_at: 42, age}},
  });
  const oldSnapshot = [work('a', 1, 28), work('b', 1, 28)];
  const updated = mergeTasks(oldSnapshot, [work('a', 2, null)], new Set());
  assert.deepEqual(updated.map(task => task.video.author_profile.age), [null, null]);
  const refreshed = mergeTasks(updated, oldSnapshot, new Set());
  assert.deepEqual(refreshed.map(task => task.video.author_profile.age), [null, null]);
});
