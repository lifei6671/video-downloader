// WB-203: real adapter + synthetic Core, isolated from all daily profiles.
import assert from 'node:assert/strict';
import {request as httpRequest} from 'node:http';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { verifyLibraryReads } from './mcp-read-checks.mjs';
import { verifyResources } from './mcp-resource-checks.mjs';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

import { verifyOperations } from './mcp-operation-checks.mjs';
import { verifyCollectionScope } from './mcp-scope-checks.mjs';
const modeArgs = process.argv.slice(2);
const codexPosition = modeArgs.indexOf('--codex');
const codexExecutable = codexPosition < 0 ? null : modeArgs.splice(codexPosition, 2)[1];
const allowAccount = modeArgs.includes('--allow-account');
if (allowAccount) modeArgs.splice(modeArgs.indexOf('--allow-account'), 1);
assert(!allowAccount || codexExecutable, '--allow-account requires --codex');
const scopeMode = modeArgs.length === 1 && modeArgs[0] === '--scope';
import { verifyAudiovisual } from './mcp-av-checks.mjs';
const avMode = modeArgs.length === 1 && modeArgs[0] === '--av';
import { verifyMedia } from './mcp-media-checks.mjs';
const mediaMode = modeArgs.length === 1 && modeArgs[0] === '--media';
const operationMode = modeArgs.length === 1 && modeArgs[0] === '--operations';
import { verifyWrites } from './mcp-write-checks.mjs';
const writeMode = modeArgs.length === 1 && modeArgs[0] === '--writes';
const resourceMode = modeArgs.length === 1 && modeArgs[0] === '--resources';
const fullLibrary = scopeMode || avMode || mediaMode || operationMode || writeMode || resourceMode || (modeArgs.length === 1 && modeArgs[0] === '--library');
assert(modeArgs.length === 0 || fullLibrary, 'Usage: node scripts/verify-mcp-adapter.mjs [--library | --resources | --writes | --operations | --media | --av | --scope]');
import { expectedTools as readTools, allTools as expectedTools } from './mcp-client-checks.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const evidenceRoot = join(root, '.local', avMode ? 'materials-av' : mediaMode ? 'wb312' : operationMode ? 'wb311' : writeMode ? 'wb310' : resourceMode ? 'wb205' : fullLibrary ? 'wb204' : 'wb203');
await mkdir(evidenceRoot, { recursive: true });
const output = await mkdtemp(join(evidenceRoot, 'run-'));
const profile = join(output, '中文 MCP adapter');
await mkdir(profile);
await writeFile(join(profile, 'fixture.json'), JSON.stringify({ purpose: 'wb203-synthetic' }));
const fixtureExe = join(root, 'src-tauri', 'target', 'debug', 'examples', process.platform === 'win32' ? 'mcp-ipc-fixture.exe' : 'mcp-ipc-fixture');
const cleanEnv = {
  ...Object.fromEntries(Object.entries(process.env)
    .filter(([key]) => ['path', 'systemroot', 'windir', 'temp', 'tmp', 'pathext'].includes(key.toLowerCase()))
    .map(([key, value]) => [process.platform === 'win32' ? key.toUpperCase() : key, value])),
  HOME: profile, USERPROFILE: profile, APPDATA: join(profile, 'appdata'), LOCALAPPDATA: join(profile, 'localappdata'),
  CODEX_HOME: join(profile, 'codex-home'), XDG_CONFIG_HOME: join(profile, 'config'),
};
const summary = { status: 'RUNNING', platform: `${process.platform}-${process.arch}`, sdk: 'rmcp 3.8.0', checks: {}, evidence: output };
const children = [];

