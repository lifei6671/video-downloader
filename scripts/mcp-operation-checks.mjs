import assert from 'node:assert/strict';
import { stat } from 'node:fs/promises';

export async function verifyOperations(client, core, extraClient) {
  const args = { asset_id: 'asset-a', file_id: 'file-a' };
  async function call(name, arguments_, c = client, options) {
    const result = await c.callTool({ name, arguments: arguments_ }, undefined, options);
    assert.notEqual(result.isError, true, JSON.stringify(result));
    const data = result.structuredContent; assert(data);
    assert.deepEqual(JSON.parse(result.content[0].text), data);
    assert(!/PRIVATE|"path"|"source"|"text"|"task_id"|"runtime_directory"/.test(JSON.stringify(data)));
    return data;
  }
  const error = (name, args_, code) => assert.rejects(call(name, args_), e => e.code === -32602 && e.message.includes(code));
  const poll = async (id, terminal = true) => {
    const limit = Date.now() + 12000;
    while (Date.now() < limit) {
      const value = await call('get_operation', { operation_id: id });
      if (terminal ? value.state !== 'running' : value.phase === 'recognizing') return value;
      await new Promise(r => setTimeout(r, 20));
    }
    throw Error('operation polling timed out');
  };
  await error('start_transcription', args, 'WRITE_DISABLED');
  await error('get_operation', { operation_id: 'missing' }, 'OPERATION_NOT_FOUND');
  await core.request('write_permission', { enabled: true });
  const initiating = await extraClient();
  const started = await call('start_transcription', args, initiating); assert.equal(started.state, 'running');
  const id = started.operation_id;
  assert.equal((await call('start_transcription', { ...args, force: true })).operation_id, id);
  await error('start_transcription', { asset_id: 'asset-c', file_id: 'file-c' }, 'TRANSCRIPTION_BUSY');
  await poll(id, false);
  let processes;
  for (let i = 0; i < 100; i++) { processes = (await core.request('transcription_processes')).result; if (processes) break; await new Promise(r => setTimeout(r, 20)); }
  assert(processes, 'actual transcription child marker missing');
  await initiating.close();
  assert.equal((await call('get_operation', { operation_id: id })).state, 'running', 'initiating client disconnect canceled the Core worker');
  await core.request('write_permission', { enabled: false });
  assert.equal((await call('get_operation', { operation_id: id })).state, 'running');
  await error('cancel_operation', { operation_id: id }, 'WRITE_DISABLED');
  await core.request('write_permission', { enabled: true });
  const pending = await call('cancel_operation', { operation_id: id });
  assert.equal(pending.state, 'running'); assert.equal(pending.cancel_requested, true);
  const canceled = await poll(id); assert.equal(canceled.state, 'canceled');
  assert.equal((await call('cancel_operation', { operation_id: id })).state, 'canceled');
  await assert.rejects(stat(processes.stage), { code: 'ENOENT' });
  for (const key of ['engine', 'child']) assert.throws(() => process.kill(processes[key], 0), { code: 'ESRCH' }, `${key} survived canceled`);

  // Hold only the reply to a real, already committed Core call. This makes client
  // timeout deterministic without slowing or changing the product's worker/IPC.
  const delayed = await extraClient(true);
  await assert.rejects(call('start_transcription', { ...args, force: true }, delayed, { timeout: 300 }), e => /timeout|timed out/i.test(e.message));
  const resumed = await call('start_transcription', args); assert.notEqual(resumed.operation_id, id);
  assert.equal(resumed.operation_id, delayed.wb311HeldOperation?.operation_id, 'timeout must follow a proved committed Core reply');
  assert.equal(resumed.state, 'running', 'a timed-out call canceled its Core job');
  await delayed.close();
  await error('get_operation', { operation_id: id }, 'OPERATION_NOT_FOUND');
  await core.request('stop'); await core.request('start');
  const reconnected = await extraClient();
  const read = await call('get_operation', { operation_id: resumed.operation_id }, reconnected);
  assert.equal(read.state, 'running', 'service/client loss canceled background work');
  await assert.rejects(call('cancel_operation', { operation_id: resumed.operation_id }, reconnected), e => e.message.includes('WRITE_DISABLED'));
  await core.request('write_permission', { enabled: true });
  await call('cancel_operation', { operation_id: resumed.operation_id }, reconnected);
  let value;
  for (let i = 0; i < 200; i++) { value = await call('get_operation', { operation_id: resumed.operation_id }, reconnected); if (value.state !== 'running') break; await new Promise(r => setTimeout(r, 20)); }
  assert.equal(value.state, 'canceled');
  const success = await call('start_transcription', { asset_id: 'asset-c', file_id: 'file-c' }, reconnected);
  for (let i = 0; i < 200; i++) { value = await call('get_operation', { operation_id: success.operation_id }, reconnected); if (value.state !== 'running') break; await new Promise(r => setTimeout(r, 20)); }
  assert.equal(value.state, 'completed', JSON.stringify(value)); assert.equal(value.has_result, true);
  assert.equal((await call('start_transcription', { asset_id: 'asset-c', file_id: 'file-c' }, reconnected)).operation_id, success.operation_id);
  await reconnected.close();
  await core.request('write_permission', { enabled: false });
  return { status: 'PASSED', defaultAndRevokedPermissions: true, exactIdsAndSingleWorker: true, initiatingClientDisconnect: true, timeoutAfterCommit: true, serviceReconnectAndPermissionReset: true, pendingIsNotCanceled: true, actualEngineAndChildReaped: true, stageRemoved: true, completionAndSavedResultReuse: true };
}
