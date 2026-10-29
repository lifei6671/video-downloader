// Verify configs copied from the isolated native WB-206 acceptance app. No model turn.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { DEFAULT_INHERITED_ENV_VARS, StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { allTools } from './mcp-client-checks.mjs';

const args = process.argv.slice(2);
const outputFlag = args.indexOf('--output');
const evidenceDirectory = outputFlag >= 0 ? args.splice(outputFlag, 2)[1] : null;
if (outputFlag >= 0) assert(evidenceDirectory, '--output requires a directory');
const minimalPath = args.includes('--minimal-path');
if (minimalPath) args.splice(args.indexOf('--minimal-path'), 1);
assert(args.length === 1 || (args.length === 2 && args[1] === '--unavailable') || (args.length === 3 && args[1] === '--codex'), 'Usage: node scripts/verify-mcp-config.mjs capture.json [--unavailable | --codex native-codex-path] [--output directory] [--minimal-path]');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const capture = JSON.parse((await readFile(resolve(args[0]), 'utf8')).replace(/^\uFEFF/, ''));
assert.equal(capture.status, 'PASSED');
const config = JSON.parse(capture.json).mcpServers.yingqu;
assert.equal(config.command, capture.info.adapter_path);
assert.deepEqual(config.args, ['--endpoint', capture.info.status.endpoint]);
const outputRoot = evidenceDirectory ? resolve(evidenceDirectory) : join(root, '.local', 'wb206');
await mkdir(outputRoot, { recursive: true });
const output = await mkdtemp(join(outputRoot, 'config-'));
const env = {
  ...Object.fromEntries(DEFAULT_INHERITED_ENV_VARS.map(key => [key, ''])),
  ...Object.fromEntries(Object.entries(process.env).filter(([key]) => ['path', 'systemroot', 'windir', 'temp', 'tmp', 'pathext'].includes(key.toLowerCase())).map(([key, value]) => [process.platform === 'win32' ? key.toUpperCase() : key, value])),
  HOME: output, USERPROFILE: output, CODEX_HOME: output, APPDATA: join(output, 'appdata'), LOCALAPPDATA: join(output, 'localappdata'), XDG_CONFIG_HOME: join(output, 'config'),
};
if (minimalPath) env.PATH = process.platform === 'win32' ? join(env.SYSTEMROOT || env.WINDIR || 'C:\\Windows', 'System32') : '/usr/bin:/bin';
const summary = { status: 'RUNNING', output, platform: `${process.platform}-${process.arch}`, checks: {}, modelTurn: 'NOT_RUN' };
const python = join(root, '.venv', process.platform === 'win32' ? 'Scripts/python.exe' : 'bin/python');
try {
  const parsed = spawnSync(python, ['-X', 'utf8', '-c', 'import sys,json,tomllib; print(json.dumps(tomllib.loads(sys.stdin.read())))'], { input: capture.toml, encoding: 'utf8', windowsHide: true, env, timeout: 10000 });
  assert.equal(parsed.status, 0, parsed.stderr);
  assert.deepEqual(JSON.parse(parsed.stdout).mcp_servers.yingqu, config);
  summary.checks.tomlJsonEquivalent = 'PASSED';
  if (args[1] === '--unavailable') {
    const result = spawnSync(config.command, config.args, { input: '', encoding: 'utf8', env, cwd: output, windowsHide: true, timeout: 10000 });
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.stdout, '');
    assert.match(result.stderr, /IPC_UNAVAILABLE/);
    summary.checks.disabledCannotConnect = 'PASSED';
  } else {
    const client = new Client({ name: 'yingqu-wb206-config', version: '0.1.0' });
    const transport = new StdioClientTransport({ ...config, env, cwd: output, stderr: 'pipe' });
    let stderr = ''; transport.stderr?.on('data', data => stderr += data);
    try {
      await client.connect(transport, { timeout: 10000 });
      const inventory = await client.listTools();
      assert.deepEqual(inventory.tools.map(tool => tool.name).sort(), allTools);
      const result = await client.callTool({ name: 'list_categories', arguments: {} });
      assert(!result.isError, JSON.stringify(result));
      summary.checks.jsonLaunchHandshakeRead = 'PASSED';
    } finally { await client.close(); await writeFile(join(output, 'adapter-stderr.log'), stderr); }
    if (args[1] === '--codex') {
      const codex = resolve(args[2]);
      const version = spawnSync(codex, ['--version'], { env, encoding: 'utf8', windowsHide: true, timeout: 10000 });
      assert.equal(version.status, 0, version.stderr);
      await writeFile(join(output, 'config.toml'), capture.toml + '\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n');
      const child = spawn(codex, ['app-server', '--listen', 'stdio://'], { env, cwd: output, windowsHide: true });
      const closed = once(child, 'close'); closed.catch(() => {});
      let sequence = 0, diagnostic = ''; const pending = new Map();
      child.stderr.on('data', data => diagnostic += data);
      const lines = createInterface({ input: child.stdout });
      const fail = error => { for (const item of pending.values()) { clearTimeout(item.timer); item.reject(error); } pending.clear(); };
      child.on('error', fail); child.stdin.on('error', fail);
      lines.on('line', line => { try { const response = JSON.parse(line); const item = pending.get(response.id); if (item) { clearTimeout(item.timer); pending.delete(response.id); item.resolve(response); } } catch (error) { fail(error); } });
      const request = (method, params) => new Promise((resolveRequest, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(Error('Timeout: ' + method)); }, 15000);
        pending.set(id, { resolve: resolveRequest, reject, timer });
        child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
      });
      try {
        const initialized = await request('initialize', { clientInfo: { name: 'yingqu_wb206', version: '0.1.0' } });
        assert(initialized.result, JSON.stringify(initialized));
        child.stdin.write(JSON.stringify({ method: 'initialized', params: {} }) + '\n');
        const inventory = await request('mcpServerStatus/list', { serverName: 'yingqu', detail: 'toolsAndAuthOnly' });
        const server = inventory.result?.data.find(item => item.name === 'yingqu');
        assert(server?.tools, JSON.stringify(inventory));
        assert.deepEqual(Object.keys(server.tools).sort(), allTools);
        summary.checks.codexCopiedTomlDiscovery = 'PASSED';
        summary.codex = version.stdout.trim();
      } finally {
        child.stdin.end(); const timer = setTimeout(() => child.kill(), 4000);
        try { await closed; } finally { clearTimeout(timer); lines.close(); fail(Error('closed')); await writeFile(join(output, 'codex-stderr.log'), diagnostic); }
      }
    }
  }
  summary.status = 'PASSED';
} catch (error) { summary.status = 'FAILED'; summary.error = String(error); process.exitCode = 1; }
await writeFile(join(output, 'summary.json'), JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));
