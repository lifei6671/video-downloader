// WB-204 checks against the synthetic Core hosted by verify-mcp-adapter.mjs.
import assert from 'node:assert/strict';

export async function verifyLibraryReads(client, core) {
  const text = '甲😀e\u0301👩\u200d💻中文\n尾';
  const values = [];
  const forbidden = new Set(['path', 'source', 'canonical_url', 'mtime_ns', 'source_json', 'cookie', 'proxy', 'credential_ref']);
  function checkPublic(value) {
    if (value && typeof value === 'object') {
      for (const [key, child] of Object.entries(value)) { assert(!forbidden.has(key), `Internal field exposed: ${key}`); checkPublic(child); }
    }
  }
  async function call(name, args = {}) {
    const result = await client.callTool({ name, arguments: args });
    assert.notEqual(result.isError, true, JSON.stringify(result));
    assert(result.structuredContent);
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert(Buffer.byteLength(JSON.stringify(result)) < 1024 * 1024);
    checkPublic(result.structuredContent);
    assert(!JSON.stringify(result).includes('PRIVATE_'));
    values.push({ name, args, result: result.structuredContent });
    return result.structuredContent;
  }
  async function fails(name, args, message) {
    await assert.rejects(client.callTool({ name, arguments: args }), error => error.code === -32602 && error.message.includes(message));
  }
  const page1 = await call('search_assets', { limit: 2 });
  const page2 = await call('search_assets', { limit: 2, cursor: page1.next_cursor });
  assert.equal(new Set([...page1.items, ...page2.items].map(item => item.asset.id)).size, 4);
  assert.equal(page2.next_cursor, null);
  assert.equal((await call('search_assets', { category_ids: ['cat-a'] })).items.length, 2);
  assert.equal((await call('search_assets', { tag_ids: ['tag-a'], query: '学习' })).items.length, 2);
  assert.equal((await call('search_assets', { file_state: 'changed' })).items[0].asset.id, 'asset-c');
  assert.equal((await call('search_assets', { query: '不存在' })).items.length, 0);
  const cats = await call('list_categories', { query: '分类', limit: 1 });
  assert.equal(cats.categories[0].id, 'cat-a'); assert.equal(cats.categories[0].asset_count, 1);
  const cats2 = await call('list_categories', { query: '分类', limit: 1, cursor: cats.next_cursor });
  assert.notEqual(cats2.categories[0].id, cats.categories[0].id);
  const tags = await call('list_tags', { query: '学习', limit: 1 });
  assert.equal(tags.tags[0].asset_count, 2);
  const asset = await call('get_asset', { asset_id: 'asset-a', limit: 1 });
  await assert.rejects(client.callTool({ name: 'get_asset', arguments: { asset_id: 'oversize' } }), error => error.code === -32603 && error.message.includes('RESPONSE_TOO_LARGE'));
  assert.equal(asset.documents.total, 3); assert.equal(asset.documents.items.length, 1);
  const nextAsset = await call('get_asset', { asset_id: 'asset-a', limit: 1, document_cursor: asset.documents.next_cursor });
  assert.notEqual(nextAsset.documents.items[0].id, asset.documents.items[0].id);
  assert.equal((await call('get_asset', { asset_id: 'asset-b' })).files[0].missing, true);
  const latest = await call('read_transcript', { asset_id: 'asset-a' });
  assert.equal(latest.document.id, 'doc-a2'); assert.equal(latest.document.version, 2);
  assert.equal(Array.from(latest.content.text).length, 4000); assert.equal(latest.content.next_offset, 4000);
  const old = await call('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', version: 1, offset: 1, length: 3 });
  assert.equal(old.content.text, '😀e\u0301');
  assert.deepEqual(old.reference, { asset_id: 'asset-a', document_id: 'doc-a1', document_version: 1, kind: 'edited', offset: 1, length: 3 });
  const end = await call('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', offset: Array.from(text).length, length: 1 });
  assert.equal(end.content.text, ''); assert.equal(end.content.eof, true); assert.equal(end.content.next_offset, null);
  const max = await call('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a2', length: 16000 });
  assert.equal(Array.from(max.content.text).length, 16000);
  await call('read_transcript', { asset_id: 'asset-a', document_id: 'doc-rec' });
  const notes = await call('list_notes', { asset_id: 'asset-a', limit: 1 });
  assert.equal(notes.total, 2); assert.equal(notes.notes[0].origin, 'human');
  assert.equal((await call('list_notes', { asset_id: 'asset-a', limit: 1, cursor: notes.next_cursor })).notes[0].origin, 'ai');
  const note = await call('read_note', { note_id: 'note-a', revision: 1, offset: 1, length: 3, citation_limit: 1 });
  assert.equal(note.content.text, '😀e\u0301'); assert.equal(note.reference.revision, 1);
  assert.equal(note.citations[0].reference.document_version, 1); assert.equal(note.citations[0].reference.offset, 1);
  assert.equal((await call('read_note', { note_id: 'note-a', revision: 1, citation_offset: note.next_citation_offset, citation_limit: 1 })).citations[0].reference.asset_id, 'asset-b');
  await core.request('edit_document');
  assert.equal((await call('read_transcript', { asset_id: 'asset-a' })).document.version, 3);
  assert.equal((await call('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1' })).content.text, text);
  await fails('read_transcript', { asset_id: 'asset-a', version: 2 }, 'VERSION_CONFLICT');
  await core.request('edit_note');
  await fails('read_note', { note_id: 'note-a', revision: 1 }, 'REVISION_CONFLICT');
  assert.equal((await call('read_note', { note_id: 'note-a' })).reference.revision, 2);
  assert.equal((await call('search_assets', { query: '第二版' })).items[0].hit.note_revision, 2);

  for (const [name, args, error] of [
    ['get_asset', { asset_id: 'unknown' }, 'ASSET_NOT_FOUND'],
    ['read_transcript', { asset_id: 'asset-a', document_id: 'doc-b1' }, 'DOCUMENT_NOT_FOUND'],
    ['read_transcript', { asset_id: 'empty' }, 'DOCUMENT_NOT_FOUND'],
    ['read_transcript', { asset_id: 'asset-a', length: 16001 }, 'INVALID_INPUT'],
    ['read_transcript', { asset_id: 'asset-a', offset: 99999999 }, 'OUT_OF_RANGE'],
    ['read_note', { note_id: 'unknown' }, 'NOTE_NOT_FOUND'],
    ['read_note', { note_id: 'note-a', asset_id: 'asset-b' }, 'NOTE_NOT_FOUND'],
    ['read_note', { note_id: 'note-a', length: 0 }, 'INVALID_INPUT'],
    ['read_note', { note_id: 'note-a', citation_offset: 99 }, 'OUT_OF_RANGE'],
    ['list_notes', { asset_id: 'asset-a', cursor: 'note-other' }, 'INVALID_CURSOR'],
    ['list_tags', { limit: 0 }, 'INVALID_INPUT'],
    ['list_categories', { cursor: 'missing' }, 'INVALID_CURSOR'],
    ['search_assets', { limit: 101 }, 'INVALID_INPUT'],
    ['search_assets', { query: '中'.repeat(513) }, 'INVALID_INPUT'],
    ['search_assets', { category_ids: ['missing'] }, 'CATEGORY_NOT_FOUND'],
    ['search_assets', { tag_ids: ['missing'] }, 'TAG_NOT_FOUND'],
  ]) await fails(name, args, error);
  await fails('search_assets', { category_ids: Array(100).fill('\0'.repeat(128)) }, 'INVALID_INPUT');
  assert.equal((await call('list_notes', { asset_id: 'empty' })).notes.length, 0);
  for (const name of ['get_settings', 'read_file', 'execute_sql']) {
    await assert.rejects(client.callTool({ name, arguments: {} }), { code: -32602 });
  }
  await fails('start_transcription', { asset_id: 'asset-a', file_id: 'file-a' }, 'WRITE_DISABLED');
  await fails('create_note', { asset_id: 'asset-a', body: '未授权写入', source: { model_name: 'synthetic', citations: [] } }, 'WRITE_DISABLED');
  for (const [name, args] of [['get_asset', { asset_id: 'asset-a', path: 'PRIVATE' }], ['read_note', { note_id: 'note-a', filename: 'PRIVATE' }]]) {
    assert.equal((await client.callTool({ name, arguments: args })).isError, true);
  }
  assert.equal((await call('get_asset', { asset_id: 'asset-a' })).documents.total, 4);
  return { status: 'PASSED', calls: values, scalarSample: text, defaultLength: 4000, maxLength: 16000, preservesOldDocuments: true, revisionConflict: true, internalFieldsOmitted: true };
}
