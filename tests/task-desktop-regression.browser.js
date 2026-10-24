// Real React UI, isolated IPC fixture. No system player, user files or downloads.
async page => {
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (value, message) => { if (!value) throw Error(message); };
  const row = id => page.locator(`[data-task-id="visual-${id}"]`);
  const choose = name => page.getByRole('radiogroup', {name:'按状态筛选下载任务'}).getByRole('radio', {name, exact:true}).click();
  const results = [];
  for (const [width, height] of [[1440,900], [1280,800], [1024,680]]) {
    await page.setViewportSize({width, height});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.getByRole('button', {name:'下载任务', exact:true}).click();
    await row(3).waitFor();
    check(await page.getByRole('heading', {name:'下载任务', exact:true}).isVisible(), 'visible page title lost');
    check((await row(2).boundingBox()).height===84, 'queued row height');
    const info = await row(3).evaluate(n=>{
      const title=n.querySelector('.task-row-heading').getBoundingClientRect();
      const byline=n.querySelector('.task-row-byline').getBoundingClientRect();
      return {gap:byline.top-title.bottom,height:byline.height};
    });
    check(info.gap===6 && info.height<=20, 'normal metadata is no longer one compact second line');
    await row(3).click({button:'right',position:{x:200,y:30}});
    const menu=page.getByRole('menu');
    await menu.waitFor();
    check((await menu.boundingBox()).width===236, 'menu width');
    check(await menu.getByRole('menuitem').evaluateAll(nodes=>nodes.every(n=>{const h=n.getBoundingClientRect().height;return h>=34&&h<=38})), 'menu item density');
    await page.keyboard.press('Escape');
    await choose('下载中');
    await page.evaluate(()=>window.backgroundTest.progress({bytes:125829120,total_bytes:167772160,speed_bps:5242880}));
    await page.waitForFunction(()=>document.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')==='75');
    await page.getByRole('button', {name:'关于软件', exact:true}).click();
    await page.evaluate(()=>window.backgroundTest.complete('visual-1'));
    await page.getByRole('button', {name:'下载任务', exact:true}).click();
    check(await row(1).count()===0, 'completed event still shown in downloading filter');
    await choose('已完成');
    await row(1).getByRole('button', {name:'播放视频', exact:true}).waitFor();
    await row(6).getByRole('button', {name:'查看图集', exact:true}).click();
    const gallery=page.getByRole('dialog');
    await gallery.waitFor();
    await gallery.locator('.album-stage').getByRole('img', {name:'第 1 张图片', exact:true}).waitFor();
    await page.keyboard.press('Escape');
    await choose('等待中');
    await row(2).getByRole('button', {name:'取消', exact:true}).click();
    await page.getByRole('alertdialog').getByRole('button', {name:'确认取消', exact:true}).click();
    await row(2).waitFor({state:'hidden'});
    await choose('全部');
    await row(2).getByText('已取消', {exact:true}).waitFor();
    await choose('失败');
    await row(4).getByRole('button', {name:'重试', exact:true}).click();
    await row(4).waitFor({state:'hidden'});
    await choose('下载中');
    await row(4).getByRole('progressbar').waitFor();
    const calls=await page.evaluate(()=>window.backgroundTest.calls);
    check(calls.filter(c=>c.type==='retry'&&c.id==='visual-4').length===1, 'retry dispatch');
    check(calls.filter(c=>c.type==='cancel'&&c.id==='visual-2').length===1, 'cancel dispatch');
    results.push(`${width}: two-level metadata, queue/menu geometry, progress/completion events across navigation, album, cancel and retry`);
  }
  return {passed:results.length,cases:results};
}
