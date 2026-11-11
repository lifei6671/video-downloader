// Simulated IPC only. No platform or clipboard requests.
async page => {
  page.setDefaultTimeout(7000);
  const fixture = page.url().split('/').slice(0, 3).join('/') + '/tests/fixtures/background-download.html?scenario=douyin';
  const source = 'https://v.douyin.com/fixture/';
  const draft = 'https://www.douyin.com/video/456';
  const input = page.locator('#video-input');
  const full = page.getByRole('button', {name: '全量下载', exact: true});
  const incremental = page.getByRole('button', {name: '增量下载', exact: true});
  const dismiss = page.getByRole('button', {name: '取消', exact: true});
  const scanButton = page.getByRole('button', {name: '扫描作品', exact: true});
  const stop = page.getByRole('button', {name: '停止扫描', exact: true});
  const reset = page.getByRole('button', {name: '重置扫描', exact: true});
  const resumeScan = page.getByRole('button', {name: '继续扫描', exact: true});

  async function prepare(status = 'completed', pending = 1) {
    await page.goto(fixture);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await page.evaluate(({status, pending}) => {
      const qa = window.backgroundTest;
      const {api} = qa;
      const task = qa.snapshot()[0];
      const list = api.tasks;
      const canonical_url = 'https://www.douyin.com/user/fixture-author';
      const author_id = 'fixture-author';
      let scan = null;
      let queued = null;
      api.tasks = async () => [...await list(), ...(queued ? [structuredClone(queued)] : [])];
      api.resolve = async () => {
        qa.calls.push({type: 'author-scan'});
        scan = {
          id: 'author-scan', author_id, author_name: '测试作者', status: 'scanning',
          pages: 1, videos: 1, albums: 1, skipped: 0, error: null, batch: null,
          pause_reason: null, remaining: 0, completed: 0, failed: 0,
          pending_downloads: pending, already_local: 2 - pending, resume_page: null
        };
        return {author_id, canonical_url, scan: structuredClone(scan)};
      };
      qa.finishScan = () => {
        scan = {...scan, status, pages: 1, videos: 1, albums: 1, resume_page: status === 'partial' ? 2 : null};
      };
      api.cancelAuthorScan = async id => {
        if (id !== scan.id) throw Error('unexpected scan canceled');
        qa.calls.push({type: 'author-stop', id});
        scan = {...scan, status: 'partial', error: 'CANCELED', resume_page: 2};
        return structuredClone(scan);
      };
      api.recoverAuthorScan = async (id, reset) => {
        if (id !== scan.id || scan.status !== 'partial') throw Error('unexpected scan recovered');
        qa.calls.push({type: 'author-recover', id, reset});
        scan = {...scan, status: 'scanning', error: null, resume_page: null};
        if (reset) scan = {...scan, id: 'author-reset', pages: 0, videos: 0, albums: 0};
        return structuredClone(scan);
      };
      api.discardAuthorScan = async id => {
        if (id !== scan.id || scan.status === 'scanning') throw Error('unexpected scan discarded');
        qa.calls.push({type: 'author-discard', id});
        scan = null;
        if (qa.holdDiscard) await new Promise(resolve => { qa.releaseDiscard = resolve; });
      };
      api.authorScan = async id => {
        if (id) qa.calls.push({type: 'author-poll', id});
        if (scan?.batch && queued) qa.calls.push({type: 'author-batch-check'});
        return structuredClone(scan);
      };
      qa.pauseBatch = () => { scan.pause_reason = 'AUTHOR_API_LIMIT'; };
      api.confirmAuthorScan = async (id, allowPartial, mode) => {
        qa.calls.push({type: 'author-confirm', id, allowPartial, mode});
        await new Promise((resolve, reject) => {
          qa.releaseConfirm = resolve;
          qa.rejectConfirm = () => reject('NETWORK_ERROR');
        });
        scan.batch = {queued: mode === 'full' ? 2 : 1, skipped: mode === 'full' ? 0 : 1};
        scan.remaining = 1;
        queued = {...task, id: 'batch-work', status: 'queued', download_requested: true,
          video: {...task.video, author_scan_id: id, title: '已入队的作者作品'}};
        return structuredClone(scan.batch);
      };
      api.resumeAuthorBatch = async id => {
        qa.calls.push({type: 'author-resume', id});
        scan.pause_reason = null;
        return structuredClone(scan);
      };
    }, {status, pending});
    await input.fill(source);
    await page.getByRole('button', {name: /^(重新解析|解析链接)$/}).click();
    await stop.waitFor();
    if (await scanButton.count() || await full.count() || await incremental.count()) {
      throw Error('an extra trigger or download mode appeared during automatic scan');
    }
  }

  async function waitForConfirm() {
    await page.waitForFunction(() => typeof window.backgroundTest.releaseConfirm === 'function');
  }

  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({width, height});
    await prepare();
    await page.evaluate(() => window.backgroundTest.finishScan());
    await full.waitFor();
    await incremental.waitFor();
    await page.getByText('已下载 1 条，待下载 1 条').waitFor();
    await page.getByText(/全量下载会覆盖已下载的视频或图集/).waitFor();
    const mode = width === 1280 ? 'incremental' : 'full';
    const confirm = mode === 'incremental' ? incremental : full;
    await confirm.focus();
    await page.keyboard.press('Enter');
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
    await full.waitFor({state: 'hidden'});
    const expected = width === 1024 ? draft : '';
    if (await input.inputValue() !== expected) throw Error('success did not clear only the submitted link');
    if (width !== 1024) await page.waitForFunction(() => document.activeElement?.id === 'video-input');
    const requests = await page.evaluate(() => window.backgroundTest.calls
      .filter(call => call.type === 'author-scan' || call.type === 'author-confirm'));
    if (requests.length !== 3 || requests[0].type !== 'author-scan'
      || requests.slice(1).some(call => call.mode !== mode || call.allowPartial)) {
      throw Error('download mode must be selected at confirmation without rescanning');
    }

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
    await stop.waitFor({state: 'hidden'});
    await input.fill(source);
    await page.getByRole('button', {name: '解析链接', exact: true}).click();
    await stop.waitFor();
    await page.getByText(/已扫描到末页|已入队 [0-9]+ 条/).waitFor({state: 'hidden'});
  }

  await prepare('completed', 0);
  await page.evaluate(() => window.backgroundTest.finishScan());
  await page.getByText('已下载 2 条，待下载 0 条').waitFor();
  if (await incremental.count()) throw Error('incremental download is visible with nothing pending');
  await full.waitFor();
  await dismiss.focus();
  await page.keyboard.press('Enter');
  await page.getByText('粘贴链接开始解析', {exact: true}).waitFor();
  await page.waitForFunction(() => document.activeElement?.id === 'video-input');
  if (await input.inputValue() || await full.count() || await dismiss.count()) {
    throw Error('cancel did not reset the link and scan result');
  }
  if (await page.evaluate(() => window.backgroundTest.calls.some(call => call.type === 'author-confirm' || call.type === 'author-stop'))) {
    throw Error('cancel result unexpectedly submitted work or stopped a scan');
  }
  if (!await page.evaluate(() => window.backgroundTest.calls.some(call => call.type === 'author-discard' && call.id === 'author-scan'))) {
    throw Error('cancel did not discard the session scan');
  }

  await prepare('completed', 0);
  await page.evaluate(() => { window.backgroundTest.finishScan(); window.backgroundTest.holdDiscard = true; });
  await dismiss.click();
  await page.waitForFunction(() => typeof window.backgroundTest.releaseDiscard === 'function');
  await page.getByRole('button', {name: '重新解析', exact: true}).click();
  await stop.waitFor();
  await page.evaluate(() => window.backgroundTest.releaseDiscard());
  await page.waitForFunction(() => window.backgroundTest.calls.filter(call => call.type === 'author-poll').length >= 2);
  if (await input.inputValue() !== source || !await stop.isVisible()) throw Error('late cancel reset a newer parse of the same link');

  await prepare();
  await stop.click();
  await reset.waitFor();
  const buttons = await Promise.all([reset, resumeScan, full, incremental].map(button => button.boundingBox()));
  if (buttons.some((box, index) => !box || Math.abs(box.y - buttons[0].y) > 1
    || (index > 0 && box.x < buttons[index - 1].x + buttons[index - 1].width))) {
    throw Error('interrupted scan actions must share one row in order');
  }
  const colors = await Promise.all([reset, resumeScan, full, incremental].map(button =>
    button.evaluate(node => getComputedStyle(node).backgroundColor)));
  if (new Set(colors).size !== 4) throw Error('interrupted scan actions must have distinct colors');
  await page.getByText('登录配置一致时，从第 2 页继续。').waitFor();
  await resumeScan.focus();
  await page.keyboard.press('Enter');
  await stop.waitFor();
  await page.getByText(/正在扫描，已读 1 页/).waitFor();
  await stop.click();
  await reset.click();
  await stop.waitFor();
  await page.getByText('正在扫描', {exact: true}).waitFor();
  await page.evaluate(() => window.backgroundTest.finishScan());
  await full.waitFor();
  await page.waitForFunction(() => window.backgroundTest.calls.some(call => call.type === 'author-poll' && call.id === 'author-reset'));
  const recoveries = await page.evaluate(() => window.backgroundTest.calls.filter(call => call.type === 'author-recover'));
  if (JSON.stringify(recoveries) !== JSON.stringify([
    {type: 'author-recover', id: 'author-scan', reset: false},
    {type: 'author-recover', id: 'author-scan', reset: true},
  ])) throw Error('continue and reset did not use the interrupted scan');

  await prepare('partial');
  await page.evaluate(() => window.backgroundTest.finishScan());
  await page.getByText(/本次只处理已扫描到的作品/).waitFor();
  await incremental.click();
  await waitForConfirm();
  const partial = await page.evaluate(() => window.backgroundTest.calls.find(call => call.type === 'author-confirm'));
  if (!partial?.allowPartial || partial.mode !== 'incremental') throw Error('partial consent was lost');
  await page.evaluate(() => window.backgroundTest.releaseConfirm());
  await page.getByText('已加入下载任务', {exact: true}).waitFor();
  return {passed: true, viewports: 3, autoScan: true, continued: true, reset: true, keptDraft: true, resumedBatch: true, partialConsent: true};
}
