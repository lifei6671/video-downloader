export function installMcpSettingsFixture(mcpApi, options) {
  let writeEnabled = false;
  let enabled = false, installed = !options.has('missing'), failure = null, readFailure = false, serviceError = null, hold = null;
  const command = 'C:\\Program Files\\中文 映取\\yingqu-mcp.exe';
  const endpoint = '\\\\.\\pipe\\yingqu-mcp-v1-0123456789abcdef01234567';
  const calls = [], copies = [];
  const snapshot = () => ({
    write_enabled: enabled && writeEnabled,
    status: { enabled, endpoint: enabled ? endpoint : null, error: serviceError },
    adapter_path: installed ? command : null, adapter_error: installed ? null : 'MCP_ADAPTER_MISSING',
    configs: enabled && installed ? {
      codex: `[mcp_servers.yingqu]\ncommand = ${JSON.stringify(command)}\nargs = ${JSON.stringify(['--endpoint', endpoint])}\n`,
      json: JSON.stringify({ mcpServers: { yingqu: { command, args: ['--endpoint', endpoint] } } }, null, 2),
    } : null,
  });
  const wait = async method => { if (hold?.method === method) { const current = hold; await new Promise(resolve => current.release = resolve); if (hold === current) hold = null; } };
  const fail = () => { if (failure) { const reason = failure; failure = null; throw reason; } };
  mcpApi.connection = async () => { calls.push('read'); const result = snapshot(); await wait('read'); if (readFailure) throw 'IO_ERROR'; return result; };
  mcpApi.setEnabled = async value => { calls.push(value ? 'enable' : 'disable'); await wait('set'); fail(); enabled = value; if (!value) writeEnabled = false; serviceError = null; return snapshot(); };
  mcpApi.setWriteEnabled = async value => { calls.push(value ? 'write-on' : 'write-off'); await wait('write'); fail(); if (!enabled) throw 'MCP_SERVICE_DISABLED'; writeEnabled = value; return snapshot(); };
  mcpApi.copyConfig = async format => { calls.push('copy'); await wait('copy'); fail(); const info = snapshot(); if (!info.configs) throw 'MCP_CONFIG_UNAVAILABLE'; copies.push(info.configs[format]); };
  window.mcpTest = { calls, copies, snapshot, failNext: reason => failure = reason, failRead: value => readFailure = value, installed: value => installed = value, stop: () => { enabled = false; writeEnabled = false; serviceError = 'IPC_SERVICE_FAILED'; }, hold: method => hold = { method }, release: () => hold?.release?.() };
}
