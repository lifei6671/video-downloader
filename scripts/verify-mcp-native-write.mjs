// Local marked Tauri acceptance profile only; no account or daily library access.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

assert(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === '--restart'));
const targets = await (await fetch('http://127.0.0.1:9340/json')).json();
const target = targets.find(t => t.type === 'page' && t.url === 'http://127.0.0.1:1420/');
assert(target, 'isolated acceptance WebView missing');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
let sequence = 0; const pending = new Map();
socket.addEventListener('message', event => { const reply = JSON.parse(event.data); const call = pending.get(reply.id); if (call) { pending.delete(reply.id); clearTimeout(call.timer); if (reply.error) call.reject(Error(JSON.stringify(reply.error))); else call.resolve(reply.result); } });
async function invoke(command, args = {}) {
  const expression = `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`;
  const id = ++sequence;
  const result = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`native ${command} timeout`)); }, 15000);
    pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
  });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails)); return result.result.value;
}
const folder = resolve('.local/wb310');
const restart = process.argv[2] === '--restart';
let client;
try {
  const settings = await invoke('get_settings');
  assert(settings.download_directory.includes('wb310-final'), 'refusing a daily profile');
  const initial = await invoke('get_mcp_connection');
  assert.equal(initial.status.enabled, false); assert.equal(initial.write_enabled, false);
  if (restart) {
    const saved = JSON.parse(await readFile(`${folder}/native-write-data.json`, 'utf8'));
    const current = { note: await invoke('get_library_note', { assetId: saved.asset, noteId: saved.ids.note }), topic: (await invoke('get_library_topic', { topicId: saved.ids.topic })).topic, idea: (await invoke('get_library_idea', { ideaId: saved.ids.idea })).idea };
    assert.deepEqual(current, saved.snapshots);
    await writeFile(`${folder}/native-restart.json`, JSON.stringify({ status: 'PASSED', defaultOff: true, provenanceAndContentEqual: true, ids: saved.ids }, null, 2));
  } else {
    const info = await invoke('set_mcp_service_enabled', { enabled: true });
    assert.equal(info.write_enabled, false);
    const config = JSON.parse(info.configs.json).mcpServers.yingqu;
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => ['path', 'systemroot', 'windir', 'temp', 'tmp', 'pathext'].includes(key.toLowerCase())));
    client = new Client({ name: 'wb310-native-synthetic-client', version: '1.0' });
    await client.connect(new StdioClientTransport({ ...config, env, stderr: 'pipe' }));
    const read = async (name, args = {}) => (await client.callTool({ name, arguments: args })).structuredContent;
    const assets = await read('search_assets', { limit: 10 }); const asset = assets.items.find(i => i.asset.video_id === '7300000000000000001').asset.id;
    const detail = await read('get_asset', { asset_id: asset }); const document = detail.documents.items.find(d => d.kind === 'edited') ?? detail.documents.items[0]; assert(document);
    const original = await read('read_transcript', { asset_id: asset, document_id: document.id, version: document.version, offset: 0, length: 100 });
    const source = { model_name: 'wb310-native-synthetic-model', citations: [{ asset_id: asset, document_id: document.id, quote: [...original.content.text].slice(0, 6).join('') }] };
    const human = await invoke('create_library_note', { assetId: asset, input: { kind: 'general', body: 'WB310 人工正文保留', citations: [] } });
    const before = await invoke('get_library_note', { assetId: asset, noteId: human });
    await assert.rejects(read('create_note', { asset_id: asset, body: '默认拒绝', source }), e => e.message.includes('WRITE_DISABLED'));
    await invoke('set_mcp_write_enabled', { enabled: true });
    const note = await read('create_note', { asset_id: asset, body: 'WB310 原生 AI 笔记😀', source });
    const topic = await read('create_topic', { input: { title: 'WB310 原生研究😀', description: '基于固定文案引用' }, source });
    const idea = await read('create_idea', { input: { title: 'WB310 原生草稿😀', viewpoint: '独立观点', body: '独立创作正文', references: [{ kind: 'video', asset_id: asset, resource_id: asset }, { kind: 'topic', asset_id: null, resource_id: topic.id }] }, source });
    const category = await read('create_category', { name: 'WB310 原生分类😀', parent_id: null });
    const tag = await invoke('create_library_tag', { name: 'WB310 原生标签😀' });
    for (let i = 0; i < 2; i++) { await read('add_asset_categories', { asset_id: asset, ids: [category.id] }); await read('add_asset_tags', { asset_id: asset, ids: [tag] }); }
    assert.deepEqual(await invoke('get_library_note', { assetId: asset, noteId: human }), before);
    assert.deepEqual(await read('read_transcript', { asset_id: asset, document_id: document.id, version: document.version, offset: 0, length: 100 }), original);
    const snapshots = { note: await invoke('get_library_note', { assetId: asset, noteId: note.id }), topic: (await invoke('get_library_topic', { topicId: topic.id })).topic, idea: (await invoke('get_library_idea', { ideaId: idea.id })).idea };
    assert.equal(snapshots.note.origin, 'ai'); assert.equal(snapshots.note.model_name, source.model_name); assert.deepEqual(snapshots.note.citations, source.citations);
    assert.deepEqual(snapshots.topic.ai_source, source); assert.deepEqual(snapshots.idea.ai_source, source);
    await invoke('set_mcp_write_enabled', { enabled: false });
    await assert.rejects(read('create_note', { asset_id: asset, body: '撤销后拒绝', source }), e => e.message.includes('WRITE_DISABLED'));
    assert((await read('get_asset', { asset_id: asset })).tag_ids.includes(tag));
    await client.close(); client = null;
    await invoke('set_mcp_service_enabled', { enabled: false });
    assert.equal((await invoke('set_mcp_service_enabled', { enabled: true })).write_enabled, false);
    await invoke('set_mcp_service_enabled', { enabled: false });
    await writeFile(`${folder}/native-write-data.json`, JSON.stringify({ asset, ids: { note: note.id, topic: topic.id, idea: idea.id }, snapshots }, null, 2));
    await writeFile(`${folder}/native-write-result.json`, JSON.stringify({ status: 'PASSED', nativeCoreAndOfficialSdk: true, sixTools: true, actualPermissions: true, sourcesAndHumanProtection: true, revokeAndServiceReset: true }, null, 2));
  }
  process.stdout.write('PASSED: native MCP writing\n');
} finally { if (client) await client.close(); socket.close(); }
