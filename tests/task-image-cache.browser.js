// Exercise the production hook through the existing isolated image IPC fixture.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.route('**/tests/fixtures/cover-save.html?html-proxy*', async route => {
    const response = await route.fetch();
    const html = (await response.text()).replace(/await import\([^;]+;/, originalImport => `
      state.reads = []; state.revoked = []; state.emit = () => {}; state.removed = () => {};
      const revoke = URL.revokeObjectURL.bind(URL);
      URL.revokeObjectURL = url => { state.revoked.push(url); revoke(url); };
      for (const method of ['cover', 'image']) {
        const read = api[method];
        api[method] = async (id, index) => {
          state.reads.push(method + ':' + id);
          if (state.hold === id) await new Promise(resolve => state.releaseRead = resolve);
          return read(id, index);
        };
      }
      api.subscribe = async (emit, removed) => { state.emit = emit; state.removed = removed; return () => {}; };
      state.update = (id, patch) => { const row = rows.find(row => row.id === id); Object.assign(row, patch); row.revision++; state.emit(structuredClone(row)); };
      api.remove = async id => { rows.splice(rows.findIndex(row => row.id === id), 1); state.removed(id); };
      ${originalImport}
    `);
    await route.fulfill({ response, body: html });
  });
  await page.goto(base + '/tests/fixtures/cover-save.html');
  const tasks = () => page.getByRole('button', { name: '下载任务', exact: true }).click();
  const away = () => page.getByRole('button', { name: '下载作品', exact: true }).click();
  const cover = id => page.locator(`[data-task-id="${id}"] [data-media-cover] img`);
  const readCount = id => page.evaluate(id => window.coverSaveTest.reads.filter(value => value === 'cover:' + id).length, id);
  await tasks();
  await cover('douyin').evaluate(n => n.decode());
  await cover('album').evaluate(n => n.decode());
  const videoURL = await cover('douyin').getAttribute('src');
  const albumURL = await cover('album').getAttribute('src');
  const initialReads = await page.evaluate(() => window.coverSaveTest.reads.filter(value => !value.includes('broken')).length);
  for (let i = 0; i < 3; i++) { await away(); await tasks(); await cover('douyin').evaluate(n => n.decode()); }
  check(await cover('douyin').getAttribute('src') === videoURL, 'page switch recreated video URL');
  check(await cover('album').getAttribute('src') === albumURL, 'page switch recreated album URL');
  check(await page.evaluate(() => window.coverSaveTest.reads.filter(value => !value.includes('broken')).length) === initialReads, 'page switch reread cached image');
  check(await readCount('broken') > 1, 'failed reads were permanently cached');
  const before = await readCount('douyin');
  await page.evaluate(() => window.coverSaveTest.update('douyin', { bytes: 222 }));
  await away(); await tasks();
  check(await readCount('douyin') === before, 'progress revision invalidated cover');
  await page.evaluate(() => { window.coverSaveTest.hold = 'douyin'; window.coverSaveTest.update('douyin', { cached_at: 100 }); });
  await page.waitForFunction(() => !!window.coverSaveTest.releaseRead);
  await away(); await tasks();
  check(await readCount('douyin') === before + 1, 'pending image read duplicated across remount');
  await page.evaluate(() => { window.coverSaveTest.hold = null; window.coverSaveTest.releaseRead(); });
  await cover('douyin').evaluate(n => n.decode());
  const refreshed = await cover('douyin').getAttribute('src');
  check(refreshed !== videoURL, 'source update retained stale image');
  await page.locator('[data-task-id="douyin"]').click({ button: 'right', position: { x: 200, y: 30 } });
  await page.getByRole('menuitem', { name: '从列表移除', exact: true }).click();
  await page.locator('[data-task-id="douyin"]').waitFor({ state: 'detached' });
  await page.waitForFunction(url => window.coverSaveTest.revoked.includes(url), refreshed);
  check(await page.evaluate(url => window.coverSaveTest.revoked.includes(url), videoURL), 'old task cover not released');
  await cover('album').evaluate(n => n.decode());
  check(!await page.evaluate(url => window.coverSaveTest.revoked.includes(url), albumURL), 'removing video revoked another task image');
  await page.unroute('**/tests/fixtures/cover-save.html?html-proxy*');
  return { status: 'PASS', cases: ['video and album reuse across three page switches', 'progress does not invalidate', 'source version refresh', 'in-flight deduplication', 'failed read retry', 'removal revokes old and current URLs after exit without affecting other tasks'] };
}
