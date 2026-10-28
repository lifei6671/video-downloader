// Synthetic workbench summaries share the library fixture's actual mutable documents/categories.
export function installWorkbenchFixture({ api, libraryApi, state, calls, clone, saved, options }) {
  let failure = false;
  const tasks = options.has('empty') ? [] : Array.from({ length: 42 }, (_, i) => ({ id: 'task-' + i, revision: 1, files: [], total_bytes: null, speed_bps: null, path: null, completed_at: null, file_missing: false, queue_order: i, cover_path: null, cover_error: null, total_items: null, completed_items: null, status: i === 41 ? 'queued' : 'completed', video: { title: '下载操作 ' + i, platform: 'youtube', kind: 'video', images: [], formats: [], cover_url: null, author: '测试作者' }, download_requested: true, bytes: 0, created_at: 100 - i, error: null, canceling: false, merging: false }));
  let operations = options.has('empty') ? [] : [
    { id: 'asr', kind: 'transcription', title: '竖屏文件文案', state: 'running', phase: 'recognizing', asset_id: 'a0', file_id: 'portrait', task_id: null, error: null },
    { id: 'task-41', kind: 'download', title: '下载操作 41', state: 'running', phase: 'queued', task_id: 'task-41', asset_id: null, file_id: null, error: null },
    { id: 'failed-asr', kind: 'transcription', title: '失败文案', state: 'failed', phase: null, asset_id: 'a3', file_id: 'f-a3', task_id: null, error: 'IO_ERROR' },
    { id: 'import', kind: 'import', title: '导入视频', state: 'running', phase: null, task_id: null, asset_id: null, file_id: null, error: null },
  ];
  api.tasks = async () => clone(tasks);
  const cancelImport = api.cancelLibraryImport;
  api.cancelLibraryImport = async () => { window.libraryTest.cancelImport?.(); await cancelImport(); };
  libraryApi.workbench = async () => {
    calls.push({ method: 'workbench' }); if (failure) { failure = false; throw 'DATABASE_ERROR'; }
    const active = state.assets.filter(a => !a.archived_at);
    const section = items => ({ items: clone(items.slice(0, 6)), total: items.length });
    const unorganized = active.filter(a => { const detail = window.writingTest.snapshot(a.id); return !detail.documents.some(d => d.summary.kind === 'edited') && !detail.notes.length; });
    return { recent: section((state.visits ?? []).map(id => active.find(a => a.id === id)).filter(Boolean)), uncategorized: section(active.filter(a => !a.category_ids.length)), unorganized: section(unorganized), operations: clone(operations) };
  };
  window.workbenchTest = { fail: () => { failure = true; }, operations: value => { operations = value; saved('operations'); }, taskGone: () => tasks.splice(41, 1), visitFailure: () => { const visit = libraryApi.visit; libraryApi.visit = async id => { libraryApi.visit = visit; throw 'DATABASE_ERROR'; }; } };
}
