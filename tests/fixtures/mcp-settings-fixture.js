export function installMcpSettingsFixture(mcpApi, options) {
  let writeEnabled = false;
  let enabled = false, failure = null, readFailure = false, serviceError = null, hold = null;
  const endpoint = 'http://127.0.0.1:17653/mcp';
  const calls = [], copies = [];
  const snapshot = () => ({
    write_enabled: enabled && writeEnabled,
    status: { enabled, endpoint: enabled ? endpoint : null, error: serviceError },

  });
  const wait = async method => { if (hold?.method === method) { const current = hold; await new Promise(resolve => current.release = resolve); if (hold === current) hold = null; } };
  const fail = () => { if (failure) { const reason = failure; failure = null; throw reason; } };
  mcpApi.connection = async () => { calls.push('read'); const result = snapshot(); await wait('read'); if (readFailure) throw 'IO_ERROR'; return result; };
  mcpApi.setEnabled = async value => { calls.push(value ? 'enable' : 'disable'); await wait('set'); fail(); enabled = value; if (!value) writeEnabled = false; serviceError = null; return snapshot(); };
  mcpApi.setWriteEnabled = async value => { calls.push(value ? 'write-on' : 'write-off'); await wait('write'); fail(); if (!enabled) throw 'MCP_SERVICE_DISABLED'; writeEnabled = value; return snapshot(); };
  mcpApi.copyUrl = async () => { calls.push('copy'); await wait('copy'); fail(); if (!enabled) throw 'MCP_CONNECTION_UNAVAILABLE'; copies.push(endpoint); };
  window.mcpTest = { calls, copies, snapshot, failNext: reason => failure = reason, failRead: value => readFailure = value, stop: () => { enabled = false; writeEnabled = false; serviceError = 'MCP_HTTP_SERVICE_FAILED'; }, hold: method => hold = { method }, release: () => hold?.release?.() };
}
