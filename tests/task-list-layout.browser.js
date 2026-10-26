// Run with the existing Playwright CLI against isolated IPC fixtures, never user data.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const row = id => page.locator(`[data-task-id="visual-${id}"]`);
  const results = [];
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const load = async scenario => {
    await page.goto(`${base}/tests/fixtures/background-download.html?scenario=${scenario}`);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await page.getByRole('button', {name:'下载任务', exact:true}).click();
    await page.getByRole('list', {name:'媒体任务列表'}).waitFor();
    await row(3).locator('.media-cover-image').evaluate(image => image.decode());
  };
  const screenshot = async name => page.screenshot({path:`output/playwright/task-list-layout/${name}.png`, animations:'disabled'});
  for (const [width, height] of [[1440,900], [1280,800], [1024,680]]) {
    await page.setViewportSize({width, height});
    await load('visual-list');
    const geometry = await page.locator('.task-row').evaluateAll(rows => rows.slice(0,7).map(node => {
      const bounds = node.getBoundingClientRect();
      const status = node.querySelector('.task-row-status').getBoundingClientRect();
      const actions = node.querySelector('.task-row-controls').getBoundingClientRect();
      const cover = node.querySelector('[data-media-cover]').getBoundingClientRect();
      return {id:node.dataset.taskId, height:bounds.height, center:bounds.y+bounds.height/2,
        statusCenter:status.y+status.height/2, actionCenter:actions.y+actions.height/2,
        statusX:status.x, actionX:actions.x, coverWidth:cover.width, coverHeight:cover.height,
        radius:getComputedStyle(node).borderRadius, border:getComputedStyle(node).borderTopWidth};
    }));
    for (const g of geometry) {
      check(Math.abs(g.center-g.statusCenter)<1 && Math.abs(g.center-g.actionCenter)<1, `unaligned state/actions ${g.id} at ${width}`);
      check(g.statusX===geometry[0].statusX && g.actionX===geometry[0].actionX, 'columns shifted between rows');
      check(g.coverWidth===56 && g.coverHeight===56, 'thumbnail container dimensions');
      check(g.radius==='8px' && g.border==='0px', 'independent cards remain');
    }
    check(geometry.find(g=>g.id==='visual-3').height===80, 'completed row is not 80px');
    check(geometry.find(g=>g.id==='visual-1').height>=120, 'progress clips in downloading row');
    check(await row(1).getByRole('progressbar').getAttribute('aria-valuenow')==='49', 'real progress changed');
    check((await row(1).innerText()).includes('预计剩余 17 秒'), 'ETA missing');
    check((await row(2).innerText()).includes('队列第 1 位'), 'queue position missing');
    check(await row(4).getByRole('alert').isVisible(), 'failure is not persistent');
    check(await row(3).locator('.task-row-byline').innerText().then(text=>text.includes('YouTube') && !text.includes('ID:')), 'source/author hierarchy');
    check(await row(3).locator('.task-row-title').evaluate(node=>node.scrollWidth>node.clientWidth), 'long title did not truncate');
    check(await row(3).locator('.task-row-title').evaluate(node=>getComputedStyle(node).fontSize==='14px'), 'title typography');
    const downloadedAt = row(3).getByLabel(/下载时间：/);
    check(await downloadedAt.isVisible(), 'download timestamp missing');
    check(await downloadedAt.evaluate(node => node.previousElementSibling?.getAttribute('title') === '文件大小'), 'download timestamp must follow file size');
    check(await row(1).getByLabel(/下载时间：/).count()===0, 'unfinished task shows download timestamp');
    const adjacent = await page.locator('[data-task-row]').evaluateAll(rows=>rows.slice(0,7).map(n=>{const r=n.getBoundingClientRect();return {top:r.top,bottom:r.bottom}}));
    check(adjacent.every((r,i)=>!i || Math.abs(r.top-adjacent[i-1].bottom)<1), 'gaps between media rows');
    const backgrounds = await page.locator('.task-row').evaluateAll(rows=>rows.slice(0,7).map(n=>{const r=n.getBoundingClientRect();return {top:r.top,bottom:r.bottom}}));
    check(backgrounds.every((r,i)=>!i || Math.abs(r.top-backgrounds[i-1].bottom-4)<1), 'task backgrounds need 4px of whitespace');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth), 'horizontal overflow');
    await screenshot(`mixed-${width}`);

    const title = row(3).locator('.task-row-title');
    const cover = row(3).getByRole('button', {name:'查看封面',exact:true});
    await title.click();
    check(await page.getByRole('dialog').count()===0, 'title opens separate details');
    const fullTitle = await title.getAttribute('title');
    await cover.focus();
    await page.keyboard.press('Enter');
    const dialog = page.getByRole('dialog', {name:'任务详情'});
    await dialog.waitFor();
    check(await dialog.getByText(fullTitle, {exact:true}).isVisible(), 'full title inaccessible');
    for (const text of ['local-author-3', 'visual-3', 'C:\\TestVideos\\evening-3.mp4'])
      check(await dialog.getByText(text, {exact:true}).count()>=1, `detail lost ${text}`);
    check(await dialog.getByLabel(/下载时间：/).isVisible(), 'download date missing in details');
    const dialogBox = await dialog.boundingBox();
    check(dialogBox.y>=0 && dialogBox.y+dialogBox.height<=height, 'details outside viewport');
    await screenshot(`details-${width}`);
    await page.keyboard.press('Escape');
    await page.waitForFunction(()=>document.activeElement?.getAttribute('data-media-cover')==='thumbnail');
    check(await cover.evaluate(node=>node===document.activeElement), 'details focus not restored');

    const more = row(3).locator('..');
    await more.focus();
    const triggerBox = await more.boundingBox();
    await page.keyboard.press('Shift+F10');
    const menu = page.getByRole('menu');
    await menu.waitFor();
    const menuBox = await menu.boundingBox();
    check(menuBox.x>=0 && menuBox.x+menuBox.width<=width && menuBox.y>=0 && menuBox.y+menuBox.height<=height, 'context menu escaped viewport');
    check((await menu.getByRole('menuitem').allTextContents()).map(t=>t.trim()).join(',')==='打开文件夹,提取文案,复制原链接,复制本地地址,从列表移除,删除本地视频', 'secondary actions changed');
    await page.keyboard.press('End');
    // Radix roving focus moves to the last item on the next timer turn.
    await page.waitForFunction(() => document.activeElement?.getAttribute('data-variant') === 'destructive' && document.activeElement?.textContent === '删除本地视频');
    check(await menu.getByRole('menuitem', {name:'删除本地视频', exact:true}).evaluate(node=>node===document.activeElement), 'menu keyboard navigation');
    await screenshot(`menu-${width}`);
    await page.keyboard.press('Escape');
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-context-target'));
    check(await more.evaluate(node=>node===document.activeElement), 'menu focus not restored');

    await load('visual-list-completed');
    check(await page.locator('.task-row').count()>6, 'not enough completed media');
    check(await row(5).getByText('文件不存在', {exact:true}).isVisible(), 'missing-file feedback hidden');
    check(await row(5).getByRole('button', {name:'播放视频', exact:true}).count()===0, 'missing file is playable');
    const albumAction = await row(6).getByRole('button', {name:'查看图集', exact:true}).boundingBox();
    const albumMore = await row(6).locator('.task-row-controls').boundingBox();
    check(Math.abs(albumAction.y+albumAction.height/2-albumMore.y-albumMore.height/2)<1, 'album actions split into two rows');
    await row(6).getByRole('img', {name:'第 1 张图片', exact:true}).waitFor();
    check(await row(7).getByRole('button', {name:'查看封面', exact:true}).getAttribute('aria-disabled')==='false', 'missing cover blocks task details');
    check((await row(8).innerText()).includes('Telegram 视频 1001234567890_12345'), 'untitled source identity invented/lost');
    await row(3).click({position:{x:8,y:8}});
    await row(5).hover({position:{x:8,y:8}});
    check(await row(3).getAttribute('data-selected')==='true', 'selected background lost');
    await screenshot(`completed-${width}`);
    for (const id of [3,9,10]) {
      const img = row(id).locator('.media-cover-image');
      await img.evaluate(node=>node.decode());
      check(await img.evaluate(node=>getComputedStyle(node).objectFit==='cover'), 'task thumbnail does not fill its container');
    }
    const tabs = page.getByRole('tablist', {name:'按平台筛选下载任务'});
    const tabsY = (await tabs.boundingBox()).y;
    const panel = page.getByRole('tabpanel');
    await panel.focus();
    await page.keyboard.press('Control+End');
    await page.waitForFunction(()=>{const panel=document.querySelector('[role="tabpanel"]');return panel.scrollHeight-panel.clientHeight-panel.scrollTop<2});
    check(Math.abs((await tabs.boundingBox()).y-tabsY)<1, 'tabs scroll with media');
    await panel.evaluate(node=>{node.scrollTop=0});
    results.push(`${width}: continuous rows, status/actions, media ratios, long/absent title, details, menu/keyboard, completed/missing/album, scroll`);
  }
  await load('visual-list-long-details');
  const longTitle = row(3).getByRole('button', {name:'查看封面',exact:true});
  await longTitle.focus();
  await page.keyboard.press('Enter');
  const longDialog = page.getByRole('dialog', {name:'任务详情'});
  const region = longDialog.getByRole('region', {name:'任务详细信息'});
  await region.waitFor();
  check(await region.evaluate(node=>node.scrollHeight>node.clientHeight+500), 'long details did not create a scroll region');
  for (let i=0;i<3 && !await region.evaluate(node=>node===document.activeElement);i++) await page.keyboard.press('Tab');
  check(await region.evaluate(node=>node===document.activeElement), 'details body cannot be reached by keyboard');
  await page.keyboard.press('Control+End');
  await page.waitForFunction(()=>{const node=document.querySelector('.task-details-content');return node.scrollHeight-node.clientHeight-node.scrollTop<2});
  const lastField = await region.locator('dd').last().boundingBox();
  const scrollBox = await region.boundingBox();
  check(lastField.y+lastField.height<=scrollBox.y+scrollBox.height+1, 'last detail field inaccessible');
  check(await longDialog.getByRole('button', {name:'关闭',exact:true}).isVisible(), 'close control clipped');
  await screenshot('long-details-1024');
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.activeElement?.getAttribute('data-media-cover')==='thumbnail');
  await page.getByRole('button', {name:'关于软件',exact:true}).click();
  await page.getByRole('button', {name:'检查更新',exact:true}).click();
  const checklist = page.locator('.release-notes .task-list-item');
  check(await checklist.count()===2, 'release checklist fixture missing');
  check(await checklist.evaluateAll(nodes=>nodes.every(node=>getComputedStyle(node).borderTopWidth==='0px')), 'task row separators leak into release notes');
  await page.keyboard.press('Escape');
  results.push('1024: long detail keyboard scrolling and final field/close access; release-note checklist isolation');
  check(errors.length===0, 'browser exceptions: '+errors.join(';'));
  return {passed:results.length, cases:results};
}
