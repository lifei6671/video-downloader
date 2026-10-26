// Actual browser media and isolated preference storage; native IPC remains mocked.
async page => {
  page.setDefaultTimeout(8000);
  const base=page.url().split('/').slice(0,3).join('/');
  const key=id=>'yingqu.playback-position.visual-'+id;
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const video=()=>page.locator('.video-player-dialog video');
  const dialog=()=>page.locator('.video-player-dialog');
  const entry=id=>page.evaluate(key=>JSON.parse(localStorage.getItem(key)??'null'),key(id));
  const tasks=()=>page.getByRole('button',{name:'下载任务',exact:true}).click();
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.evaluate(()=>{for(const key of Object.keys(localStorage))if(key.startsWith('yingqu.playback-position.'))localStorage.removeItem(key);localStorage.removeItem('yingqu.task-view')});
    await page.reload();await tasks();
  };
  const open=async(id=3)=>{
    await page.locator(`[data-task-id="visual-${id}"] .task-play`).click();
    await page.waitForFunction(()=>document.querySelector('.video-player-dialog video')?.readyState>=2);
  };
  const close=async()=>{await page.keyboard.press('Escape');await dialog().waitFor({state:'detached'})};
  const seek=async seconds=>{
    await video().evaluate((v,seconds)=>{v.pause();v.currentTime=seconds},seconds);
    await page.waitForFunction(seconds=>{const v=document.querySelector('.video-player-dialog video');return v&&!v.seeking&&Math.abs(v.currentTime-seconds)<0.05},seconds);
  };
  const near=async seconds=>{
    const current=await video().evaluate(v=>{v.pause();return v.currentTime});
    check(current>=seconds-0.1&&current<seconds+0.5,'wrong restored position '+current+' expected '+seconds);
  };
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});await fresh();await open();await seek(1.25);
    check(Math.abs((await entry(3)).seconds-1.25)<0.05,'seek did not persist immediately');
    await close();await open();await near(1.25);await close();
    await page.reload();await tasks();await open();await near(1.25);await close();
    await open(9);await near(0);await seek(0.7);await close();
    await open();await near(1.25);await close();
    check(Math.abs((await entry(9)).seconds-0.7)<0.05,'different videos share position');
    results.push(`${width}: seek/pause/close flush, reopen/app reload restore, independent videos`);
  }
  await fresh();await open();await seek(1.3);
  await video().evaluate(v=>{v.playbackRate=0.1});
  await page.evaluate(()=>{
    window.positionWrites=0;const original=Storage.prototype.setItem;
    Storage.prototype.setItem=function(key,value){if(key.startsWith('yingqu.playback-position.'))window.positionWrites++;return original.call(this,key,value)};
  });
  await video().evaluate(v=>v.play());await page.waitForTimeout(5700);
  const writes=await page.evaluate(()=>window.positionWrites);
  check(writes>=1&&writes<=2&&(await entry(3)).seconds>1.6,'continuous playback is not saved/throttled: '+writes);
  const beforeReload=await video().evaluate(v=>v.currentTime);
  await page.reload();await tasks();await open();await near(beforeReload);await close();
  results.push('continuous playback checkpoints are throttled; pagehide flushes latest position');
  await fresh();await open();await seek(3.7);
  await video().evaluate(async v=>{v.playbackRate=2;await v.play()});
  await page.waitForFunction(()=>document.querySelector('.video-player-dialog video')?.ended===true);
  check(await entry(3)===null,'finished video retains resume point');
  await close();await open();await near(0);await close();
  await open();await seek(1.2);await seek(0);await close();
  check(await entry(3)===null,'rewinding to start did not clear position');
  results.push('completed video and explicit rewind start from zero next time');
  for(const invalid of ['not-json','outside-duration','different-generation']){
    await fresh();
    await page.evaluate(({key,invalid})=>{
      const task=window.backgroundTest.snapshot().find(t=>t.id==='visual-3');
      localStorage.setItem(key,invalid==='not-json'?'not-json':JSON.stringify({path:task.path,completedAt:invalid==='different-generation'?task.completed_at+1:task.completed_at,seconds:invalid==='different-generation'?1.5:99}));
    },{key:key(3),invalid});
    await open();await near(0);await close();
  }
  await fresh();await open();await seek(1.2);await close();
  await page.evaluate(()=>window.backgroundTest.complete('visual-3'));
  await open();await near(0);await close();
  results.push('invalid, out-of-range, changed completion and changed path do not restore stale points');
  await fresh();await open();await seek(1.2);
  await page.evaluate(async()=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    await api.remove('visual-3');
  });
  await dialog().waitFor({state:'detached'});
  check(await entry(3)===null,'removed task progress was recreated by player cleanup');
  results.push('removal clears progress and cleanup cannot write deleted records back');
  await fresh();await open();
  await page.evaluate(()=>{
    window.progressWriteAttempts=0;window.originalProgressSet=Storage.prototype.setItem;
    Storage.prototype.setItem=function(key,value){if(key.startsWith('yingqu.playback-position.')){window.progressWriteAttempts++;throw new DOMException('Storage full','QuotaExceededError')}return window.originalProgressSet.call(this,key,value)};
  });
  await seek(1.1);
  await page.locator('[data-sonner-toast]').filter({hasText:'无法保存播放进度'}).waitFor();
  await video().evaluate(async v=>{await v.play();v.pause();v.dispatchEvent(new Event('timeupdate'))});
  check(await page.evaluate(()=>window.progressWriteAttempts)===1&&await video().evaluate(v=>v.videoWidth===160),'storage failure repeats or breaks player');
  await page.evaluate(()=>{Storage.prototype.setItem=window.originalProgressSet});await close();
  await open();await seek(1.4);check((await entry(3)).seconds>1.3,'new playback session cannot save after storage recovers');await close();
  results.push('storage error warns once without breaking video; new session can retry saving');
  return {passed:results.length,results};
}
