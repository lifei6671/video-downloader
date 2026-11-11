// Run through the installed Playwright CLI against npm run dev. All IPC is mocked.
async page => {
  const results = [];
  const base = page.url().split('/').slice(0,3).join('/');
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const check = (value, message) => { if (!value) throw new Error(message); };
  for (const platform of ['douyin', 'tiktok', 'youtube', 'telegram', 'missing', 'broken']) {
    await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-${platform}`);
    await page.waitForFunction(() => window.backgroundTest.loaded);
    const group = page.getByRole('group', {name: platform === 'telegram' ? '频道信息' : '作者信息', exact: true});
    await group.waitFor();
    check((await group.innerText()).includes(platform === 'telegram' ? 'channel_name' : '作者昵称'), `name: ${platform}`);
    check((await group.innerText()).includes('ID:') === (platform !== 'telegram'), `id visibility: ${platform}`);
    const hasAvatar = !['missing', 'broken'].includes(platform);
    if (hasAvatar) {
      await group.getByRole('img').waitFor();
      await group.getByRole('img').evaluate(image => image.decode());
      check((await group.getByRole('img').getAttribute('src')).startsWith('blob:'), 'avatar bypassed SQLite IPC');
    } else {
      check(await group.getByRole('img').count() === 0, 'fallback should not load remote image');
    }
    for (const pageName of ['下载作品', '下载任务']) {
      await page.getByRole('button', {name: pageName, exact: true}).click();
      if (pageName === '下载任务') {
        await group.getByText(platform === 'telegram' ? 'channel_name' : '作者昵称', {exact: true}).waitFor();
        if (hasAvatar) {
          const image = group.getByRole('img');
          await image.waitFor();
          await image.evaluate(node => node.decode());
          const avatarBox = await image.boundingBox();
          const nameBox = await group.getByText(platform === 'telegram' ? 'channel_name' : '作者昵称', {exact: true}).boundingBox();
          check(avatarBox.x + avatarBox.width <= nameBox.x, 'avatar must be before the name');
        } else {
          check(await group.getByRole('img').count() === 0, 'task avatar fallback loaded remote media');
        }
      }
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
    if (platform === 'douyin') {
      await page.evaluate(() => {
        const tasks = window.backgroundTest.snapshot();
        const listed = tasks.find(task => task.download_requested);
        listed.revision++;
        listed.video.author_profile.nickname = '   ';
        window.backgroundTest.setSnapshot(tasks);
        window.dispatchEvent(new Event('focus'));
      });
      await page.getByText('MS4w.' + 'author-id-'.repeat(6), {exact:true}).waitFor();
      await page.evaluate(() => {
        const tasks = window.backgroundTest.snapshot();
        const listed = tasks.find(task => task.download_requested);
        listed.revision++;
        listed.video.author_profile = null;
        listed.video.author = '旧记录昵称';
        window.backgroundTest.setSnapshot(tasks);
        window.dispatchEvent(new Event('focus'));
      });
      await page.getByText('旧记录昵称', {exact:true}).waitFor();
    }
    results.push(platform);
  }
  await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-missing`);
  await page.waitForFunction(() => window.backgroundTest.loaded);
  await page.evaluate(() => {
    const tasks = window.backgroundTest.snapshot();
    const legacyBatch = tasks.find(task => task.download_requested);
    legacyBatch.revision++;
    legacyBatch.video.author_scan_id = 'old-batch';
    delete legacyBatch.video.author_profile.avatar_cached;
    window.backgroundTest.setSnapshot(tasks);
    window.dispatchEvent(new Event('focus'));
  });
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  const cached = page.getByRole('img', {name: '作者昵称的头像', exact: true});
  await cached.waitFor();
  await cached.evaluate(image => image.decode());
  check((await cached.getAttribute('src')).startsWith('blob:'), 'legacy batch did not load the shared avatar');
  results.push('legacy-batch');

  await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-results`);
  await page.evaluate(() => localStorage.removeItem('yingqu.task-view'));
  await page.reload();
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  const search = page.getByRole('textbox', {name: '搜索作品标题或作者', exact: true});
  await search.fill('原作品');
  await page.getByRole('tab', {name: '抖音', exact: true}).click();
  const completed = page.getByLabel('已完成', {exact: true});
  await completed.click();
  const authorLink = page.getByRole('button', {name: '查看作者昵称的作品', exact: true});
  await authorLink.focus();
  await page.keyboard.press('Enter');
  const profile = page.getByRole('region', {name: '作者昵称', exact: true});
  await profile.waitFor();
  check(await profile.getByRole('heading').evaluate(node => node === document.activeElement), 'profile heading did not receive focus');
  check((await profile.innerText()).includes('第二行简介'), 'saved biography missing');
  check(await profile.getByText('@account_handle', {exact: true}).isVisible(), 'account handle missing');
  check(await profile.getByText('作者 ID', {exact: true}).count() === 0, 'Douyin author ID label was displayed');
  check(await profile.getByText('author-1', {exact: true}).count() === 0, 'Douyin author ID was displayed');
  check(await profile.getByText('12,345', {exact: true}).isVisible(), 'follower count missing');
  check(await profile.getByText('0', {exact: true}).isVisible(), 'a reported zero was hidden');
  check(await profile.getByText('获赞', {exact: true}).count() === 0, 'unknown likes were displayed');
  for (const value of ['28 岁', '中国 上海', '浙江']) {
    check(await profile.getByText(value, {exact: true}).isVisible(), `public profile detail missing: ${value}`);
  }
  const profileAvatar = profile.getByRole('img', {name: '作者昵称的头像', exact: true});
  await profileAvatar.waitFor();
  check((await profileAvatar.getAttribute('src')).startsWith('blob:'), 'profile avatar did not use cached bytes');
  check(await search.count() === 0 && await page.getByRole('button', {name: '清理历史', exact: true}).count() === 0, 'profile exposed download-task search or bulk actions');
  const works = page.getByRole('list', {name: '媒体任务列表', exact: true});
  const workIds = () => works.locator('[data-task-id]').evaluateAll(rows => rows.map(row => row.dataset.taskId));
  check(JSON.stringify(await workIds()) === JSON.stringify(['first', 'second', 'legacy']), 'profile mixed identities or dropped previews');
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({width, height});
    const toCards = page.getByRole('button', {name: '切换为卡片', exact: true});
    await toCards.focus();
    await page.keyboard.press('Enter');
    check(await works.getAttribute('data-view') === 'feed', 'profile did not switch to cards');
    check(JSON.stringify(await workIds()) === JSON.stringify(['first', 'second', 'legacy']), 'layout switch changed works');
    check(await works.getByRole('button', {name: '下载视频', exact: true}).isVisible(), 'profile lost ready action');
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'profile overflow');
    await page.screenshot({path: `.local/author-home-card-${width}.png`});
    await page.getByRole('button', {name: '切换为列表', exact: true}).click();
    check(await works.getAttribute('data-view') === 'list', 'profile did not switch back to list');
    await page.screenshot({path: `.local/author-home-list-${width}.png`});
  }
  await page.evaluate(() => {
    const sibling = window.backgroundTest.snapshot().find(task => task.id === 'second');
    sibling.revision++;
    Object.assign(sibling.video.author_profile, {profile_updated_at:11,follower_count:12346,age:null});
    window.backgroundTest.emit(sibling);
  });
  await profile.getByText('12,346', {exact: true}).waitFor();
  check(await profile.getByText('年龄', {exact: true}).count() === 0, 'sibling event did not clear the previously visible age');
  await page.getByRole('button', {name: '返回下载任务', exact: true}).click();
  check(await search.inputValue() === '原作品', 'return lost task search');
  check(await page.getByRole('tab', {name: '抖音', exact: true}).getAttribute('aria-selected') === 'true', 'return lost platform filter');
  check(await completed.getAttribute('data-state') === 'on', 'return lost status filter');
  await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === '查看作者昵称的作品');
  await authorLink.click();
  await profile.waitFor();
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  check(await search.inputValue() === '原作品', 'sidebar return lost task search');
  results.push('author-home');

  await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-missing`);
  const previewAuthor = page.getByRole('button', {name: '查看作者昵称的作品', exact: true});
  await previewAuthor.click();
  await profile.waitFor();
  check(await profile.getByRole('img').count() === 0, 'missing profile avatar requested remote media');
  check(await profile.locator('.author-profile-bio').count() === 0, 'profile invented a missing biography');
  for (const label of ['粉丝', '关注', '获赞', '年龄', '地区', 'IP 属地']) {
    check(await profile.getByText(label, {exact: true}).count() === 0, `unknown profile detail displayed: ${label}`);
  }
  await page.evaluate(() => {
    const task = window.backgroundTest.snapshot().find(task => task.id === 'author-list');
    task.revision++;
    task.video.author_profile.avatar_cached = true;
    window.backgroundTest.emit(task);
  });
  await profile.getByRole('img').waitFor();
  await page.getByRole('button', {name: '返回下载作品', exact: true}).click();
  await previewAuthor.waitFor();
  await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === '查看作者昵称的作品');
  results.push('profile-preview-return');

  await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-telegram`);
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  await page.getByRole('button', {name: '查看channel_name的作品', exact: true}).click();
  const channel = page.getByRole('region', {name: 'channel_name', exact: true});
  await channel.waitFor();
  check(await channel.getByText('频道 ID', {exact: true}).isVisible() && await channel.getByText('100', {exact: true}).isVisible(), 'channel profile lost its stable identity');
  check((await workIds()).length === 2, 'channel profile did not group messages');
  results.push('channel-home');

  await page.goto(`${base}/tests/fixtures/background-download.html?scenario=author-results`);
  await page.getByRole('button', {name: '下载任务', exact: true}).click();
  await page.getByRole('button', {name: '查看account_handle的作品', exact: true}).click();
  check(JSON.stringify(await workIds()) === JSON.stringify(['legacy']), 'legacy profile combined different confirmed authors');
  await works.getByLabel('任务操作', {exact: true}).focus();
  await page.keyboard.press('Shift+F10');
  await page.getByRole('menuitem', {name: '从列表移除', exact: true}).focus();
  await page.keyboard.press('Enter');
  await page.getByText('暂无作品记录', {exact: true}).waitFor();
  await page.waitForFunction(() => document.activeElement?.matches('.task-list-scroll'));
  results.push('last-work-removal-focus');
  check(errors.length === 0, errors.join('\n'));
  return {passed: results, viewports: ['1440x900', '1280x800', '1024x680'], errors};
}
