// Existing task and backup IPC fixtures. No requests to media platforms or cloud services.
async page => {
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const open = scenario => page.goto(`${base}/tests/fixtures/background-download.html?scenario=${scenario}`);
  const downloadTasks = page.getByRole('button', { name: '下载任务', exact: true });
  const preview = page.locator('.media-preview');
  const time = preview.locator('time');
  const setPublication = fields => page.evaluate(fields => {
    const control = window.backgroundTest;
    const [task] = control.snapshot();
    delete task.video.published_at;
    delete task.video.published_date;
    Object.assign(task.video, fields);
    task.revision++;
    control.setSnapshot([task]);
    control.emit(task);
  }, fields);

  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height });
    await open('youtube');
    await preview.getByText('原作品', { exact: true }).waitFor();
    check(await preview.getByText('发布时间').count() === 0, 'unknown publication time was shown');
    await setPublication({ published_at: 1700000001 });
    await time.waitFor();
    check(await time.getAttribute('datetime') === '2023-11-14T22:13:21.000Z', 'publication timestamp lost its precision');
    check(/:\d{2}:21$/.test(await time.innerText()), 'publication seconds are not visible');
    await setPublication({ published_date: '2024-02-29' });
    await preview.locator('time[datetime="2024-02-29"]').waitFor();
    check(await time.getAttribute('datetime') === '2024-02-29', 'date-only value shifted or gained a time');
    await setPublication({});
    await time.waitFor({ state: 'detached' });

    await open('unified-card-status');
    await page.evaluate(() => {
      const { assets, change } = window.unifiedTest;
      for (const id of ['asset-old-operation', 'asset-backup-canceled']) {
        assets.find(asset => asset.asset_id === id).has_pending_local_changes = true;
      }
      change('op-backup-failed', { last_error: 'BACKUP_NETWORK_ERROR' });
      const control = window.backgroundTest;
      const tasks = control.snapshot();
      const failed = tasks.find(task => task.id === 'backup-failed');
      failed.error = 'NETWORK_ERROR';
      failed.revision++;
      control.setSnapshot(tasks);
      control.emit(failed);
    });
    await downloadTasks.click();
    const all = page.getByRole('tab', { name: '全部', exact: true });
    const statuses = page.getByLabel('按状态筛选下载任务');
    const sync = statuses.getByLabel('同步中', { exact: true });
    const syncCount = sync.locator('.task-filter-count');
    check(await statuses.getByLabel('已完成', { exact: true }).isVisible() &&
      await statuses.getByLabel('失败', { exact: true }).isVisible(), 'sync filter is not grouped with download statuses');
    check(await page.getByRole('tab', { name: '同步中', exact: true }).count() === 0, 'sync filter is in the platform tabs');
    await sync.press('Space');
    check(await sync.getAttribute('data-state') === 'on', 'keyboard did not select the sync filter');
    const panel = page.getByRole('tabpanel');
    const row = id => panel.locator(`[data-task-id="${id}"]`);
    const upload = panel.locator('[data-asset-id="remote-upload"]');
    await upload.waitFor();
    await panel.locator('[data-asset-id="asset-download-wins"]').waitFor();
    for (const id of ['pending', 'upload', 'retry', 'backup-failed', 'old-operation']) await row(id).waitFor();
    for (const id of ['done', 'restore', 'foreign', 'download', 'download-wins', 'backup-canceled', 'cleaning']) {
      check(await row(id).count() === 0, `unrelated item in sync list: ${id}`);
    }
    const total = await syncCount.innerText();
    check(Number(total) === await panel.getByRole('listitem').count(), 'sync count does not match the visible rows');
    await page.getByRole('tab', { name: 'YouTube', exact: true }).click();
    await panel.getByText('暂无待同步作品').waitFor();
    check(await syncCount.innerText() === '0', 'platform did not filter the sync count');
    check(await sync.getAttribute('data-state') === 'on', 'changing platforms cleared the sync filter');
    await all.click();
    await upload.waitFor();
    check(await syncCount.innerText() === total, 'returning to all platforms did not restore the sync count');
    const syncBadges = [[row('pending'), '待同步'], [row('retry'), '待同步'], [row('old-operation'), '待同步'],
      [row('upload'), '同步中'], [row('backup-failed'), '同步失败'], [row('unknown'), '待确认'],
      [row('dependency'), '待确认'], [upload, '同步中']];
    const toList = page.getByRole('button', { name: '切换为列表', exact: true });
    if (await toList.count()) await toList.click();
    for (const [item, label] of syncBadges) await item.getByText(label, { exact: true }).waitFor();
    const toCards = page.getByRole('button', { name: '切换为信息流', exact: true });
    await toCards.click();
    await panel.locator('[data-view="feed"]').waitFor();
    for (const [item, label] of syncBadges) {
      check(await item.locator('.task-feed-status').innerText() === label, `sync card did not show ${label}`);
      check(await item.getByText('已完成', { exact: true }).count() === 0, 'download completion shown on sync card');
    }
    await row('upload').getByText('上传进度 40%', { exact: true }).waitFor();
    await row('backup-failed').getByRole('alert').getByText('无法连接远端存储，请检查网络后重试').waitFor();
    await row('dependency').getByRole('alert').getByText('Telegram 发送结果尚未确认，请重试').waitFor();
    const search = page.getByRole('textbox', { name: '搜索作品标题或作者', exact: true });
    await search.fill('remote-upload');
    await row('upload').waitFor({ state: 'detached' });
    await upload.waitFor();
    check(await syncCount.innerText() === '1', 'search did not filter the sync count');
    await page.evaluate(() => window.unifiedTest.change('op-remote-upload', { state: 'blocked', last_error: 'BACKUP_SOURCE_CHANGED' }));
    await upload.getByText('同步失败', { exact: true }).waitFor();
    await page.evaluate(() => window.unifiedTest.change('op-remote-upload', { state: 'completed', phase: 'completed' }));
    await panel.getByText('没有匹配的同步任务').waitFor();
    await search.fill('');
    await page.getByRole('button', { name: '切换为列表', exact: true }).click();
    await panel.locator('[data-view="list"]').waitFor();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `horizontal overflow at ${width}`);
    await statuses.getByLabel('已完成', { exact: true }).click();
    await row('done').waitFor();
  }

  await open('unified-card-status-off');
  await downloadTasks.click();
  const pausedPanel = page.getByRole('tabpanel');
  const pending = pausedPanel.locator('[data-task-id="pending"]');
  await pending.getByText('已完成', { exact: true }).waitFor();
  await page.getByLabel('按状态筛选下载任务').getByLabel('同步中', { exact: true }).click();
  await pending.getByText('待同步', { exact: true }).waitFor();
  await page.getByRole('button', { name: '切换为信息流', exact: true }).click();
  check(await pending.locator('.task-feed-status').innerText() === '待同步', 'paused queue was shown as completed');

  for (const [count, expected] of [[9999, '9,999'], [10000, '1.0万'], [12345678, '1.2千万'], [250000000, '2.5亿']]) {
    await open('author-results');
    await downloadTasks.click();
    const author = page.locator('[data-task-id="first"]').getByRole('button', { name: '查看作者昵称的作品' });
    await author.waitFor();
    await page.evaluate(count => {
      const control = window.backgroundTest;
      const tasks = control.snapshot();
      for (const task of tasks.slice(0, 2)) {
        task.video.author_profile.follower_count = count;
        task.revision++;
        control.emit(task);
      }
      control.setSnapshot(tasks);
    }, count);
    await author.click();
    await page.getByRole('region', { name: '作者昵称' }).getByText(expected, { exact: true }).waitFor();
  }
  return { passed: true, viewports: 3, followerCounts: 4 };
}
