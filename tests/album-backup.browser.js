// IPC-only fixture. Actual archive and provider bytes are covered by Rust tests.
async page => {
  page.setDefaultTimeout(10000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const results = [];
  for (const [width, height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-backup');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    const trigger = page.getByRole('button',{name:'查看图集',exact:true});
    await trigger.click();
    const dialog = page.locator('.album-dialog'); await dialog.waitFor();
    check(!(await dialog.innerText()).includes('1970') && !(await dialog.innerText()).includes('0 B'), 'remote details invented task values');
    await page.waitForFunction(() => document.querySelector('.album-dialog video')?.currentTime > 0.1);
    check(await dialog.getByRole('button',{name:'查看第 2 张图片',exact:true}).count() === 1,'manifest slides missing');
    await page.keyboard.press('ArrowRight');
    await dialog.locator('.album-still:not([aria-hidden]) img').last().evaluate(image=>image.decode());
    check(await dialog.locator('video').count()===0,'static slide retained live video');
    const box=await dialog.boundingBox();
    check(box.x>=-1 && box.y>=-1 && box.x+box.width<=width+1 && box.y+box.height<=height+1,'dialog overflow');
    check(await page.evaluate(()=>window.backgroundTest.calls.some(c=>c.type==='restore-album' && c.id==='album-cloud')),'missing restore request');
    check(await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='album-video'||c.type==='album-image').every(c=>c.id==='album-cloud')),'wrong album identity');
    await page.keyboard.press('Escape'); await dialog.waitFor({state:'detached'});
    results.push(`${width}x${height}: restore, original images, live video, keyboard and bounds`);
  }
  await page.evaluate(()=>window.backgroundTest.restoreFailure=true);
  await page.getByRole('button',{name:'查看图集',exact:true}).click();
  const dialog=page.locator('.album-dialog');
  const failure = dialog.getByRole('alert').filter({hasText:'远端内容校验失败'});
  await failure.waitFor();
  check(await failure.innerText() !== '', 'restore failure disappeared');
  check(await dialog.locator('video').count()===0,'failed restore opened video');
  results.push('restore failure remains visible');
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-backup-task');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  const feed=page.getByRole('button',{name:'切换为信息流',exact:true});
  if(await feed.count())await feed.click();
  await page.getByRole('button',{name:'重试下载',exact:true}).click();
  await page.locator('.album-dialog').waitFor();
  await page.waitForFunction(()=>document.querySelector('.album-dialog video')?.currentTime>0.1);
  check(await page.evaluate(()=>window.backgroundTest.calls.some(c=>c.type==='restore-album'&&c.id==='album-cloud')),'existing task retry did not restore album');
  results.push('existing download task retries failed album restore from feed');
  return results;
}
