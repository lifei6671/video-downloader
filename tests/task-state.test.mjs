import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mergeTasks } from '../src/api.ts';

test('retry updates the original row and ignores late snapshots from the failed attempt', () => {
  const failed = { id: 'original', revision: 10, status: 'failed', created_at: 100, bytes: 7, error: 'NETWORK_ERROR', download_requested: true };
  const other = { id: 'other', revision: 2, status: 'completed', created_at: 200 };
  const retry = { ...failed, revision: 11, status: 'parsing', bytes: 0, error: null };
  const removed = new Set();

  let rows = mergeTasks([other, failed], [retry], removed);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map(row => row.id), ['other', 'original']);
  assert.deepEqual(rows[1], retry);
  rows = mergeTasks(rows, [failed], removed);
  assert.deepEqual(rows[1], retry);

  const completed = { ...retry, revision: 15, status: 'completed', bytes: 40 };
  rows = mergeTasks(rows, [completed], removed);
  rows = mergeTasks(rows, [retry, failed], removed);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[1], completed);
});
