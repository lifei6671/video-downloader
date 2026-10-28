// WB-201 only. Uses synthetic strings and an isolated Codex home; never a model turn.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { DEFAULT_INHERITED_ENV_VARS, StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const codexFlag = args.indexOf('--codex');
assert(args.length === 0 || (codexFlag === 0 && args.length === 2), 'Usage: node scripts/verify-mcp-sdk.mjs [--codex /path/to/native/codex]');
const evidenceRoot = join(root, '.local', 'wb201');
await mkdir(evidenceRoot, { recursive: true });
const output = await mkdtemp(join(evidenceRoot, 'run-'));
const fixture = join(output, '中文 MCP probe');
await mkdir(fixture);
const executable = join(fixture, process.platform === 'win32' ? 'probe.exe' : 'probe');
await copyFile(join(root, 'tests', 'mcp-sdk-probe', 'target', 'debug', process.platform === 'win32' ? 'yingqu-mcp-sdk-probe.exe' : 'yingqu-mcp-sdk-probe'), executable);
const sdk = JSON.parse(await readFile(join(root, 'node_modules', '@modelcontextprotocol', 'sdk', 'package.json'), 'utf8'));
const summary = { platform: `${process.platform}-${process.arch}`, rustSdk: '3.8.0', typescriptSdk: sdk.version, checks: {}, codex: { status: 'NOT_RUN' }, evidence: output };
const sample = '中文灵感😀 100%_\n第二段';

// Explicitly omit API keys, proxies and user profile directories from test children.
// The TS SDK merges defaults before env, so explicitly overwrite every default.
const cleanEnv = {
  ...Object.fromEntries(DEFAULT_INHERITED_ENV_VARS.map(key => [key, ''])),
  ...Object.fromEntries(Object.entries(process.env)
    .filter(([key]) => ['path', 'systemroot', 'windir', 'temp', 'tmp', 'pathext'].includes(key.toLowerCase()))
    .map(([key, value]) => [process.platform === 'win32' ? key.toUpperCase() : key, value])),
  HOME: fixture, USERPROFILE: fixture, APPDATA: join(fixture, 'appdata'), LOCALAPPDATA: join(fixture, 'localappdata'),
  CODEX_HOME: join(fixture, 'codex-home'), XDG_CONFIG_HOME: join(fixture, 'config'),
};

// A bounded JSONL test driver, not an MCP implementation. The server is entirely rmcp.
class JsonLines {
  constructor(command, commandArgs, options = {}) {
    this.proc = spawn(command, commandArgs, { windowsHide: true, env: cleanEnv, cwd: fixture, ...options });
    this.closed = once(this.proc, 'close');
    this.pending = new Map();
    this.wire = []; this.stderr = ''; this.sequence = 0; this.failure = null; this.closing = false;
    this.proc.stderr.setEncoding('utf8').on('data', data => { this.stderr += data; });
    this.lines = createInterface({ input: this.proc.stdout });
    this.lines.on('line', line => {
      try {
        const message = JSON.parse(line);
        this.wire.push({ direction: 'server', message });
        const waiting = this.pending.get(message.id);
        if (waiting) { this.pending.delete(message.id); clearTimeout(waiting.timer); waiting.resolve(message); }
      } catch (error) { this.fail(error); }
    });
    this.proc.on('error', error => this.fail(error));
    this.proc.stdin.on('error', error => this.fail(error));
    this.proc.on('close', () => { if (!this.closing) this.fail(new Error('Child closed before response')); });
    // The close rejection is handled by close(), including process spawn failure.
    this.closed.catch(() => {});
  }
  fail(error) {
    this.failure ??= error;
    for (const waiting of this.pending.values()) { clearTimeout(waiting.timer); waiting.reject(error); }
    this.pending.clear();
  }
  send(message) {
    if (this.failure) throw this.failure;
    this.wire.push({ direction: 'client', message });
    this.proc.stdin.write(JSON.stringify(message) + '\n');
  }
  request(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`Timeout: ${method}`)); }, 20000);
      this.pending.set(id, { resolve, reject, timer });
      try { this.send({ jsonrpc: '2.0', id, method, params }); }
      catch (error) { this.pending.delete(id); clearTimeout(timer); reject(error); }
    });
  }
  async close() {
    this.closing = true;
    this.proc.stdin.end();
    const timer = setTimeout(() => this.proc.kill(), 4000);
    try { const [code, signal] = await this.closed; assert.equal(signal, null, 'Child required forced termination'); assert.equal(code, 0, this.stderr); if (this.failure) throw this.failure; }
    finally { clearTimeout(timer); this.lines.close(); }
  }
  async save(name) {
    await writeFile(join(output, `${name}.json`), JSON.stringify(this.wire, null, 2));
    await writeFile(join(output, `${name}.stderr.log`), this.stderr);
  }
}

