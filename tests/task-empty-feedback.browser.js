// Isolated IPC fixture: no real downloads or history changes.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height });
    await page.goto(base + '/tests/fixtures/task-platforms.html?empty=telegram');
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
    const platforms = page.getByRole('tablist', { name: '按平台筛选下载任务' });
    await platforms.getByRole('tab', { name: 'Telegram', exact: true }).click();
    await page.getByText('该平台暂无下载任务', { exact: true }).waitFor();
    check(await page.getByRole('button', { name: '去下载作品', exact: true }).count() === 0, 'empty task list has navigation CTA');
    check(await platforms.evaluate(n => getComputedStyle(n).borderBottomWidth === '0px'), 'platform bar still has underline');
    check(await platforms.getByRole('tab', { selected: true }).evaluate(n => getComputedStyle(n, '::after').display === 'none'), 'platform selection still has underline');
    const empty = page.locator('.task-browser [data-slot="tabs-content"] > [data-slot="card"]');
    check(await empty.evaluate(n => ['Top', 'Right', 'Bottom', 'Left'].every(side => getComputedStyle(n)['border' + side + 'Width'] === '0px') && getComputedStyle(n).boxShadow === 'none'), 'empty state has card border or shadow');
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'horizontal overflow');
    await page.screenshot({ path: `output/task-ui-v5/empty-${width}.png` });
    await page.getByRole('button', { name: '下载中心', exact: true }).click();
    await page.evaluate(async () => {
      const { api } = await import('/src/api');
      api.resolve = async () => { window.unexpectedResolve = true; throw Error('empty input reached IPC'); };
    });
    await page.locator('#video-input').fill('   ');
    await page.locator('.share-card button[type="submit"], .share-card button:not([type])').filter({ hasText: /解析/ }).click();
    await page.locator('[data-sonner-toast]').getByText('请输入视频分享链接', { exact: true }).waitFor();
    check(await page.locator('#video-input').evaluate(n => n === document.activeElement), 'empty input did not regain focus');
    check(await page.locator('[data-slot="page-content"] [data-slot="alert"]').filter({ hasText: '请输入视频分享链接' }).count() === 0, 'empty input rendered persistent error');
    check(!await page.evaluate(() => window.unexpectedResolve), 'empty input invoked resolve');
    await page.locator('#video-input').press('Enter');
    check(await page.locator('[data-sonner-toast]').filter({ hasText: '请输入视频分享链接' }).count() === 1, 'repeated submit duplicated toast');
    await page.waitForFunction(() => {
      const toast = document.querySelector('[data-sonner-toast]');
      return toast && toast.getBoundingClientRect().top >= 24 && getComputedStyle(toast).opacity === '1';
    });
    await page.screenshot({ path: `output/task-ui-v5/toast-${width}.png` });
    results.push(`${width}: empty border, platform tabs, Toast, focus, no IPC, no duplicate`);
  }
  return { status: 'PASS', results };
}
