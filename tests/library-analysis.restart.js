// Read-only snapshot for before/after idle restart of ONLY the WB306 synthetic profile.
async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9336');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB306 WebView2 not found');
  const result = await p.evaluate(async () => {
    const invoke = (command, args = {}) => window.__TAURI_INTERNALS__.invoke(command, args);
    if (!(await invoke('get_settings')).download_directory.includes('wb306-final')) throw Error('Unmarked profile');
    const operation = await invoke('get_library_keyframe_operation');
    if (operation?.state === 'running') throw Error('Cannot snapshot during extraction');
    const assets = await invoke('search_library_assets', { request: {} });
    const assetId = assets.items.find(row => row.asset.video_id === '7300000000000000001').asset.id;
    const rows = await invoke('list_library_notes', { assetId });
    const notes = [];
    for (const note of rows.filter(note => note.kind.endsWith('_analysis'))) notes.push(await invoke('get_library_note', { assetId, noteId: note.id }));
    if (!notes.some(note => note.origin === 'ai' && note.analysis.human_revised) || !notes.some(note => note.kind === 'technique_analysis')) throw Error('Acceptance data not saved');
    return { status: 'PASSED', assetId, notes };
  });
  await browser.close(); return result;
}
