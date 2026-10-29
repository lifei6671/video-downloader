// Local marked Tauri acceptance profile only; no account or daily library access.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

assert(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === '--restart'));
let target;
for (let i = 0; i < 100; i++) {
  try { const targets = await (await fetch('http://127.0.0.1:9341/json')).json(); target = targets.find(t => t.type === 'page' && t.url === 'http://127.0.0.1:1420/'); } catch {}
  if (target) break; await new Promise(r => setTimeout(r, 100));
}
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
const folder = resolve('.local/wb311');
const restart = process.argv[2] === '--restart';
let client; let verifiedProfile = false;
try {
  assert((await invoke('get_settings')).download_directory.includes('wb311-final'), 'refusing a daily profile');
  verifiedProfile = true;
  const initial = await invoke('get_mcp_connection');
  assert.equal(initial.status.enabled, false); assert.equal(initial.write_enabled, false);
  const info = await invoke('set_mcp_service_enabled', { enabled: true });
  const url = info.status.endpoint; assert(url);
  client = new Client({ name: 'wb311-native-operations', version: '1.0' });
  await client.connect(new StreamableHTTPClientTransport(new URL(url)));
  const call = async (name, args) => (await client.callTool({ name, arguments: args })).structuredContent;
  if (restart) {
    const before = JSON.parse(await readFile(`${folder}/native-operation-data.json`, 'utf8'));
    const after = await call('get_operation', { operation_id: before.operation_id });
    assert.deepEqual(after, before);
    await assert.rejects(call('cancel_operation', { operation_id: before.operation_id }), e => e.message.includes('WRITE_DISABLED'));
    await writeFile(`${folder}/native-restart.json`, JSON.stringify({ status: 'PASSED', defaultOff: true, exactOperationAfterRestart: true, operation_id: before.operation_id }, null, 2));
  } else {
    const assets = await call('search_assets', { limit: 10 });
    const asset = assets.items.find(i => i.asset.video_id === '7300000000000000001').asset.id;
    const detail = await call('get_asset', { asset_id: asset }); const file = detail.files.find(f => !f.missing); assert(file);
    const document = detail.documents.items[0];
    const original = await call('read_transcript', { asset_id: asset, document_id: document.id, version: document.version, offset: 0, length: 100 });
    const args = { asset_id: asset, file_id: file.id, force: true };
    await assert.rejects(call('start_transcription', args), e => e.message.includes('WRITE_DISABLED'));
    await invoke('set_mcp_write_enabled', { enabled: true });
    const running = await call('start_transcription', args); assert.equal(running.state, 'running');
    const pendingCancel = await call('cancel_operation', { operation_id: running.operation_id });
    if (pendingCancel.state === 'running') assert.equal(pendingCancel.cancel_requested, true);
    let result;
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
      result = await call('get_operation', { operation_id: running.operation_id });
      if (result.state !== 'running') break;
      await new Promise(r => setTimeout(r, 20));
    }
    assert(['canceled', 'failed', 'completed', 'no_speech'].includes(result.state), JSON.stringify(result));
    const desktop = await invoke('get_asset_transcript', { assetId: asset, fileId: file.id });
    assert.equal(desktop.operation_id, result.operation_id); assert.equal(desktop.state, result.state);
    assert.deepEqual(await call('read_transcript', { asset_id: asset, document_id: document.id, version: document.version, offset: 0, length: 100 }), original);
    await invoke('set_mcp_write_enabled', { enabled: false });
    assert.deepEqual(await call('get_operation', { operation_id: running.operation_id }), result);
    await assert.rejects(call('cancel_operation', { operation_id: running.operation_id }), e => e.message.includes('WRITE_DISABLED'));
    await writeFile(`${folder}/native-operation-data.json`, JSON.stringify(result, null, 2));
    await writeFile(`${folder}/native-result.json`, JSON.stringify({ status: 'PASSED', actualTauriAndOfficialSdk: true, permissions: true, startAndCancelAndPoll: true, stateMatchesDesktop: true, originalPreserved: true, terminalState: result.state, error_code: result.error_code, modelQualityEvaluated: false }, null, 2));
  }
  await client.close(); client = null;
  await invoke('set_mcp_service_enabled', { enabled: false });
  process.stdout.write('PASSED: native MCP operations\n');
} finally {
  try { if (client) await client.close(); }
  finally {
    try { if (verifiedProfile) await invoke('set_mcp_service_enabled', { enabled: false }); }
    finally { socket.close(); }
  }
}
