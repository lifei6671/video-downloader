// Full React app with synthetic IPC. No native sessions or cloud requests.
async page => {
  page.setDefaultTimeout(10000);
  const base = page.url().split('/').slice(0, 3).join('/'), results = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const row = id => page.locator(`[data-task-id="${id}"]`);
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    for (const view of ['feed', 'list']) {
      await page.setViewportSize({width, height});
      await page.evaluate(view => localStorage.setItem('yingqu.task-view', view), view);
      await page.goto(base + '/tests/fixtures/background-download.html?scenario=unified-card-status');
      await page.getByRole('button', {name: '下载任务', exact: true}).click();
      await row('upload').locator('.task-card-message').waitFor();
      for (const [id, label] of [['pending','待备份'], ['preparing','准备备份'], ['upload','备份中 40%'], ['confirm','确认备份'], ['retry','等待重试'], ['cleaning','清理中'], ['backup-canceled','备份已取消']]) {
        const line = row(id).locator('.task-card-feedback');
        check((await line.textContent()).includes(label), `${view}/${width}/${id}: missing ${label}`);
        check(!/点击|取消上传|取消下载|重试备份|上传备份|下载到本机|重新上传/.test(await line.textContent()), `${view}/${width}/${id}: passive status contains an action hint`);
        const layout = await line.evaluate(node => {
          const text = node.querySelector('.task-card-message'), parent = node.closest('.task-row-information');
          const rect = node.getBoundingClientRect(), bounds = parent.getBoundingClientRect();
          return {height: rect.height, inside: rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1 && rect.bottom <= bounds.bottom + 1,
            nowrap: getComputedStyle(text).whiteSpace === 'nowrap', oneLine: text.scrollHeight <= rect.height + 1};
        });
        check(layout.height >= 18 && layout.inside && layout.nowrap && layout.oneLine, `${view}/${width}/${id}: wrapped or overflowing status`);
      }
      check(await row('retry').getByRole('alert').count() === 0, 'automatic retry exposed old failure');
      for (const id of ['backup-failed','unknown','restore','foreign','unregistered']) {
        check(!/点击|取消上传|取消下载|重试备份|上传备份|下载到本机|重新上传/.test(await row(id).locator('.task-card-feedback').textContent()), `${id}: error/restore/status line contains an action hint`);
      }
      await row('preparing').locator('.backup-sync-tooltip').hover();
      await page.getByRole('tooltip').filter({hasText:'取消上传'}).waitFor();
      await page.keyboard.press('Escape');await page.mouse.move(0,0);
      await row('preparing').locator('.task-card-message').click();
      check(await page.evaluate(()=>window.unifiedTest.calls.length)===0,'passive status line triggered a transfer action');
      check(await row('dependency').locator('.backup-sync-button').isDisabled(), 'unconfirmed cleanup enabled a new upload');
      check((await row('dependency').locator('.task-card-message').textContent()).includes('待确认'), 'cleanup dependency lost its confirmation status');
      await row('dependency').locator('.backup-sync-button').evaluate(node => node.click());
      check(await page.evaluate(() => !window.unifiedTest.calls.some(call => call.type === 'start' && call.id === 'asset-dependency')), 'unconfirmed cleanup started a redundant upload');
      const before = await row('upload').evaluate(node => node.getBoundingClientRect().height);
      await page.evaluate(() => window.unifiedTest.change('op-upload', {state:'blocked', attempt:5, last_error:'TELEGRAM_CONNECTION_ERROR'}));
      await row('upload').getByRole('alert').waitFor();
      check(await row('upload').getByRole('alert').count() === 1, 'terminal error was duplicated');
      check(Math.abs((await row('upload').evaluate(node => node.getBoundingClientRect().height)) - before) < 1, 'terminal failure changed row height');
      await page.evaluate(() => {
        const failed = window.unifiedTest.assets.find(asset => asset.asset_id === 'asset-upload');
        failed.remote_state = 'backed_up'; failed.has_pending_local_changes = true;
        const asset = window.unifiedTest.assets.find(asset => asset.asset_id === 'asset-done');
        asset.backup_locations = [{space_id:'status-space', display_name:'用于验证单行省略的很长备份空间名称'.repeat(30)}];
        window.unifiedTest.operations.push({operation_id:'completed-upload',space_id:'status-space',asset_id:'asset-done',kind:'upload',state:'completed',phase:'completed',payload:{},last_error:null,attempt:1});
        window.unifiedTest.operations.unshift({operation_id:'cleaned-old-upload',space_id:'status-space',asset_id:'asset-done',kind:'upload',state:'blocked',phase:'cancel_cleaned',payload:{},last_error:'TELEGRAM_CONNECTION_ERROR',attempt:1});
      });
      await page.waitForFunction(() => document.querySelector('[data-task-id="done"] .task-card-message')?.textContent.includes('用于验证'));
      check(await row('upload').locator('.backup-sync-button').isEnabled(), 'existing remote copy disabled retry of a failed newer edit');
      check(await row('done').locator('.backup-sync-button').isDisabled(), 'a confirmed completed upload still offers a redundant upload');
      check(await row('done').getByRole('alert').count() === 0, 'a cleaned historical upload obscured confirmed completion');
      const long = await row('done').locator('.task-card-message').evaluate(node => ({nowrap: getComputedStyle(node).whiteSpace === 'nowrap', clipped: node.scrollWidth > node.clientWidth, oneLine: node.scrollHeight <= node.clientHeight + 1}));
      check(long.nowrap && long.clipped && long.oneLine, 'long destination was not ellipsized on one line');
      const unlinked = page.locator('[data-asset-id="remote-upload"] .task-card-feedback .task-card-message');
      check((await unlinked.textContent()).includes('备份中 40%'), 'unlinked asset lacks the shared status line');
      if (width === 1440) {
        await row('upload').scrollIntoViewIfNeeded();
        await page.screenshot({path: `.local/media-pool-recovery/status-${view}.png`});
      }
      results.push(`${view}/${width}: state/percentage, one line, long-name ellipsis, terminal error slot, stable height and unlinked asset`);
    }
  }
  return {passed: results.length, results};
}
