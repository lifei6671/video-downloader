// Browser-only IPC fixture: no real account or platform request.
async page => {
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({width, height});
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=author-bilibili');
    await page.evaluate(() => localStorage.removeItem('yingqu.task-view'));
    await page.reload();
    const preview = page.locator('.media-preview');
    await preview.getByLabel('作者简介', {exact: true}).waitFor();
    check((await preview.innerText()).includes('B站作者的简介'), 'preview biography missing');
    await preview.getByRole('img', {name: 'B站作者的头像', exact: true}).evaluate(img => img.decode());
    await page.getByRole('button', {name: '下载任务', exact: true}).click();
    const row = page.locator('[data-task-id="author-list"]');
    await row.getByLabel('作者简介', {exact: true}).waitFor();
    await row.getByRole('img', {name: 'B站作者的头像', exact: true}).evaluate(img => img.decode());
    await page.getByRole('button', {name: /切换为(信息流|卡片)/}).click();
    await page.locator('.task-list[data-view="feed"]').waitFor();
    check(await row.getByLabel('作者简介', {exact: true}).isVisible(), 'feed biography missing');
    await row.getByRole('img', {name: 'B站作者的头像', exact: true}).evaluate(img => img.decode());
    await row.getByRole('button', {name: '查看B站作者的作品', exact: true}).focus();
    await page.keyboard.press('Enter');
    const profile = page.getByRole('region', {name: 'B站作者', exact: true});
    await profile.waitFor();
    check((await profile.innerText()).includes('这是第二行简介。'), 'author page biography missing');
    await profile.getByRole('img', {name: 'B站作者的头像', exact: true}).evaluate(img => img.decode());
    check(await profile.getByRole('heading').evaluate(el => document.activeElement === el), 'author heading focus missing');
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
    results.push({width, height, preview: true, tasks: true, authorPage: true});
  }
  return results;
}
