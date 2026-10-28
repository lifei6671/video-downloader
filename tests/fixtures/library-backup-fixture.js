export function installBackupFixture({ libraryApi, state, calls, clone, saved, options }) {
  let operation = null; let order = 0; let mode = 'start'; let empty = options.has('empty');
  const listeners = new Set(); let fail = null; let heldRead = null; let pickerResolve = null;
  const emit = () => listeners.forEach(fn => fn(clone(operation)));
  const info = () => ({ assets: empty ? 0 : 45, documents: empty ? 0 : 47, notes: empty ? 0 : 1, files: empty ? 0 : 46, available_files: empty ? 0 : 44, media_bytes: empty ? 0 : 2147483648, empty, busy: false });
  libraryApi.workbench = async () => ({ recent:{items:[],total:0}, uncategorized:{items:[],total:0}, unorganized:{items:[],total:0}, operations: operation && ['running','failed'].includes(operation.state) ? [{id:operation.id,kind:operation.kind,title:operation.kind==='backup'?'备份知识库':'恢复知识库',state:operation.state,phase:operation.phase,error:operation.error,asset_id:null,file_id:null,task_id:null}] : [] });
  libraryApi.backupInfo = async () => { if (fail === 'info') { fail = null; throw 'DATABASE_ERROR'; } return info(); };
  libraryApi.transfer = async () => { const value = clone(operation); if (heldRead) await new Promise(resolve => heldRead.resolve = resolve); return value; };
  libraryApi.subscribeTransfer = async fn => { listeners.add(fn); return () => listeners.delete(fn); };
  const start = async (kind, includeMedia = false) => {
    calls.push({ method: kind === 'backup' ? 'startBackup' : 'startRestore', includeMedia });
    if (mode === 'hold') return await new Promise(resolve => { pickerResolve = resolve; });
    if (mode === 'cancel') return null;
    if (mode === 'error') throw 'LIBRARY_PACKAGE_INVALID';
    operation = { id: 'transfer-' + ++order, order, sequence: 0, kind, include_media: includeMedia, state: 'running', phase: 'preparing', completed_files: 0, total_files: 0, bytes: 0, canceling: false, path: 'C:/SyntheticBackup/' + '很长的路径/'.repeat(25) + kind, result: null, error: null };
    emit(); return clone(operation);
  };
  libraryApi.startBackup = includeMedia => start('backup', includeMedia);
  libraryApi.startRestore = () => start('restore');
  libraryApi.cancelTransfer = async id => { calls.push({ method: 'cancelTransfer', id }); Object.assign(operation, { sequence: operation.sequence + 1, state: 'canceled', canceling: false }); emit(); };
  window.backupTest = {
    failTransferSubscription: () => { const original=libraryApi.subscribeTransfer; libraryApi.subscribeTransfer=async () => { libraryApi.subscribeTransfer=original; throw 'DATABASE_ERROR'; }; },
    releasePicker: () => { pickerResolve?.(null); pickerResolve=null; },
    mode: value => { mode = value; }, empty: value => { empty = value; saved(); }, failInfo: () => { fail = 'info'; }, listeners: () => listeners.size,
    progress: phase => { Object.assign(operation, { sequence: operation.sequence + 1, phase, completed_files: 2, total_files: 5, bytes: 1234567 }); emit(); },
    finish: (error = null) => { Object.assign(operation, { sequence: operation.sequence + 1, state: error ? 'failed' : 'completed', error, result: error ? null : { assets: 3, files: 2, documents: 4, notes: 1, missing_media: 1, bytes: 1234567 } }); if (!error && operation.kind === 'restore') { empty = false; state.assets.push({id:'restored-asset',title:'恢复后的中文资料',created_at:500,updated_at:500,archived_at:null,category_ids:[],tag_ids:[],file_state:'missing'}); saved(); } emit(); },
    emitOld: value => listeners.forEach(fn => fn(clone(value))), snapshot: () => clone(operation), holdRead: () => { heldRead = {}; }, releaseRead: () => { const current = heldRead; heldRead = null; current?.resolve?.(); },
  };
}
