async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9338');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB308 page missing');
  const topics = await p.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    if (!(await invoke('get_settings')).download_directory.includes('wb308-final')) throw Error('wrong profile');
    const values = await invoke('list_library_topics');
    return Promise.all(values.map(t => invoke('get_library_topic', { topicId: t.id })));
  });
  if (!topics.length || !topics.some(t => t.members.length === 3 && t.members.some(m => m.missing))) throw Error('topic state not persisted');
  return { status: 'PASSED', topics };
}
