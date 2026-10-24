// Run through the installed Playwright CLI against npm run dev. All IPC is mocked.
async page => {
  const results = [];
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const check = (value, message) => { if (!value) throw new Error(message); };
  for (const platform of ['douyin', 'tiktok', 'youtube', 'telegram', 'missing', 'broken']) {
    await page.goto(`http://127.0.0.1:1420/tests/fixtures/background-download.html?scenario=author-${platform}`);
    await page.waitForFunction(() => window.backgroundTest.loaded);
    const group = page.getByRole('group', {name: platform === 'telegram' ? '频道信息' : '作者信息', exact: true});
    await group.waitFor();
    check((await group.innerText()).includes(platform === 'telegram' ? 'channel_name' : '作者昵称'), `name: ${platform}`);
    check((await group.innerText()).includes('ID:'), `id: ${platform}`);
    const hasAvatar = !['telegram', 'missing', 'broken'].includes(platform);
    if (hasAvatar) {
      await group.getByRole('img').waitFor();
      await group.getByRole('img').evaluate(image => image.decode());
      check((await group.getByRole('img').getAttribute('src')).startsWith('blob:'), 'avatar bypassed SQLite IPC');
    } else {
      check(await group.getByRole('img').count() === 0, 'fallback should not load remote image');
    }
    for (const pageName of ['下载作品', '下载任务']) {
      await page.getByRole('button', {name: pageName, exact: true}).click();
      for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
        await page.setViewportSize({width, height});
        check(await group.isVisible(), `${platform} ${pageName}: author hidden at ${width}`);
        check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), `${platform}: page overflow at ${width}`);
        const bounds = await group.boundingBox();
        check(bounds.x >= 0 && bounds.x + bounds.width <= width, `${platform}: author overflow at ${width}`);
        if (platform === 'douyin') await page.screenshot({path: `.local/author-${pageName === '下载作品' ? 'preview' : 'tasks'}-${width}.png`});
      }
      if (pageName === '下载任务' && platform !== 'telegram') {
        const search = page.getByPlaceholder('搜索作品标题或作者');
        for (const query of ['作者昵称', 'MS4w.author-id']) {
          await search.fill(query);
          check(await group.isVisible(), `search failed for ${query}`);
        }
        await search.fill('不存在的作者');
        check(await group.count() === 0, 'nonmatching author stayed visible');
        await search.fill('');
      }
    }
    results.push(platform);
  }
  check(errors.length === 0, errors.join('\n'));
  return {passed: results, viewports: ['1440x900', '1280x800', '1024x680'], errors};
}
