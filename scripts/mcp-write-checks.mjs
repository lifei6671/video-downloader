// WB-310 official SDK client against the real adapter and isolated synthetic Core.
import assert from 'node:assert/strict';

export async function verifyWrites(client, core) {
  const source = { model_name: 'synthetic-model😀', citations: [{ asset_id: 'asset-a', document_id: 'doc-a1', quote: '甲😀' }] };
  const args = { asset_id: 'asset-a', body: '新的 AI 学习笔记😀', source };
  const read = async (name, arguments_ = {}) => (await client.callTool({ name, arguments: arguments_ })).structuredContent;
  async function error(name, arguments_, expected) {
    // rmcp returns tool-domain errors through the JSON-RPC error channel.
    await assert.rejects(client.callTool({ name, arguments: arguments_ }), e => e.message.includes(expected), expected);
  }
  const original = await read('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', offset: 0, length: 100 });
  const human = await read('read_note', { asset_id: 'asset-a', note_id: 'note-a', offset: 0, length: 100 });
  for (const [name, value] of [
    ['create_note', args], ['create_category', { name: '默认不允许', parent_id: null }],
    ['add_asset_categories', { asset_id: 'asset-a', ids: ['cat-c'] }], ['add_asset_tags', { asset_id: 'asset-a', ids: ['tag-c'] }],
  ]) await error(name, value, 'WRITE_DISABLED');
  assert.equal((await core.request('write_permission', { enabled: true })).result.write_enabled, true);
  const note = await read('create_note', args);
  const reread = await read('read_note', { asset_id: 'asset-a', note_id: note.id, offset: 0, length: 100 });
  assert.equal(reread.summary.origin, 'ai');
  assert.equal(reread.citations[0].reference.document_id, 'doc-a1');
  assert.equal(reread.citations[0].quote_excerpt, '甲😀');
  await error('create_category', { name: '不允许子分类', parent_id: 'cat-a' }, 'INVALID_INPUT');
  const category = await read('create_category', { name: '开场学习😀', parent_id: null });
  for (let i = 0; i < 2; i++) {
    await read('add_asset_categories', { asset_id: 'asset-a', ids: [category.id, category.id] });
    await read('add_asset_tags', { asset_id: 'asset-a', ids: ['tag-c', 'tag-c'] });
  }
  let asset = await read('get_asset', { asset_id: 'asset-a' });
  assert.equal(asset.category_ids.filter(id => id === category.id).length, 1);
  assert.equal(asset.tag_ids.filter(id => id === 'tag-c').length, 1);
  await error('add_asset_tags', { asset_id: 'asset-a', ids: ['tag-b', 'missing'] }, 'TAG_NOT_FOUND');
  asset = await read('get_asset', { asset_id: 'asset-a' }); assert(!asset.tag_ids.includes('tag-b'));
  await error('create_note', { ...args, source: { ...source, citations: [{ ...source.citations[0], asset_id: 'asset-b' }] } }, 'INVALID_CITATION');
  for (const name of ['create_topic', 'create_idea']) await assert.rejects(client.callTool({ name, arguments: {} }), { code: -32602 });
  assert.equal((await client.callTool({ name: 'create_note', arguments: { ...args, id: 'note-a' } })).isError, true);
  assert.deepEqual(await read('read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', offset: 0, length: 100 }), original);
  assert.deepEqual(await read('read_note', { asset_id: 'asset-a', note_id: 'note-a', offset: 0, length: 100 }), human);
  for (const name of ['delete_file', 'update_settings', 'edit_note', 'save_document', 'set_mcp_write_enabled']) {
    await assert.rejects(client.callTool({ name, arguments: {} }), { code: -32602 });
  }
  await core.request('write_permission', { enabled: false });
  await error('create_note', args, 'WRITE_DISABLED');
  assert((await read('list_categories')).categories.some(c => c.id === category.id), 'permission errors must not kill the connection');
  return { status: 'PASSED', defaultReadOnly: 4, fourTools: true, provenanceAndQuotes: true, additiveAndAtomic: true, humanAndOriginalUnchanged: true, forbiddenMethods: 5, revokeKeepsReads: true, created: { note: note.id } };
}
