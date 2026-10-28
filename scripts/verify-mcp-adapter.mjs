// WB-203: real adapter + synthetic Core, isolated from all daily profiles.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { verifyLibraryReads } from './mcp-read-checks.mjs';
import { verifyResources } from './mcp-resource-checks.mjs';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { DEFAULT_INHERITED_ENV_VARS, StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

import { verifyOperations } from './mcp-operation-checks.mjs';
const operationMode = process.argv.length === 3 && process.argv[2] === '--operations';
import { verifyWrites } from './mcp-write-checks.mjs';
const writeMode = process.argv.length === 3 && process.argv[2] === '--writes';
const resourceMode = process.argv.length === 3 && process.argv[2] === '--resources';
const fullLibrary = operationMode || writeMode || resourceMode || (process.argv.length === 3 && process.argv[2] === '--library');
assert(process.argv.length === 2 || fullLibrary, 'Usage: node scripts/verify-mcp-adapter.mjs [--library | --resources | --writes | --operations]');
import { expectedTools as readTools, allTools as expectedTools } from './mcp-client-checks.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const evidenceRoot = join(root, '.local', operationMode ? 'wb311' : writeMode ? 'wb310' : resourceMode ? 'wb205' : fullLibrary ? 'wb204' : 'wb203');
await mkdir(evidenceRoot, { recursive: true });
const output = await mkdtemp(join(evidenceRoot, 'run-'));
const profile = join(output, '中文 MCP adapter');
await mkdir(profile);
await writeFile(join(profile, 'fixture.json'), JSON.stringify({ purpose: 'wb203-synthetic' }));
const executable = join(profile, process.platform === 'win32' ? 'yingqu-mcp.exe' : 'yingqu-mcp');
await copyFile(join(root, 'src-tauri', 'target', 'debug', process.platform === 'win32' ? 'yingqu-mcp.exe' : 'yingqu-mcp'), executable);
const fixtureExe = join(root, 'src-tauri', 'target', 'debug', 'examples', process.platform === 'win32' ? 'mcp-ipc-fixture.exe' : 'mcp-ipc-fixture');
const cleanEnv = {
  ...Object.fromEntries(DEFAULT_INHERITED_ENV_VARS.map(key => [key, ''])),
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
    this.proc = spawn(command, args, { windowsHide: true, cwd: profile, env: { ...cleanEnv, ...(fixture ? { YINGQU_WB203_FIXTURE: '1', ...(operationMode ? { YINGQU_WB311_FIXTURE: '1' } : {}), ...(fullLibrary ? { YINGQU_WB204_FIXTURE: '1', ...(resourceMode ? { YINGQU_WB205_FIXTURE: '1' } : {}) } : {}) } : {}) } });
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

async function legacyInitialize(child) {
  const initialized = await child.request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'wb203-wire-client', version: '0.1.0' } });
  assert.equal(initialized.result.protocolVersion, '2025-11-25');
  child.send({ jsonrpc: '2.0', method: 'notifications/initialized' });
}
const core = new JsonProcess('core', fixtureExe, [profile], true);
let endpoint;
try {
  endpoint = (await core.request('status')).result.endpoint;
  assert(endpoint);
  const transport = new StdioClientTransport({ command: executable, args: ['--endpoint', endpoint], env: cleanEnv, stderr: 'pipe', cwd: profile });
  const wire = []; let stderr = '';
  const start = transport.start.bind(transport); const send = transport.send.bind(transport);
  transport.start = async () => {
    const receive = transport.onmessage;
    transport.onmessage = message => { wire.push({ direction: 'server', message: structuredClone(message) }); receive(message); };
    await start();
  };
  transport.send = async message => { wire.push({ direction: 'client', message: structuredClone(message) }); await send(message); };
  transport.stderr.on('data', data => { stderr += data.toString(); });
  const client = new Client({ name: 'yingqu-wb203-ts-client', version: '0.1.0' });
  try {
    await client.connect(transport, { timeout: 8000 });
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), expectedTools);
    for (const tool of tools.tools) {
      assert.equal(tool.annotations.readOnlyHint, readTools.includes(tool.name) || tool.name === 'get_operation');
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
        const t = new StdioClientTransport({ command: executable, args: ['--endpoint', endpoint], env: cleanEnv, stderr: 'pipe', cwd: profile });
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
    if (fullLibrary && !writeMode && !operationMode) summary.checks.libraryReads = await verifyLibraryReads(client, core);
    if (resourceMode) summary.checks.resources = await verifyResources(client, core);
    summary.checks.officialTypescriptClient = { status: 'PASSED', server: client.getServerVersion(), tools, categories, tags, liveUpdate: true };
  } finally {
    try { await client.close(); }
    finally { await writeFile(join(output, 'typescript.json'), JSON.stringify(wire, null, 2)); await writeFile(join(output, 'typescript.stderr.log'), stderr); }
  }

  const modern = new JsonProcess('modern', executable, ['--endpoint', endpoint]);
  const meta = { 'io.modelcontextprotocol/protocolVersion': '2026-07-28', 'io.modelcontextprotocol/clientInfo': { name: 'wb203-modern-client', version: '0.1.0' }, 'io.modelcontextprotocol/clientCapabilities': {} };
  const discovery = await modern.request('server/discover', { _meta: meta });
  assert(discovery.result.supportedVersions.includes('2026-07-28'));
  assert.deepEqual((await modern.request('tools/list', { _meta: meta })).result.tools.map(tool => tool.name), expectedTools);
  const result = await modern.request('tools/call', { _meta: meta, name: 'list_tags', arguments: {} });
  assert.equal(result.result.structuredContent.tags[0].name, '镜头节奏😀');
  assert.equal((await modern.request('tools/list', { _meta: { ...meta, 'io.modelcontextprotocol/protocolVersion': '1900-01-01' } })).error.code, -32022);
  assert.equal((await modern.request('tools/list', { _meta: { 'io.modelcontextprotocol/protocolVersion': '2026-07-28' } })).error.code, -32602);
  if (resourceMode) {
    const templates = await modern.request('resources/templates/list', { _meta: meta });
    assert.equal(templates.result.resourceTemplates.length, 3);
    const resource = await modern.request('resources/read', { _meta: meta, uri: 'yingqu://assets/asset-a/documents/doc-a1?offset=1&length=3' });
    assert.equal(JSON.parse(resource.result.contents[0].text).content.text, '😀e\u0301');
    assert.equal(resource.result.ttlMs, 0);
    assert.equal((await modern.request('resources/read', { _meta: meta, uri: 'file:///private' })).error.message, 'INVALID_RESOURCE_URI');
  }
  assert.equal((await modern.close()).code, 0);
  summary.checks.modernHandshake = { status: 'PASSED', protocol: '2026-07-28', eofExit: true };

  const eof = new JsonProcess('legacy-eof', executable, ['--endpoint', endpoint]);
  await legacyInitialize(eof);
  assert((await eof.request('tools/list')).result);
  assert.equal((await eof.close()).code, 0);
  const earlyEof = new JsonProcess('early-eof', executable, ['--endpoint', endpoint]);
  assert.equal((await earlyEof.close()).code, 0);
  assert.equal((await core.request('status')).result.enabled, true);
  summary.checks.clientDisconnect = { status: 'PASSED', beforeAndAfterInitialize: true, coreRemainsRunning: true };

  const idle = new JsonProcess('idle-core-loss', executable, ['--endpoint', endpoint]);
  await legacyInitialize(idle);
  const initializing = new JsonProcess('modern-core-loss', executable, ['--endpoint', endpoint]);
  // Receiving a valid discovery reply proves the child connected to the Core;
  // no legacy initialize is sent on this process.
  assert((await initializing.request('server/discover', { _meta: meta })).result);
  await core.request('stop');
  for (const child of [idle, initializing]) {
    assert.equal(child.proc.stdin.writableEnded, false, 'stdin must stay open while the Core disconnects');
    assert.equal((await child.waitExit()).code, 1);
    assert.equal(child.stderr.trim(), 'IPC_DISCONNECTED');
  }
  const unavailable = new JsonProcess('service-disabled', executable, ['--endpoint', endpoint]);
  assert.equal((await unavailable.waitExit()).code, 1);
  assert.equal(unavailable.stderr.trim(), 'IPC_UNAVAILABLE');
  assert.equal(unavailable.wire.length, 0);
  summary.checks.serviceDisabled = { status: 'PASSED', idleAndDiscoveryDisconnect: true, openStdinExit: true, noAutomaticRestart: true };

  await core.request('start');
  const reconnected = new JsonProcess('fresh-connection', executable, ['--endpoint', endpoint]);
  await legacyInitialize(reconnected);
  assert((await reconnected.request('tools/call', { name: 'list_categories', arguments: {} })).result.structuredContent.categories.some(category => category.name === '中文灵感😀'));
  assert.equal((await reconnected.close()).code, 0);
  const exiting = new JsonProcess('application-exit', executable, ['--endpoint', endpoint]);
  await legacyInitialize(exiting);
  assert.equal((await core.close()).code, 0);
  assert.equal((await exiting.waitExit()).code, 1);
  assert.equal(exiting.stderr.trim(), 'IPC_DISCONNECTED');
  const notRunning = new JsonProcess('application-not-running', executable, ['--endpoint', endpoint]);
  assert.equal((await notRunning.waitExit()).code, 1);
  assert.equal(notRunning.stderr.trim(), 'IPC_UNAVAILABLE');
  summary.checks.applicationExit = { status: 'PASSED', freshAdapterAfterReenable: true, noAppLaunch: true };

  const invalid = new JsonProcess('invalid-endpoint', executable, ['--endpoint', 'http://invalid.example/private-token']);
  assert.equal((await invalid.waitExit()).code, 1);
  assert.equal(invalid.stderr.trim(), 'INVALID_ENDPOINT');
  assert(!invalid.stderr.includes('private-token'));
  const argumentsMissing = new JsonProcess('invalid-args', executable, []);
  assert.equal((await argumentsMissing.waitExit()).code, 2);
  assert.equal(argumentsMissing.wire.length, 0);
  summary.checks.cliAndDiagnostics = { status: 'PASSED', stdoutProtocolOnly: true, staticDiagnostics: true, unicodeExecutablePath: true };
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
