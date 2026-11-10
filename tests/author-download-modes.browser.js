// Simulated IPC only. No platform or clipboard requests.
async page => {
  page.setDefaultTimeout(7000);
  const fixture = page.url().split('/').slice(0, 3).join('/') + '/tests/fixtures/background-download.html?scenario=douyin';
  const source = 'https://v.douyin.com/fixture/';
  const draft = 'https://www.douyin.com/video/456';
  const input = page.locator('#video-input');
  const full = page.getByRole('radio', {name: '全量归档', exact: true});
  const incremental = page.getByRole('radio', {name: '增量下载', exact: true});
  const confirm = page.getByRole('button', {name: '加入下载队列', exact: true});
  const scanButton = page.getByRole('button', {name: '扫描作品', exact: true});

  async function prepare() {
    await page.goto(fixture);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await page.evaluate(async () => {
      const {api} = await import('/src/api.ts');
      const qa = window.backgroundTest;
      const task = qa.snapshot()[0];
      const list = api.tasks;
      const canonical_url = 'https://www.douyin.com/user/fixture-author';
      const author_id = 'fixture-author';
      let scan = null;
      let queued = null;
      api.tasks = async () => [...await list(), ...(queued ? [structuredClone(queued)] : [])];
      api.resolve = async () => ({author_id, canonical_url, default_mode: 'full', scan: structuredClone(scan)});
      api.startAuthorScan = async (url, mode) => {
        if (url !== canonical_url) throw Error('expected canonical author URL');
        qa.calls.push({type: 'author-scan', mode});
        scan = {
          id: 'author-scan', author_id, author_name: '测试作者', mode, status: 'completed',
          pages: 1, videos: 1, albums: 0, skipped: 0, error: null, batch: null,
          pause_reason: null, remaining: 0, completed: 0, failed: 0,
          needs_baseline: mode === 'incremental', new_posts: 0, retry_posts: 0, already_local: 0
        };
        return structuredClone(scan);
      };
      api.authorScan = async () => {
        if (scan?.batch && queued) qa.calls.push({type: 'author-batch-check'});
        return structuredClone(scan);
      };
      qa.pauseBatch = () => { scan.pause_reason = 'AUTHOR_API_LIMIT'; };
      api.confirmAuthorScan = async (id, _partial, baselineAction) => {
        qa.calls.push({type: 'author-confirm', id, baselineAction});
        await new Promise((resolve, reject) => {
          qa.releaseConfirm = resolve;
          qa.rejectConfirm = () => reject('NETWORK_ERROR');
        });
        const baseline_only = baselineAction === 'baseline_only';
        scan.batch = {queued: baseline_only ? 0 : 1, skipped: 0, baseline_only};
        if (!baseline_only) {
          scan.remaining = 1;
          queued = {...task, id: 'batch-work', status: 'queued', download_requested: true,
            video: {...task.video, author_scan_id: id, title: '已入队的作者作品'}};
        }
        return structuredClone(scan.batch);
      };
      api.resumeAuthorBatch = async id => {
        qa.calls.push({type: 'author-resume', id});
        scan.pause_reason = null;
        return structuredClone(scan);
      };
    });
    await input.fill(source);
    await page.getByRole('button', {name: /^(重新解析|解析链接)$/}).click();
    await full.waitFor();
    if (await page.evaluate(() => window.backgroundTest.calls.some(call => call.type === 'author-scan'))) {
      throw Error('resolving the author started a scan');
    }
  }

  async function waitForConfirm() {
    await page.waitForFunction(() => typeof window.backgroundTest.releaseConfirm === 'function');
  }

  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({width, height});
    await prepare();
    await scanButton.click();
    await confirm.waitFor();
    await full.focus();
    await full.press('ArrowRight');
    await incremental.press('Space');
    await page.getByText(/全量归档，已扫描到末页/).waitFor();
    await confirm.waitFor({state: 'hidden'});
    await full.click();
    await confirm.click();
    await waitForConfirm();
    if (width === 1024) await input.fill(draft);
    await page.evaluate(() => {
      window.backgroundTest.rejectConfirm();
      delete window.backgroundTest.releaseConfirm;
    });
    if (width === 1024) {
      await page.getByText('操作失败', {exact: true}).waitFor();
      if (await input.inputValue() !== draft) throw Error('failed confirmation erased the new draft');
      await input.fill(source);
    } else {
      await page.getByRole('alert').filter({hasText: /网络/}).waitFor();
      if (await input.inputValue() !== source) throw Error('failed confirmation cleared the submitted link');
    }
    await confirm.click();
    await waitForConfirm();
    if (width === 1024) await input.fill(draft);
    await page.evaluate(() => window.backgroundTest.releaseConfirm());
    await page.getByText('已加入下载任务', {exact: true}).waitFor();
    await scanButton.waitFor({state: 'hidden'});
    const expected = width === 1024 ? draft : '';
    if (await input.inputValue() !== expected) throw Error('success did not clear only the submitted link');
    if (width !== 1024) await page.waitForFunction(() => document.activeElement?.id === 'video-input');

    await page.getByRole('button', {name: '下载任务', exact: true}).click();
    await page.getByText('已入队的作者作品', {exact: true}).waitFor();
    await page.waitForFunction(() => window.backgroundTest.calls.some(call => call.type === 'author-batch-check'));
    const summary = page.getByText(/待完成 [0-9]+ 条，已完成/);
    if (await summary.count()) throw Error('active author batch displayed a progress strip');
    await page.evaluate(() => window.backgroundTest.pauseBatch());
    const paused = page.getByRole('alert').filter({hasText: '批次已暂停'});
    await paused.waitFor();
    const resume = page.getByRole('button', {name: '恢复批次', exact: true});
    await resume.click();
    await paused.waitFor({state: 'hidden'});
    if (await summary.count()) throw Error('resumed author batch displayed a progress strip');
    const resumed = await page.evaluate(() => window.backgroundTest.calls
      .filter(call => call.type === 'author-resume').map(call => call.id));
    if (resumed.join() !== 'author-scan') throw Error('wrong batch resumed');
    await page.getByRole('button', {name: '下载作品', exact: true}).click();
    await scanButton.waitFor({state: 'hidden'});
    await input.fill(source);
    await page.getByRole('button', {name: '解析链接', exact: true}).click();
    await scanButton.waitFor();
    await page.getByText(/已扫描到末页|已入队 [0-9]+ 条/).waitFor({state: 'hidden'});
  }

  await prepare();
  await incremental.click();
  await scanButton.click();
  await page.getByRole('button', {name: '仅建立基线', exact: true}).click();
  await waitForConfirm();
  await page.evaluate(() => window.backgroundTest.releaseConfirm());
  await page.getByText('已建立基线', {exact: true}).waitFor();
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  await page.getByRole('button', {name: '下载作品', exact: true}).click();
  await page.getByRole('radio', {name: '增量下载', exact: true, checked: true}).waitFor();
  if (await input.inputValue() !== source) throw Error('baseline confirmation cleared the link');
  return {passed: true, viewports: 3, keptDraft: true, resumedBatch: true, keptBaseline: true};
}