try {
  // Interoperability with a different official SDK, using the legacy initialize flow.
  const transport = new StdioClientTransport({ command: executable, env: cleanEnv, stderr: 'pipe', cwd: fixture });
  let legacyStderr = '';
  const legacyWire = [];
  const start = transport.start.bind(transport);
  const send = transport.send.bind(transport);
  transport.start = async () => {
    const receive = transport.onmessage;
    transport.onmessage = message => { legacyWire.push({ direction: 'server', message: structuredClone(message) }); receive(message); };
    await start();
  };
  transport.send = async message => { legacyWire.push({ direction: 'client', message: structuredClone(message) }); await send(message); };
  transport.stderr.on('data', data => { legacyStderr += data.toString(); });
  const client = new Client({ name: 'yingqu-wb201-ts-client', version: '0.1.0' });
  try {
    await client.connect(transport, { timeout: 10000 });
    const initialized = legacyWire.find(item => item.direction === 'server' && item.message.result?.protocolVersion)?.message.result;
    assert.equal(initialized.protocolVersion, '2025-11-25');
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), ['probe_echo']);
    const tool = tools.tools[0];
    assert.equal(tool.annotations.readOnlyHint, true);
    assert.equal(tool.annotations.openWorldHint, false);
    assert.equal(tool.inputSchema.additionalProperties, false);
    assert(tool.outputSchema);
    const result = await client.callTool({ name: 'probe_echo', arguments: { text: sample } });
    assert.deepEqual(result.structuredContent, { text: sample });
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    const invalid = await client.callTool({ name: 'probe_echo', arguments: { text: 42 } });
    assert.equal(invalid.isError, true);
    await assert.rejects(client.callTool({ name: 'not_a_tool', arguments: {} }), { code: -32602 });
    summary.checks.officialTypescriptClient = { status: 'PASSED', protocolVersion: initialized.protocolVersion, server: client.getServerVersion(), tools, result, invalid };
  } finally {
    try { await client.close(); }
    finally { await writeFile(join(output, 'typescript.stderr.log'), legacyStderr); await writeFile(join(output, 'typescript.json'), JSON.stringify(legacyWire, null, 2)); }
  }

  const modern = new JsonLines(executable, []);
  const meta = { 'io.modelcontextprotocol/protocolVersion': '2026-07-28', 'io.modelcontextprotocol/clientInfo': { name: 'yingqu-wb201-wire-probe', version: '0.1.0' }, 'io.modelcontextprotocol/clientCapabilities': {} };
  try {
    const discovery = await modern.request('server/discover', { _meta: meta });
    assert(discovery.result, JSON.stringify(discovery));
    assert(discovery.result.supportedVersions.includes('2026-07-28'));
    assert(discovery.result.supportedVersions.includes('2025-11-25'));
    const tools = await modern.request('tools/list', { _meta: meta });
    assert.deepEqual(tools.result.tools.map(tool => tool.name), ['probe_echo']);
    const result = await modern.request('tools/call', { _meta: meta, name: 'probe_echo', arguments: { text: sample } });
    assert.deepEqual(result.result.structuredContent, { text: sample });
    const unsupported = await modern.request('tools/list', { _meta: { ...meta, 'io.modelcontextprotocol/protocolVersion': '1900-01-01' } });
    assert.equal(unsupported.error.code, -32022);
    const missing = await modern.request('tools/list', { _meta: { 'io.modelcontextprotocol/protocolVersion': '2026-07-28' } });
    assert.equal(missing.error.code, -32602);
    summary.checks.modernDiscovery = { status: 'PASSED', supportedVersions: discovery.result.supportedVersions, noInitialize: true };
  } finally { try { await modern.close(); } finally { await modern.save('modern'); } }
  assert(modern.wire.filter(item => item.direction === 'server').every(item => item.message.jsonrpc === '2.0'), 'Non-protocol JSON on server stdout');
  summary.checks.modernDiscovery.eofExit = 'PASSED';

  if (codexFlag >= 0) {
    const codex = resolve(args[1]);
    const home = join(output, 'codex-home');
    await mkdir(home);
    const version = spawnSync(codex, ['--version'], { encoding: 'utf8', windowsHide: true, env: cleanEnv, timeout: 10000 });
    assert.equal(version.status, 0, version.stderr);
    const config = `[mcp_servers.wb201_probe]\ncommand = ${JSON.stringify(executable)}\nstartup_timeout_sec = 10\n\n[analytics]\nenabled = false\n\n[feedback]\nenabled = false\n`;
    await writeFile(join(home, 'config.toml'), config);
    const isolatedEnv = { ...cleanEnv, CODEX_HOME: home, HOME: home, USERPROFILE: home, APPDATA: join(home, 'appdata'), LOCALAPPDATA: join(home, 'localappdata'), XDG_CONFIG_HOME: join(home, 'config') };
    const app = new JsonLines(codex, ['app-server', '--listen', 'stdio://'], { env: isolatedEnv, cwd: home });
    try {
      const initialized = await app.request('initialize', { clientInfo: { name: 'yingqu_wb201_probe', version: '0.1.0' } });
      assert(initialized.result, JSON.stringify(initialized));
      app.send({ method: 'initialized', params: {} });
      const inventory = await app.request('mcpServerStatus/list', { serverName: 'wb201_probe', detail: 'toolsAndAuthOnly' });
      assert(inventory.result, JSON.stringify(inventory));
      const server = inventory.result.data.find(server => server.name === 'wb201_probe');
      assert(server?.tools.probe_echo, JSON.stringify(inventory));
      assert.equal(server.tools.probe_echo.annotations.readOnlyHint, true);
      summary.codex = { status: 'PASSED', version: version.stdout.trim(), inventory: server, modelTurn: 'NOT_RUN', isolatedHome: true };
    } finally { try { await app.close(); } finally { await app.save('codex'); } }
  }
  summary.status = 'PASSED';
} catch (error) {
  summary.status = 'FAILED'; summary.error = error.stack; process.exitCode = 1;
} finally {
  await writeFile(join(output, 'summary.json'), JSON.stringify(summary, null, 2));
  console.log(JSON.stringify({ status: summary.status, checks: Object.keys(summary.checks), codex: summary.codex.status, evidence: output, error: summary.error }, null, 2));
}
