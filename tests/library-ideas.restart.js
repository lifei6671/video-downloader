async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9339');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB309 page missing');
  const ideas = await p.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    if (!(await invoke('get_settings')).download_directory.includes('wb309-final')) throw Error('wrong profile');
    return Promise.all((await invoke('list_library_ideas')).map(i => invoke('get_library_idea', { ideaId: i.id })));
  });
  if (!ideas.some(i => i.idea.revision === 3 && i.references.some(r => r.missing))) throw Error('original writing and references not persisted');
  return { status: 'PASSED', ideas };
}
