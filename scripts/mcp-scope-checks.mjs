import assert from 'node:assert/strict';

export async function verifyCollectionScope(client, core) {
  const deny = async (name, args, code = 'ASSET_NOT_FOUND') => assert.rejects(client.callTool({ name, arguments: args }), error => error.message.includes(code));
  await core.request('archive_asset', { asset_id: 'asset-a', archived: true });
  for (const [name, args] of [
    ['get_asset', { asset_id: 'asset-a' }], ['read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1' }],
    ['read_note', { note_id: 'note-a' }], ['list_notes', { asset_id: 'asset-a' }],
    ['get_keyframes', { asset_id: 'asset-a' }], ['get_clip', { asset_id: 'asset-a' }],
    ['get_audio_excerpt', { asset_id: 'asset-a', file_id: 'file-a', start_ms: 0, end_ms: 1000 }],
  ]) await deny(name, args);
  await deny('search_assets', { archived: true }, 'INVALID_INPUT');
  const results = (await client.callTool({ name: 'search_assets', arguments: {} })).structuredContent.items;
  assert(!results.some(r => r.asset.id === 'asset-a'));
  for (const uri of ['yingqu://assets/asset-a', 'yingqu://assets/asset-a/documents/doc-a1', 'yingqu://assets/asset-a/notes/note-a']) await assert.rejects(client.readResource({ uri }), error => error.message.includes('ASSET_NOT_FOUND'));
  await core.request('write_permission', { enabled: true });
  for (const [name, args] of [
    ['create_note', { asset_id: 'asset-a', body: 'uncollected', source: { model_name: 'fixture', citations: [] } }],
    ['add_asset_categories', { asset_id: 'asset-a', ids: ['cat-a'] }], ['add_asset_tags', { asset_id: 'asset-a', ids: ['tag-a'] }],
    ['start_transcription', { asset_id: 'asset-a', file_id: 'file-a' }],
    ['start_frame_sampling', { asset_id: 'asset-a', file_id: 'file-a', start_ms: 0, end_ms: 1000 }],
  ]) await deny(name, args);
  await core.request('write_permission', { enabled: false });
  await core.request('archive_asset', { asset_id: 'asset-a', archived: false });
  const invalid = await client.callTool({ name: 'get_asset', arguments: { asset_id: 'asset-a', 'PRIVATE_SECRET_UNKNOWN_FIELD': 'value' } });
  assert.equal(invalid.isError, true); assert.equal(invalid.content[0].text, 'INVALID_INPUT');
  const tools = (await client.listTools()).tools.map(t => t.name);
  for (const name of ['create_topic', 'create_idea']) { assert(!tools.includes(name)); await assert.rejects(client.callTool({ name, arguments: {} }), { code: -32602 }); }
  return { status: 'PASSED', knownIds: true, resources: true, archivedFilterRejected: true, writesAndMedia: true, unknownFieldsSanitized: true, hiddenLegacyTools: true };
}