// Fixed-frame test driver only. rmcp and the official TS client own MCP semantics.
class JsonProcess {
  constructor(label, command, args, fixture = false) {
    this.label = label; this.fixture = fixture; this.wire = []; this.stderr = ''; this.pending = new Map(); this.sequence = 0;
    this.failure = null; this.status = null; this.parseErrors = [];
    this.proc = spawn(command, args, { windowsHide: true, cwd: profile, env: { ...cleanEnv, ...(fixture ? { YINGQU_WB203_FIXTURE: '1', ...(avMode ? { YINGQU_AV_FIXTURE: '1' } : {}), ...(mediaMode ? { YINGQU_WB312_FIXTURE: '1' } : {}), ...(operationMode ? { YINGQU_WB311_FIXTURE: '1' } : {}), ...(fullLibrary ? { YINGQU_WB204_FIXTURE: '1', ...((resourceMode || mediaMode) ? { YINGQU_WB205_FIXTURE: '1' } : {}) } : {}) } : {}) } });
    this.closed = new Promise(resolve => {
      this.proc.once('close', (code, signal) => { this.status = { code, signal }; this.fail(new Error(`Child closed: ${label}`)); resolve(this.status); });
    });
    this.proc.on('error', error => this.fail(error));
    this.proc.stdin.on('error', error => this.fail(error));
    this.proc.stderr.setEncoding('utf8').on('data', data => { this.stderr += data; });
    this.lines = createInterface({ input: this.proc.stdout });
    this.lines.on('line', line => {
      try {
        const message = JSON.parse(line);
        if (!fixture) assert.equal(message.jsonrpc, '2.0', 'Non-protocol stdout');
        this.wire.push({ direction: 'server', message });
        const request = this.pending.get(message.id);
        if (request) { this.pending.delete(message.id); clearTimeout(request.timer); request.resolve(message); }
      } catch (error) { this.parseErrors.push(String(error)); this.fail(error); }
    });
    children.push(this);
  }
  fail(error) {
    this.failure ??= error;
    for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(error); }
    this.pending.clear();
  }
  send(message) {
    if (this.status || this.failure) throw this.failure ?? new Error('Child already closed');
    this.wire.push({ direction: 'client', message });
    this.proc.stdin.write(`${JSON.stringify(message)}\n`);
  }
  request(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`${this.label}: ${method} timeout`)); }, this.fixture && method === 'status' && id === 1 ? 30000 : 8000);
      this.pending.set(id, { resolve, reject, timer });
      try { this.send({ jsonrpc: '2.0', id, method, params }); }
      catch (error) { clearTimeout(timer); this.pending.delete(id); reject(error); }
    });
  }
  async waitExit() {
    let timer;
    try {
      const result = await Promise.race([this.closed, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${this.label}: process failed to exit`)), 4000); })]);
      return result;
    } finally { clearTimeout(timer); }
  }
  async close() {
    if (!this.status) this.proc.stdin.end();
    try { return await this.waitExit(); }
    catch (error) { this.proc.kill(); await this.closed; throw error; }
  }
  async save() {
    await writeFile(join(output, `${this.label}.json`), JSON.stringify({ status: this.status, wire: this.wire }, null, 2));
    await writeFile(join(output, `${this.label}.stderr.log`), this.stderr);
  }
}

const core = new JsonProcess('core', fixtureExe, [profile], true);
let endpoint;
try {
  endpoint = (await core.request('status')).result.endpoint;
  assert(endpoint);
  const transport = new StreamableHTTPClientTransport(new URL(endpoint));
  const wire = []; let stderr = '';
  const start = transport.start.bind(transport); const send = transport.send.bind(transport);
  transport.start = async () => {
    const receive = transport.onmessage;
    transport.onmessage = message => { wire.push({ direction: 'server', message: structuredClone(message) }); receive(message); };
    await start();
  };
  transport.send = async message => { wire.push({ direction: 'client', message: structuredClone(message) }); await send(message); };
  const client = new Client({ name: 'yingqu-wb203-ts-client', version: '0.1.0' });
  try {
    await client.connect(transport, { timeout: 8000 });
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), expectedTools);
    for (const tool of tools.tools) {
      assert.equal(tool.annotations.readOnlyHint, readTools.includes(tool.name) || ['get_operation', 'get_clip', 'get_keyframes', 'get_audio_excerpt', 'get_frame_sampling_status'].includes(tool.name));
      assert.equal(tool.inputSchema.additionalProperties, false);
      assert(tool.outputSchema);
      assert(!/"format":"uint(?:32|64)"/.test(JSON.stringify(tool.outputSchema)), 'Nonstandard integer format in public schema');
    }
    const [categories, tags] = await Promise.all([
      client.callTool({ name: 'list_categories', arguments: {} }), client.callTool({ name: 'list_tags', arguments: {} }),
    ]);
    assert(categories.structuredContent.categories.some(category => category.name === '中文灵感😀'));
    assert.equal(tags.structuredContent.tags[0].name, '制作手法');
    assert.deepEqual(JSON.parse(categories.content[0].text), categories.structuredContent);
    assert((await client.callTool({ name: 'list_categories' })).structuredContent.categories.some(category => category.name === '中文灵感😀'));
    await core.request('rename');
    assert.equal((await client.callTool({ name: 'list_tags', arguments: {} })).structuredContent.tags[0].name, '镜头节奏😀');
    assert.equal((await client.callTool({ name: 'list_tags', arguments: { path: 'not-allowed' } })).isError, true);
    await assert.rejects(client.callTool({ name: 'execute_sql', arguments: {} }), { code: -32602 });
    if (operationMode) {
      const extras = [];
      async function extraClient(delayReply = false) {
        endpoint = (await core.request('status')).result.endpoint;
        const t = new StreamableHTTPClientTransport(new URL(endpoint));
        if (delayReply) {
          const begin = t.start.bind(t);
          t.start = async () => { const receive = t.onmessage; t.onmessage = message => {
            if (message.result?.structuredContent?.operation_id) c.wb311HeldOperation = message.result.structuredContent;
            else receive(message);
          }; await begin(); };
        }
        const c = new Client({ name: 'wb311-disconnect-and-timeout-client', version: '1.0' }); extras.push(c);
        await c.connect(t, { timeout: 8000 }); return c;
      }
      try { summary.checks.operations = await verifyOperations(client, core, extraClient); }
      finally { await Promise.all(extras.map(c => c.close())); }
    }
    if (writeMode) summary.checks.writes = await verifyWrites(client, core);
    if (fullLibrary && !writeMode && !operationMode && !avMode) summary.checks.libraryReads = await verifyLibraryReads(client, core);
    if (avMode) {
      summary.checks.audiovisual = await verifyAudiovisual(client, core, profile, output);
      if (codexExecutable) {
        await core.request('write_permission', { enabled:true });
        const capturePath=join(output,'codex-capture.json'), expectedPath=join(output,'codex-expected.json');
        await writeFile(capturePath,JSON.stringify({status:'PASSED',json:JSON.stringify({mcpServers:{yingqu:{url:endpoint}}}),info:{status:{enabled:true,endpoint}}}));
        await writeFile(expectedPath,JSON.stringify({purpose:'materials-synthetic',...summary.checks.audiovisual}));
        const checked=spawnSync(process.execPath,[join(root,'scripts/verify-mcp-client.mjs'),resolve(codexExecutable),capturePath,expectedPath,'--materials',...(allowAccount?['--allow-account']:[])],{encoding:'utf8',windowsHide:true,timeout:300000});
        await writeFile(join(output,'codex-result.log'),checked.stdout??'');
        await writeFile(join(output,'codex-error.log'),checked.stderr??'');
        summary.checks.codex={status:checked.status===0?'PASSED':'FAILED',exit:checked.status};
        await core.request('write_permission', { enabled:false });
        assert.equal(checked.status,0,'Codex audiovisual check failed; see isolated codex-result.log');
      }
    }
    if (mediaMode) summary.checks.media = await verifyMedia(client, output);
    if (resourceMode) summary.checks.resources = await verifyResources(client, core);
    if (scopeMode) summary.checks.collectionScope = await verifyCollectionScope(client, core);
    summary.checks.officialTypescriptClient = { status: 'PASSED', server: client.getServerVersion(), tools, categories, tags, liveUpdate: true };
  } finally {
    try { await client.close(); }
    finally { await writeFile(join(output, 'typescript.json'), JSON.stringify(wire, null, 2)); await writeFile(join(output, 'typescript.stderr.log'), stderr); }
  }

  let modernSequence = 0;
  const modern = async (method, params) => {
    const response = await fetch(endpoint, { method: 'POST', headers: { 'content-type': 'application/json', accept: 'application/json, text/event-stream', 'mcp-protocol-version':'2026-07-28', 'mcp-method':method }, body: JSON.stringify({jsonrpc:'2.0',id:++modernSequence,method,params}) });
    const body = await response.text();
    assert.equal(response.status, 200, body);
    return JSON.parse(body);
  };
  const meta = { 'io.modelcontextprotocol/protocolVersion': '2026-07-28', 'io.modelcontextprotocol/clientInfo': { name: 'yingqu-http-modern-client', version: '1.0' }, 'io.modelcontextprotocol/clientCapabilities': {} };
  const discovery = await modern('server/discover', { _meta: meta });
  assert(discovery.result.supportedVersions.includes('2026-07-28'));
  assert.deepEqual((await modern('tools/list', { _meta: meta })).result.tools.map(tool => tool.name), expectedTools);
  for (const headers of [{origin:'https://evil.invalid'}, {host:'evil.invalid'}, {origin:'null'}]) {
    const rejected = await new Promise((resolve, reject) => {
      const request = httpRequest(endpoint, {method:'POST',signal:AbortSignal.timeout(5000),headers:{'content-type':'application/json',accept:'application/json, text/event-stream',...headers}}, response => {
        let body=''; response.setEncoding('utf8'); response.on('data', chunk=>body+=chunk); response.on('end',()=>resolve({status:response.statusCode,body}));
      }); request.on('error',reject); request.end('{}');
    });
    assert.equal(rejected.status,403, JSON.stringify({headers,body:rejected.body}));
    assert(!rejected.body.includes(profile));
  }
  assert.equal((await fetch(new URL('/private',endpoint))).status,404);
  summary.checks.httpBoundary = {status:'PASSED', loopbackUrl:endpoint, originHostValidation:true, legacyAndModernProtocol:true};
  await core.request('write_permission', {enabled:true});
  const oldEndpoint = endpoint;
  await core.request('stop');
  await assert.rejects(fetch(oldEndpoint, {signal:AbortSignal.timeout(2000)}));
  endpoint = (await core.request('start')).result.endpoint;
  const fresh = new Client({name:'yingqu-http-restart',version:'1.0'});
  await fresh.connect(new StreamableHTTPClientTransport(new URL(endpoint)));
  try {
    assert((await fresh.callTool({name:'list_tags',arguments:{}})).structuredContent.tags.length);
    await assert.rejects(fresh.callTool({name:'create_category',arguments:{name:'readonly'}}), error => error.message.includes('WRITE_DISABLED'));
  } finally { await fresh.close(); }
  await core.close();
  await assert.rejects(fetch(endpoint, {signal:AbortSignal.timeout(2000)}));
  summary.checks.applicationExit = {status:'PASSED', stopRevokesOldAddress:true, reenableReadOnly:true};
  summary.status = 'PASSED';
} catch (error) {
  summary.status = 'FAILED'; summary.error = String(error); throw error;
} finally {
  try {
    const results = await Promise.allSettled(children.map(child => child.close()));
    const failures = results.filter(result => result.status === 'rejected');
    if (failures.length) { summary.status = 'FAILED'; throw new AggregateError(failures.map(result => result.reason), 'Child cleanup failed'); }
    if (children.some(child => child.parseErrors.length)) {
      summary.status = 'FAILED'; throw new Error('Non-protocol stdout was observed');
    }
  } finally {
    await Promise.all(children.map(child => child.save()));
    await writeFile(join(output, 'summary.json'), JSON.stringify(summary, null, 2));
    process.stdout.write(`${summary.status}: ${output}\n`);
  }
}
