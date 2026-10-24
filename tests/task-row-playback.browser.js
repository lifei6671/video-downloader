// Isolated IPC fixtures: no system player or real user files are opened.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const row = id => page.locator(`[data-task-id="${id}"]`);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const plays = () => page.evaluate(() => window.backgroundTest.calls.filter(c=>c.type==='play'));
  const load = async (scenario='actions') => {
    await page.goto(`${base}/tests/fixtures/background-download.html?scenario=${scenario}`);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await row('done').waitFor();
  };
  const results = [];
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await load();
    await row('done').click({position:{x:8,y:8}});
    check((await plays()).length===0, 'single click must only select');
    await row('done').dblclick({position:{x:8,y:8}});
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='play').length===1);
    check((await plays())[0].id==='done', 'double click played wrong task');
    for (const id of ['album','missing','live','failed','cancel'])
      await row(id).dblclick({position:{x:8,y:8}});
    check((await plays()).length===1, 'unsupported task played');
    await row('done').getByRole('button',{name:'播放视频',exact:true}).dispatchEvent('dblclick');
    await row('done').getByRole('button',{name:'查看封面',exact:true}).dispatchEvent('dblclick');
    check((await plays()).length===1, 'nested control bubbled into row playback');
    await row('done').locator('.task-row-title').click();
    check(await page.getByRole('dialog').count()===0, 'title single click opens a dialog');
    await row('done').locator('.task-row-title').dblclick();
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='play').length===2);
    await row('done').getByRole('button',{name:'查看封面',exact:true}).click();
    const dialog = page.getByRole('dialog',{name:'任务详情'});
    await dialog.getByRole('heading',{name:'任务详情',exact:true}).dblclick();
    check((await plays()).length===2, 'portal dialog bubbled into row playback');
    await page.keyboard.press('Escape');
    await row('done').getByRole('button',{name:'播放视频',exact:true}).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='play').length===3);
    await page.getByRole('button',{name:'关于软件',exact:true}).click();
    const badge = page.locator('[data-slot="card-footer"] [data-slot="badge"]').filter({hasText:/^Telegram$/});
    check(await badge.isVisible(), 'Telegram badge missing in brand footer');
    check(await badge.getAttribute('title')==='Telegram 实验模式', 'experimental context missing');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth), 'about page overflows');
    await page.screenshot({path:`output/task-double-play/about-${width}.png`});
    results.push(`${width}: row playback, excluded states/controls/dialog, keyboard play, Telegram badge`);
  }
  await load('actions-remove-pending');
  await row('album').click({button:'right',position:{x:8,y:8}});
  await page.getByRole('menuitem',{name:'从列表移除',exact:true}).click();
  await row('done').dblclick({position:{x:8,y:8}});
  check((await plays()).length===0, 'busy row played');
  await page.evaluate(()=>window.backgroundTest.release());
  await row('album').waitFor({state:'hidden'});
  await load('actions-error');
  await row('done').dblclick({position:{x:8,y:8}});
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'无法启动系统默认播放器'}).waitFor();
  check(await row('done').isVisible(), 'player failure removed task');
  results.push('busy prevents playback; system player errors use existing feedback');
  return {passed:results.length,results};
}
