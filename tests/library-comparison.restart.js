async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9337');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB307 native page missing');
  const result = await p.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const settings = await invoke('get_settings');
    if (!settings.download_directory.includes('wb307-final')) throw Error('wrong profile');
    const rows = await invoke('search_library_assets', { request: {} });
    const notes = [];
    for (const row of rows.items) for (const note of await invoke('list_library_notes', { assetId: row.asset.id })) {
      if (note.kind === 'comparison') notes.push(await invoke('get_library_note', { assetId: row.asset.id, noteId: note.id }));
    }
    return notes;
  });
  if (!result.length || !result.some(n => n.revision === 2) || result.some(n => n.comparison.points[0].sources.length !== 2)) throw Error('comparison not persisted');
  return { status: 'PASSED', notes: result };
}
