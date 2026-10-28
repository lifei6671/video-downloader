// WB-205: compare Resources with the existing Tools on an isolated Core.
import assert from 'node:assert/strict';

export async function verifyResources(client, core) {
  assert.equal((await core.request('private_probe')).result.configured, true);
  const capabilities = client.getServerCapabilities();
  assert(capabilities.resources);
  assert.notEqual(capabilities.resources.subscribe, true);
  assert.notEqual(capabilities.resources.listChanged, true);
  const listed = await client.listResources();
  assert.deepEqual(listed.resources, []);
  assert.equal(listed.nextCursor, undefined);
  const templates = await client.listResourceTemplates();
  assert.deepEqual(templates.resourceTemplates.map(template => template.name), ['asset', 'document', 'note']);
  for (const template of templates.resourceTemplates) {
    assert.equal(template.mimeType, 'application/json');
    assert(template.uriTemplate.startsWith('yingqu://assets/'));
  }
  for (const method of [() => client.listResources({ cursor: 'private' }), () => client.listResourceTemplates({ cursor: 'private' })]) {
    await assert.rejects(method(), error => error.code === -32602 && error.message.includes('INVALID_RESOURCE_CURSOR'));
  }

  const results = [];
  const forbidden = new Set(['path', 'source', 'canonical_url', 'mtime_ns', 'source_json', 'cookie', 'proxy', 'credential_ref', 'session_directory']);
  function noInternals(value) {
    if (value && typeof value === 'object') {
      for (const [key, child] of Object.entries(value)) { assert(!forbidden.has(key), `Private field exposed: ${key}`); noInternals(child); }
    }
    const serialized = JSON.stringify(value);
    assert(!serialized.includes('PRIVATE_'));
    assert(!serialized.includes('WB205_SYNTHETIC'));
  }
  async function resource(uri) {
    const response = await client.readResource({ uri });
    assert.equal(response.contents.length, 1);
    const content = response.contents[0];
    assert.equal(content.uri, uri);
    assert.equal(content.mimeType, 'application/json');
    assert.equal(content.blob, undefined);
    const value = JSON.parse(content.text);
    noInternals(value);
    results.push({ uri, value });
    return value;
  }
  async function equivalent(uri, tool, args) {
    const fromResource = await resource(uri);
    const fromTool = (await client.callTool({ name: tool, arguments: args })).structuredContent;
    noInternals(fromTool);
    assert.deepEqual(fromResource, fromTool, `${uri} diverged from ${tool}`);
    return fromResource;
  }
  const asset = await equivalent('yingqu://assets/asset-a?limit=1', 'get_asset', { asset_id: 'asset-a', limit: 1 });
  assert.equal(asset.documents.items.length, 1);
  await equivalent(`yingqu://assets/asset-a?limit=1&document_cursor=${encodeURIComponent(asset.documents.next_cursor)}`, 'get_asset', { asset_id: 'asset-a', limit: 1, document_cursor: asset.documents.next_cursor });
  await equivalent('yingqu://assets/asset-b', 'get_asset', { asset_id: 'asset-b' });
  await equivalent('yingqu://assets/archived', 'get_asset', { asset_id: 'archived' });
  await equivalent(`yingqu://assets/${encodeURIComponent('中文 +%😀')}`, 'get_asset', { asset_id: '中文 +%😀' });
  const doc = await equivalent('yingqu://assets/asset-a/documents/doc-a1?version=1&offset=1&length=3', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', version: 1, offset: 1, length: 3 });
  assert.equal(doc.content.text, '😀e\u0301');
  await equivalent(`yingqu://assets/asset-a/documents/doc-a1?offset=${doc.content.total_characters}&length=1`, 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', offset: doc.content.total_characters, length: 1 });
  const max = await equivalent('yingqu://assets/asset-a/documents/doc-a2?length=16000', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a2', length: 16000 });
  assert.equal(Array.from(max.content.text).length, 16000);
  await equivalent('yingqu://assets/asset-a/documents/doc-rec', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-rec' });
  const note = await equivalent('yingqu://assets/asset-a/notes/note-a?revision=2&length=3&citation_limit=1', 'read_note', { asset_id: 'asset-a', note_id: 'note-a', revision: 2, length: 3, citation_limit: 1 });
  assert.equal(note.citations.length, 1);
  await equivalent(`yingqu://assets/asset-a/notes/note-a?revision=2&citation_offset=${note.next_citation_offset}&citation_limit=1`, 'read_note', { asset_id: 'asset-a', note_id: 'note-a', revision: 2, citation_offset: note.next_citation_offset, citation_limit: 1 });

  async function pairedFailure(uri, tool, args, message, resourceCode = -32602) {
    await assert.rejects(client.readResource({ uri }), error => error.code === resourceCode && error.message.includes(message));
    await assert.rejects(client.callTool({ name: tool, arguments: args }), error => error.code === (resourceCode === -32002 ? -32602 : resourceCode) && error.message.includes(message));
  }
  for (const args of [
    ['yingqu://assets/missing', 'get_asset', { asset_id: 'missing' }, 'ASSET_NOT_FOUND', -32002],
    ['yingqu://assets/asset-b/documents/doc-a1', 'read_transcript', { asset_id: 'asset-b', document_id: 'doc-a1' }, 'DOCUMENT_NOT_FOUND', -32002],
    ['yingqu://assets/asset-b/notes/note-a', 'read_note', { asset_id: 'asset-b', note_id: 'note-a' }, 'NOTE_NOT_FOUND', -32002],
    ['yingqu://assets/asset-a/documents/missing', 'read_transcript', { asset_id: 'asset-a', document_id: 'missing' }, 'DOCUMENT_NOT_FOUND', -32002],
    ['yingqu://assets/asset-a/documents/doc-a1?version=2', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', version: 2 }, 'VERSION_CONFLICT'],
    ['yingqu://assets/asset-a/notes/note-a?revision=1', 'read_note', { asset_id: 'asset-a', note_id: 'note-a', revision: 1 }, 'REVISION_CONFLICT'],
    ['yingqu://assets/asset-a?limit=101', 'get_asset', { asset_id: 'asset-a', limit: 101 }, 'INVALID_INPUT'],
    ['yingqu://assets/asset-a?document_cursor=doc-b1', 'get_asset', { asset_id: 'asset-a', document_cursor: 'doc-b1' }, 'INVALID_CURSOR'],
    ['yingqu://assets/asset-a/documents/doc-a1?length=0', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', length: 0 }, 'INVALID_INPUT'],
    ['yingqu://assets/asset-a/documents/doc-a1?length=16001', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', length: 16001 }, 'INVALID_INPUT'],
    ['yingqu://assets/asset-a/documents/doc-a1?offset=99999', 'read_transcript', { asset_id: 'asset-a', document_id: 'doc-a1', offset: 99999 }, 'OUT_OF_RANGE'],
    ['yingqu://assets/asset-a/notes/note-a?citation_limit=101', 'read_note', { asset_id: 'asset-a', note_id: 'note-a', citation_limit: 101 }, 'INVALID_INPUT'],
    ['yingqu://assets/asset-a/notes/note-a?citation_offset=99', 'read_note', { asset_id: 'asset-a', note_id: 'note-a', citation_offset: 99 }, 'OUT_OF_RANGE'],
    ['yingqu://assets/oversize', 'get_asset', { asset_id: 'oversize' }, 'RESPONSE_TOO_LARGE', -32603],
  ]) await pairedFailure(...args);
  for (const uri of [
    'file:///C:/private', 'http://127.0.0.1/private', 'https://example.invalid', 'yingqu://settings/private',
    'yingqu://assets/asset-a/files/file-a', 'yingqu://assets/asset-a/documents/doc-a1/extra',
    'yingqu://assets/asset-a/../asset-b', 'yingqu://assets/asset-a/%2e%2e/asset-b', 'yingqu://assets/asset-a#private',
    'yingqu://assets/asset-a?path=C:/private', 'yingqu://assets/asset-a?limit=1&limit=100',
    'yingqu://assets/asset-a?document_cursor=%FF', 'yingqu://assets/asset-a?document_cursor=%ZZ',
    'yingqu://assets/asset-a/notes/note-a?asset_id=asset-b', 'yingqu://assets/asset-a/documents/doc-a1?length=all',
    'yingqu://assets/%FF', 'yingqu://assets/%ZZ', 'yingqu://assets/%61sset-a',
  ]) {
    await assert.rejects(client.readResource({ uri }), error => error.code === -32602 && error.message.includes('INVALID_RESOURCE_URI') && !error.message.includes('C:/private'));
  }
  await equivalent('yingqu://assets/asset-a', 'get_asset', { asset_id: 'asset-a' });
  return { status: 'PASSED', templates, results, noPrivateFields: true, noCredentialMarkers: true, sameReadScope: true, missingCode: -32002, toolsRemainAvailable: true };
}
