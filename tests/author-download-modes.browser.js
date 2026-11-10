// Simulated IPC only. No platform or clipboard requests.
async page => {
  page.setDefaultTimeout(7000);
  const fixture = page.url().split('/').slice(0, 3).join('/') + '/tests/fixtures/background-download.html';
  const home = page.getByRole('radio', {name: '抖音主页', exact: true});
  const full = page.getByRole('radio', {name: '全量归档', exact: true});
  const incremental = page.getByRole('radio', {name: '增量下载', exact: true});
  const confirm = page.getByRole('button', {name: '加入下载队列', exact: true});
  const input = page.locator('#video-input');

  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({width, height});
    await page.goto(fixture + '?scenario=douyin');
    await page.evaluate(async () => {
      const {api} = await import('/src/api.ts');
      const scan = {
        id: 'full-scan', author_name: '测试作者', mode: 'full', status: 'completed',
        pages: 1, videos: 1, albums: 0, skipped: 0, error: null, batch: null,
        pause_reason: null, remaining: 0, completed: 0, failed: 0,
        needs_baseline: false, new_posts: 0, retry_posts: 0, already_local: 0
      };
      api.authorScan = async () => scan;
      api.confirmAuthorScan = async id => {
        window.backgroundTest.calls.push({type: 'author-confirm', id});
        scan.batch = {queued: 1, skipped: 0, baseline_only: false};
        return scan.batch;
      };
    });
    await home.click();
    await confirm.waitFor();
    await full.focus();
    await full.press('ArrowRight');
    await incremental.press('Space');
    await page.getByText(/全量归档，已扫描到末页/).waitFor();
    await confirm.waitFor({state: 'hidden'});
    await full.click();
    await confirm.click();
    await page.getByText(/已入队 1 条/).waitFor();
    const ids = await page.evaluate(() => window.backgroundTest.calls
      .filter(call => call.type === 'author-confirm').map(call => call.id));
    if (ids.join() !== 'full-scan') throw Error('unexpected author confirmation');
  }

  await page.goto(fixture + '?scenario=parse-controls');
  await page.evaluate(async () => {
    const {api} = await import('/src/api.ts');
    api.authorScan = async () => null;
  });
  await input.fill('https://t.me/c/100/10');
  await page.getByRole('button', {name: '重新解析', exact: true}).click();
  await page.waitForFunction(() => typeof window.backgroundTest.releaseResolve === 'function');
  await page.evaluate(() => window.backgroundTest.releaseResolve());
  await page.locator('[data-slot=resolve-progress]').waitFor();
  await home.click();
  const paste = page.getByRole('button', {name: '粘贴链接', exact: true});
  await paste.click();
  await page.waitForFunction(() => typeof window.backgroundTest.releaseClipboard === 'function');
  await page.evaluate(() => window.backgroundTest.releaseClipboard('https://www.douyin.com/user/fixture'));
  await page.waitForFunction(() => document.querySelector('#video-input').value === 'https://www.douyin.com/user/fixture');
  await page.evaluate(() => { delete window.backgroundTest.releaseClipboard; });
  await paste.click();
  await page.waitForFunction(() => typeof window.backgroundTest.releaseClipboard === 'function');
  await input.fill('https://www.douyin.com/user/typed');
  await page.evaluate(() => window.backgroundTest.releaseClipboard('https://www.douyin.com/user/late'));
  await paste.waitFor();
  if (await input.inputValue() !== 'https://www.douyin.com/user/typed') throw Error('late paste erased author draft');
  await page.evaluate(() => {
    const qa = window.backgroundTest;
    const original = qa.snapshot().find(task => task.id === 'original');
    qa.emit({...original, id: 'previous', status: 'completed', download_requested: true,
      completed_at: 9, path: 'C:\\TestVideos\\previous.mp4'});
    qa.finishParsing('ready');
  });
  const duplicate = page.getByRole('alertdialog', {name: '该规格已下载过', exact: true});
  await page.getByRole('radio', {name: '单个作品', exact: true}).click();
  await duplicate.waitFor();

  await page.goto(fixture + '?scenario=pending');
  await page.evaluate(async () => {
    const {api} = await import('/src/api.ts');
    api.authorScan = async () => null;
  });
  await page.getByRole('button', {name: '后台下载', exact: true}).click();
  await page.waitForFunction(() => window.backgroundTest.calls.some(call => call.type === 'download'));
  await home.click();
  const draft = 'https://www.douyin.com/user/fixture';
  await input.fill(draft);
  await page.evaluate(() => window.backgroundTest.release());
  await page.getByText('已加入下载任务', {exact: true}).waitFor();
  if (await input.inputValue() !== draft) throw Error('background download erased author draft');
  return {passed: true, viewports: 3, isolatedClipboard: true, preservedDraft: true};
}
