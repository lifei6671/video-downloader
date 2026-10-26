// Run through Playwright CLI against the isolated browser fixture, never user data.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const nav = name => page.getByRole('button', {name, exact:true});
  const output = 'output/playwright/visual-redesign/';
  const results = [];
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const load = async scenario => {
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=' + scenario);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await page.getByRole('button', {name:'后台下载', exact:true}).waitFor();
  };
  for (const [width,height,stage] of [[1440,900,360],[1280,800,320],[1024,680,300]]) {
    await page.setViewportSize({width,height});
    await load('visual');
    const cover = page.locator('[data-media-cover="preview"]');
    await cover.locator('.media-cover-image').evaluate(image => image.decode());
    check(Math.abs((await cover.boundingBox()).height - stage) < 1, 'stage height ' + width);
    check(await cover.locator('.media-cover-image').evaluate(image => getComputedStyle(image).objectFit === 'contain' && image.naturalHeight > image.naturalWidth), 'portrait image ratio');
    check(await page.locator('#video-input').evaluate(node => node.clientHeight >= 78 && node.clientHeight <= 80), 'three line input');
    await page.screenshot({path:output+'home-'+width+'.png',animations:'disabled'});
    await cover.click();
    await page.getByRole('dialog').getByRole('img', {name:'封面大图'}).waitFor();
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => document.activeElement?.getAttribute('data-media-cover') === 'preview');
    check(await cover.evaluate(node => node === document.activeElement), 'cover focus restore');
    await nav('下载任务').click();
    const rows = page.locator('.task-row');
    check(await rows.count() === 30 && await page.locator('[data-task-id="visual-0"]').count() === 0, 'initial page or preview exclusion changed');
    const ordinary = page.locator('[data-task-id="visual-3"]');
    check(Math.abs((await ordinary.boundingBox()).height - 80) < 1, 'media row height');
    check(await ordinary.getByRole('button').count() >= 3, 'media row actions');
    const live = page.locator('[data-task-id="visual-1"]');
    check((await live.boundingBox()).height >= 120, 'active row clips progress');
    check((await live.innerText()).includes('预计剩余 17 秒'), 'real ETA missing');
    const missing = page.locator('[data-task-id="visual-5"]');
    check(await missing.getByRole('button', {name:'播放视频',exact:true}).count() === 0, 'missing file plays');
    await missing.click({button:'right',position:{x:200,y:30}});
    check(await page.getByRole('menuitem', {name:'打开文件夹',exact:true}).count() === 0, 'missing file folder exposed');
    check(await page.getByRole('menuitem', {name:'从列表移除',exact:true}).isVisible(), 'missing file cannot be removed');
    await page.keyboard.press('Escape');
    const list = page.getByRole('tabpanel');
    const before = await page.getByRole('tablist').boundingBox();
    await list.evaluate(node => {node.scrollTop=400;});
    check(Math.abs((await page.getByRole('tablist').boundingBox()).y-before.y)<1, 'tabs scroll with list');
    await list.evaluate(node => {node.scrollTop=0;});
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'horizontal overflow '+width);
    await page.screenshot({path:output+'tasks-'+width+'.png',animations:'disabled'});
    await nav('系统设置').click();
    await nav('平台账号').click();
    await page.getByRole('tab', {name:'YouTube',exact:true}).click();
    check(await page.locator('[data-slot="toggle-group-item"][data-state="on"]').evaluateAll(nodes=>nodes.every(node=>{const other=node.parentElement.querySelector('[data-state="off"]');return !other || getComputedStyle(node).backgroundColor!==getComputedStyle(other).backgroundColor})), 'setting selection visually identical');
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'settings overflow '+width);
    await page.screenshot({path:output+'settings-'+width+'.png',animations:'disabled'});
    await nav('运行依赖').click();
    await page.getByText('语音转录模型 Fun-ASR-Nano', {exact:true}).waitFor();
    await nav('关于软件').click();
    check(await page.getByRole('heading',{name:'映取',exact:true}).evaluate(node => getComputedStyle(node).fontSize==='20px'), 'brand typography');
    const support=await page.locator('.project-support-link').evaluateAll(nodes=>nodes.map(node=>{const box=node.getBoundingClientRect();return {top:box.top,bottom:box.bottom,height:box.height,scroll:node.scrollHeight,client:node.clientHeight}}));
    check(support.every((box,i)=>box.height>=56 && box.scroll<=box.client+1 && (!i || box.top>=support[i-1].bottom)), 'multiline support links overlap');
    await page.screenshot({path:output+'about-'+width+'.png',animations:'disabled'});
    await nav('检查更新').click();
    const dialog = page.getByRole('dialog');
    check(Math.abs((await dialog.boundingBox()).width-640)<1, 'update dialog width');
    const button = dialog.getByRole('button',{name:'下载更新',exact:true});
    const bounds = await button.boundingBox();
    check(bounds.y+ bounds.height<=height, 'dialog footer not reachable');
    await page.screenshot({path:output+'update-'+width+'.png',animations:'disabled'});
    await page.keyboard.press('Escape');
    results.push(width+': home/task/settings/about/update/scroll/focus');
  }
  for (const ratio of ['landscape','square']) {
    await load('visual-'+ratio);
    const image=page.locator('.media-cover-image');
    await image.evaluate(node=>node.decode());
    check(await image.evaluate((node,shape)=>getComputedStyle(node).objectFit==='contain' && (shape==='square' ? node.naturalWidth===node.naturalHeight : node.naturalWidth>node.naturalHeight),ratio), 'cover proportion '+ratio);
    results.push(ratio+' natural cover');
  }
  await load('visual-album');
  await page.getByRole('button',{name:'立即下载',exact:true}).waitFor();
  check(await page.locator('.album-stage').isVisible(), 'album stage');
  check(await page.locator('.album-thumbnail[aria-pressed="true"] .album-selection').count()===1, 'album selection indicator missing');
  await page.getByRole('button',{name:'查看第 2 张图片',exact:true}).click();
  check(await page.getByRole('button',{name:'查看第 2 张图片',exact:true}).getAttribute('aria-pressed')==='true', 'album selection did not change');
  check(await page.locator('.album-thumbnail[aria-pressed="true"] .album-selection').count()===1, 'album indicator did not move');
  await page.emulateMedia({reducedMotion:'reduce'});
  await load('visual');
  check(await page.locator('[data-slot="button"]').first().evaluate(node=>getComputedStyle(node).transitionDuration==='0s'), 'reduced motion');
  await page.emulateMedia({reducedMotion:'no-preference'});
  for (const [width,height] of [[1440,900],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=author-results');
    await nav('下载任务').click();
    await page.locator('[data-task-id="first"]').getByRole('button',{name:'查看作者昵称的作品',exact:true}).click();
    const ready=page.locator('[data-task-id="second"]');
    const controls=await ready.locator('.task-row-controls').boundingBox();
    const actions=await ready.locator('.task-actions').getByRole('button').evaluateAll(nodes=>nodes.map(node=>{const b=node.getBoundingClientRect();return {x:b.x,right:b.right}}));
    check(actions.every(b=>b.x>=controls.x-1 && b.right<=controls.x+controls.width+1), 'ready author-result actions overflow');
    results.push(width+': author ready row actions wrap');
  }
  check(errors.length===0,errors.join('\n'));
  return {passed:results,errors};
}
