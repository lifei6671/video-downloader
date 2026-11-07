// Isolated IPC fixtures: no system player or real user files are opened.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const row = id => page.locator(`[data-task-id="${id}"]`);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const plays = () => page.evaluate(() => window.backgroundTest.calls.filter(c=>c.type==='prepare-playback'));
  const playing = () => page.waitForFunction(() => {
    const element = document.querySelector('.task-media-dialog video');
    return element?.autoplay && element.readyState >= 2 && !element.paused && element.currentTime > 0.05;
  });
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
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='prepare-playback').length===1);
    check((await plays())[0].id==='done', 'double click played wrong task');
    await page.locator('.video-player-dialog').waitFor();await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
    for (const id of ['album','missing','live','failed'])
      await row(id).dblclick({position:{x:8,y:8}});
    check(await row('cancel').count() === 0, 'canceled task should not be playable from the list');
    check((await plays()).length===1, 'unsupported task played');
    await row('done').getByRole('button',{name:'播放视频',exact:true}).dispatchEvent('dblclick');
    await row('done').getByRole('button',{name:'查看封面',exact:true}).dispatchEvent('dblclick');
    check((await plays()).length===1, 'nested control bubbled into row playback');
    await row('done').locator('.task-row-title').click();
    check(await page.getByRole('dialog').count()===0, 'title single click opens a dialog');
    await row('done').locator('.task-row-title').dblclick();
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='prepare-playback').length===2);
    await page.locator('.video-player-dialog').waitFor();await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
    await row('done').getByRole('button',{name:'查看封面',exact:true}).click();
    const dialog = page.getByRole('dialog',{name:'任务详情'});
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='prepare-playback').length===3);
    const video = dialog.locator('video');
    await playing();
    await dialog.getByRole('heading',{name:'可播放视频',exact:true}).dblclick();
    check((await plays()).length===3, 'portal dialog started another playback');
    check(await dialog.getByRole('region',{name:'任务详细信息'}).isVisible(), 'cover playback left the detail dialog');
    await video.evaluate(element=>element.pause());
    await dialog.getByRole('button',{name:'播放视频',exact:true}).press('Enter');
    await page.waitForFunction(()=>document.querySelector('.task-media-dialog video')?.paused === false);
    await page.keyboard.press('Escape');
    await row('done').getByRole('button',{name:'播放视频',exact:true}).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='prepare-playback').length===4);
    await playing();
    await page.locator('.video-player-dialog').waitFor();await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
    await page.getByRole('button',{name:'关于软件',exact:true}).click();
    const badge = page.locator('[data-slot="card-footer"] [data-slot="badge"]').filter({hasText:/^Telegram$/});
    check(await badge.isVisible(), 'Telegram badge missing in brand footer');
    check(await badge.getAttribute('title')==='Telegram 实验模式', 'experimental context missing');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth), 'about page overflows');
    await page.screenshot({path:`output/task-double-play/about-${width}.png`});
    results.push(`${width}: cover and play button both autoplay in details, pause and keyboard resume, excluded states`);
  }
  await load('actions-remove-pending');
  await row('done').click({button:'right',position:{x:8,y:8}});
  await page.getByRole('menuitem',{name:'从列表移除',exact:true}).click();
  await page.waitForFunction(()=>typeof window.backgroundTest.release === 'function');
  await row('done').dblclick({position:{x:8,y:8}});
  check((await plays()).length===0, 'busy row played');
  await page.evaluate(()=>window.backgroundTest.release());
  await row('done').waitFor({state:'hidden'});
  await load('actions-error');
  await row('done').dblclick({position:{x:8,y:8}});
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'无法启动系统默认播放器'}).waitFor();
  check(await row('done').isVisible(), 'player failure removed task');
  results.push('busy prevents playback; system player errors use existing feedback');
  await load('unified-card-status');
  await page.evaluate(async () => {
    const moduleUrl = performance.getEntriesByType('resource').find(entry=>entry.name.includes('/src/api.ts')).name;
    const {api} = await import(moduleUrl);
    const tasks = await api.tasks();
    const task = tasks.find(item=>item.id === 'restore');
    Object.assign(task, {revision:task.revision+1, file_missing:true, path:null});
    api.tasks = async () => structuredClone(tasks);
    const asset = window.unifiedTest.assets.find(item=>item.asset_id === task.asset_id);
    Object.assign(asset, {local_state:'missing', path:null});
    window.coverRestoreCalls = [];
    api.assetPlaybackSource = async id => {
      window.coverRestoreCalls.push(id);
      return {path:'C:/TestVideos/restore.mp4', source:'/tests/fixtures/media/player.webm'};
    };
    window.dispatchEvent(new Event('focus'));
  });
  const restoreRow = row('restore');
  await restoreRow.getByText('文件不存在',{exact:true}).waitFor();
  const cover = restoreRow.getByRole('button',{name:'查看封面',exact:true});
  await cover.click();
  const detail = page.getByRole('dialog',{name:'任务详情',exact:true});
  await playing();
  check(await detail.getByRole('region',{name:'任务详细信息'}).isVisible(), 'backup autoplay left the detail dialog');
  check(await page.evaluate(()=>window.coverRestoreCalls.join(',') === 'asset-restore'), 'cover playback prepared the wrong backup');
  await page.keyboard.press('Escape');
  await detail.waitFor({state:'detached'});
  check(await cover.evaluate(node=>node === document.activeElement), 'restored playback lost cover focus');
  results.push('recoverable cover automatically restores and plays in the same dialog');
  return {passed:results.length,results};
}
