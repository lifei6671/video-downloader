import assert from 'node:assert/strict';
import { test } from 'node:test';
import { authorSearch, matchesAuthor } from '../src/lib/author-search.ts';

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

test('nickname-only metadata without an identity is not an actionable author search', () => {
  assert.equal(authorSearch({platform:'youtube',video_id:'123',
    author_profile:{id:null,handle:null,nickname:'只有昵称',kind:'channel'}}), null);
});
