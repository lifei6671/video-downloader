// Synthetic registered paths; media URLs resolve only to generated test clips.
export function installLibraryDetailFixture({ api, libraryApi, state, calls, clone, saved, asset }) {
  const files = new Map();
  const makeFile = (id, owner, extra = {}) => ({ id, asset_id: owner, path: 'C:\\SyntheticVideos\\' + '中文目录\\'.repeat(60) + id + '.webm', size: 12345, mtime_ns: '1000', duration_ms: 4000, width: 160, height: 90, missing: false, changed: false, ...extra });
  state.assets.forEach(a => files.set(a.id, [makeFile('f-' + a.id, a.id, { missing: a.file_state === 'missing', changed: a.file_state === 'changed' })]));
  files.set('a0', [makeFile('f-a0', 'a0'), makeFile('portrait', 'a0', { path: 'C:\\SyntheticVideos\\竖屏样本.mp4', width: 90, height: 160 })]);
  files.set('a4', [makeFile('f-a4', 'a4', { duration_ms: null, width: null, height: null })]);
  files.set('a5', []);
  let mode = 'valid'; let fail = null; let relinkMode = 'success'; let pendingRelink;
  let pendingDetails = []; let pendingSources = []; let holdDetail = false;
  const documents = id => [{ id: 'd-' + id, asset_id: id, file_id: 'f-' + id, kind: 'recognized', version: 2, source_document_id: null, characters: 200, model_name: null, backend: null, created_at: 1000 }];
  const detail = id => ({ asset: clone(asset(id)), files: clone(files.get(id) ?? []), category_ids: [...asset(id).category_ids], tag_ids: [...asset(id).tag_ids], documents: documents(id), notes: [{ id: 'n-' + id, asset_id: id, kind: 'note', origin: 'human', characters: 8, revision: 1 }] });
  const syncState = id => { const all = files.get(id) ?? []; asset(id).file_state = all.every(f => f.missing) ? 'missing' : all.some(f => f.changed) ? 'changed' : 'available'; };
  const consumeFailure = method => { if (fail === method) { fail = null; throw method === 'open' ? 'PLAYBACK_OPEN_FAILED' : 'DATABASE_ERROR'; } };
  window.libraryDetailTest = {
    calls, snapshot: id => detail(id), mode: value => { mode = value; }, fail: method => { fail = method; }, relinkMode: value => { relinkMode = value; },
    changeFile: (id, fileId, patch, emit = true) => { Object.assign(files.get(id).find(f => f.id === fileId), patch); syncState(id); if (emit) saved('assets', [id]); },
    holdDetails: () => { holdDetail = true; }, releaseDetails: () => { holdDetail = false; pendingDetails.splice(0).forEach(resolve => resolve()); },
    releaseSources: () => pendingSources.splice(0).forEach(resolve => resolve('/tests/fixtures/media/player.webm')),
  };
  libraryApi.asset = async id => { calls.push({ method: 'detail', assetId: id }); consumeFailure('detail'); const result = detail(id); if (holdDetail) await new Promise(resolve => pendingDetails.push(resolve)); return result; };
  libraryApi.playbackUrl = async (assetId, fileId) => {
    calls.push({ method: 'playback', assetId, fileId });
    if (mode === 'missing') { window.libraryDetailTest.changeFile(assetId, fileId, { missing: true }, false); throw 'FILE_MISSING'; }
    if (mode === 'prepare-error') throw 'PLAYBACK_SOURCE_FAILED';
    if (mode === 'hold') return new Promise(resolve => pendingSources.push(resolve));
    if (mode === 'positive-pts') return '/.local/wb302/clock-matrix/mp4-origin.mp4';
    if (mode === 'sync-long') return '/.local/wb305/long.webm';
    if (mode === 'audio') return '/tests/fixtures/media/audio-only.webm';
    if (mode === 'unsupported') return '/tests/fixtures/media/unsupported.webm';
    return fileId === 'portrait' ? '/tests/fixtures/media/player-portrait.mp4' : '/tests/fixtures/media/player.webm';
  };
  const clipRows = new Map(); let clipSequence = 1;
  const fileFor = (assetId, fileId) => files.get(assetId)?.find(file => file.id === fileId);
  const binding = file => JSON.stringify([file.path, file.size, file.mtime_ns]);
  const clipValue = row => { const file = fileFor(row.asset_id, row.file_id); return { ...clone(row), missing: !file || file.missing, source_changed: !file || file.missing || file.changed || binding(file) !== row.source }; };
  libraryApi.clips = async assetId => [...clipRows.values()].filter(row => row.asset_id === assetId).map(clipValue);
  libraryApi.createClip = async (assetId, input) => {
    consumeFailure('createClip'); calls.push({ method: 'createClip', assetId, input: clone(input) });
    const file = fileFor(assetId, input.file_id); if (!file || file.missing) throw 'FILE_MISSING'; if (file.changed) throw 'CLIP_SOURCE_CHANGED';
    const document = input.document_id ? await libraryApi.document(assetId, input.document_id) : null;
    if (document && (!document.text.includes(input.quote) || document.source_changed || document.summary.file_id && document.summary.file_id !== file.id)) throw 'INVALID_CITATION';
    const id = 'clip-' + clipSequence++;
    clipRows.set(id, { ...clone(input), id, asset_id: assetId, source: binding(file), document_version: document?.summary.version ?? null, created_at: 1 }); saved('clips', [assetId]); return id;
  };
  libraryApi.deleteClip = async (assetId, id) => { consumeFailure('deleteClip'); if (clipRows.get(id)?.asset_id !== assetId) throw 'CLIP_NOT_FOUND'; clipRows.delete(id); saved('clips', [assetId]); };
  libraryApi.clipPlayback = async (assetId, id) => {
    const row = clipRows.get(id); if (row?.asset_id !== assetId) throw 'CLIP_NOT_FOUND'; const clip = clipValue(row);
    if (clip.missing) throw 'FILE_MISSING'; if (clip.source_changed) throw 'CLIP_SOURCE_CHANGED';
    calls.push({ method: 'clipPlayback', assetId, clipId: id });
    return { clip, url: await libraryApi.playbackUrl(assetId, clip.file_id) };
  };
  window.libraryDetailTest.seedClip = (assetId, fileId, start, end) => { const id = 'clip-' + clipSequence++; clipRows.set(id, { id, asset_id: assetId, file_id: fileId, title: '范围测试 ' + id, reason: '', start_ms: start, end_ms: end, document_id: null, document_version: null, quote: null, created_at: 1, source: binding(fileFor(assetId, fileId)) }); saved('clips', [assetId]); return id; };
  window.libraryDetailTest.clips = () => clone([...clipRows.values()]);
  libraryApi.open = async (assetId, fileId) => { calls.push({ method: 'open', assetId, fileId }); consumeFailure('open'); };
  libraryApi.reveal = async (assetId, fileId) => { calls.push({ method: 'reveal', assetId, fileId }); consumeFailure('reveal'); };
  api.relinkLibraryFile = async (assetId, fileId) => {
    calls.push({ method: 'relink', assetId, fileId });
    if (relinkMode === 'cancel') return null;
    if (relinkMode === 'failure') throw 'LOCAL_VIDEO_INVALID';
    if (relinkMode === 'hold') await new Promise((resolve, reject) => { pendingRelink = { resolve, reject }; });
    Object.assign(files.get(assetId).find(file => file.id === fileId), { path: 'C:\\SyntheticVideos\\重新关联.mp4', missing: false, changed: false, width: 90, height: 160, mtime_ns: '2000' });
    syncState(assetId); saved('assets', [assetId]); return { asset_id: assetId, file_id: fileId, already_present: false };
  };
  api.cancelLibraryImport = async () => { calls.push({ method: 'cancelRelink' }); pendingRelink?.reject('CANCELED'); pendingRelink = undefined; };
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async text => calls.push({ method: 'copy', text }) } });
  state.assets.forEach((a, index) => { a.created_at = 1793000000 - index; a.updated_at = a.created_at; });
  libraryApi.cover = async id => { if (id !== 'a0') throw 'COVER_UNAVAILABLE'; return Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg=='), c => c.charCodeAt(0)); };
  if (state.assets.some(a => a.id === 'a0')) asset('a0').canonical_url = 'https://example.invalid/video/source-a0';
}
