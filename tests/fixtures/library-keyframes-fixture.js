// UI contract fixture only. Real extraction/PTS and process reaping have separate tests.
export function installKeyframeFixture({ libraryApi, clone, saved }) {
  const rows = new Map(), listeners = new Set(); let operation = null, sequence = 1, mode = 'success', failure;
  let releaseStart, holdRead = false, releaseRead;
  const calls = [];
  const png = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYGAAAAAEAAH2FzhVAAAAAElFTkSuQmCC'), c => c.charCodeAt(0));
  const fileFor = (asset, file) => window.libraryDetailTest.snapshot(asset).files.find(f => f.id === file);
  const binding = file => JSON.stringify([file.path, file.size, file.mtime_ns]);
  const emit = () => listeners.forEach(callback => callback(clone(operation)));
  const fail = method => { if (failure === method) { failure = null; throw 'DATABASE_ERROR'; } };
  const complete = () => {
    if (!operation || operation.state !== 'running') return;
    const current = operation;
    if (mode === 'failure') { Object.assign(current, { state: 'failed', error: 'KEYFRAME_RANGE_UNAVAILABLE', sequence: current.sequence + 1 }); emit(); return; }
    current.points.forEach((point, index) => {
      const id = 'keyframe-' + sequence++;
      rows.set(id, { id, asset_id: current.asset_id, file_id: current.file_id, clip_id: current.clip_id, batch_id: current.id,
        sample_start_ms: current.start, sample_end_ms: current.end, sample_index: index, sample_count: current.total,
        requested_ms: point, actual_ms: Math.ceil(point / 100) * 100, pts: Math.ceil(point / 100) * 100,
        time_base_num: 1, time_base_den: 1000, width: 1, height: 1, black: index === 0, created_at: 1, bytes: png.length, source: current.source });
      current.frame_ids.push(id);
    });
    Object.assign(current, { state: 'completed', completed: current.total, sequence: current.sequence + 1 });
    emit(); saved('keyframes', [current.asset_id]);
  };
  libraryApi.keyframes = async asset => { fail('list'); return [...rows.values()].filter(row => row.asset_id === asset).reverse().map(row => {
    const file = fileFor(asset, row.file_id);
    return { ...clone(row), missing: !file || file.missing, source_changed: !file || file.missing || file.changed || binding(file) !== row.source };
  }); };
  libraryApi.keyframeImage = async (assetId, id) => { calls.push({ method: 'image', assetId, id }); fail('image'); if (rows.get(id)?.asset_id !== assetId) throw 'KEYFRAME_NOT_FOUND'; return png; };
  libraryApi.keyframeOperation = async () => { fail('operation'); const captured = clone(operation); if (holdRead) { holdRead = false; await new Promise(resolve => { releaseRead = resolve; }); } return captured; };
  libraryApi.subscribeKeyframes = async callback => { listeners.add(callback); return () => listeners.delete(callback); };
  libraryApi.startKeyframes = async (assetId, request) => {
    fail('start'); if (operation?.state === 'running') throw 'KEYFRAME_BUSY';
    calls.push({ method: 'start', assetId, request: clone(request) });
    const clip = request.kind === 'clip' ? window.libraryDetailTest.clips().find(c => c.id === request.clip_id && c.asset_id === assetId) : null;
    const fileId = clip?.file_id ?? request.file_id, file = fileFor(assetId, fileId);
    if (!file || file.missing) throw 'FILE_MISSING'; if (file.changed) throw 'KEYFRAME_SOURCE_CHANGED';
    const start = clip?.start_ms ?? request.start_ms ?? request.position_ms, end = clip?.end_ms ?? request.end_ms ?? null;
    const points = end != null ? Array.from({ length: request.count }, (_, i) => start + Math.floor((end - start) * (2 * i + 1) / (2 * request.count))) : [start];
    operation = { id: 'frames-run-' + sequence++, sequence: 1, asset_id: assetId, file_id: fileId, clip_id: clip?.id ?? null,
      state: 'running', completed: 0, total: points.length, canceling: false, error: null, frame_ids: [], points, start, end, source: binding(file) };
    emit();
    if (mode === 'hold-start') await new Promise(resolve => { releaseStart = resolve; });
    if (!['hold', 'hold-start'].includes(mode)) setTimeout(complete, 20);
    return clone(operation);
  };
  libraryApi.cancelKeyframes = async id => { fail('cancel'); calls.push({ method: 'cancel', id }); if (operation?.id !== id) throw 'KEYFRAME_OPERATION_NOT_FOUND'; Object.assign(operation, { state: 'canceled', canceling: false, error: 'CANCELED', sequence: operation.sequence + 1 }); emit(); };
  libraryApi.deleteKeyframe = async (assetId, id) => { fail('delete'); if (rows.get(id)?.asset_id !== assetId) throw 'KEYFRAME_NOT_FOUND'; rows.delete(id); calls.push({ method: 'delete', assetId, id }); saved('keyframes', [assetId]); };
  const deleteClip = libraryApi.deleteClip;
  libraryApi.deleteClip = async (assetId, id) => { await deleteClip(assetId, id); for (const [key, row] of rows) if (row.asset_id === assetId && row.clip_id === id) rows.delete(key); saved('keyframes', [assetId]); };
  window.keyframesTest = { calls, mode: value => { mode = value; }, fail: value => { failure = value; }, complete,
    holdRead: () => { holdRead = true; }, releaseRead: () => { releaseRead?.(); releaseRead = null; },
    snapshot: () => clone([...rows.values()]), operation: () => clone(operation), subscriptions: () => listeners.size,
    releaseStart: () => { releaseStart?.(); releaseStart = null; }, emitOld: () => listeners.forEach(callback => callback({ id: 'obsolete-run', sequence: 99, state: 'failed' })) };
}
