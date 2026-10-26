// Real browser decoding/seeking with tiny generated media; all native IPC mocked.
async page => {
  page.setDefaultTimeout(8000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const row=id=>page.locator(`[data-task-id="visual-${id}"]`);
  const dialog=()=>page.locator('.video-player-dialog');
  const video=()=>dialog().locator('video');
  const externalCalls=()=>page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='play'));
  const load=async()=>{
    await page.evaluate(()=>localStorage.removeItem('yingqu.task-view'));
    await page.evaluate(()=>{for(const key of Object.keys(localStorage))if(key.startsWith('yingqu.playback-position.'))localStorage.removeItem(key)});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
  };
  const open=async(id=3)=>{await row(id).locator('.task-play').click();await dialog().waitFor()};
  const close=async()=>{await page.keyboard.press('Escape');await dialog().waitFor({state:'detached'})};
  const ready=()=>page.waitForFunction(()=>document.querySelector('.video-player-dialog video')?.readyState>=2);
  const override=async mode=>page.evaluate(async mode=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    api.playbackSource=async id=>{
      window.backgroundTest.calls.push({type:'prepare-playback',id});
      if(mode==='prepare-error')throw 'FILE_MISSING';
      if(mode==='hold')return await new Promise(resolve=>window.releaseVideoSource=resolve);
      return mode==='audio'?'/tests/fixtures/media/audio-only.webm':'/tests/fixtures/media/unsupported.webm';
    };
  },mode);
  await page.route('**/unsupported.webm',route=>route.fulfill({status:200,contentType:'video/webm',body:'not a playable video'}));
  const mediaRequests=[];
  page.on('request',request=>{if(request.url().endsWith('/player.webm'))mediaRequests.push(request.headers().range)});
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});await load();
    const snapshot=await page.evaluate(()=>JSON.stringify(window.backgroundTest.snapshot()));
    await open();await ready();
    const media=await video().evaluate(async v=>{
      await v.play();v.pause();v.currentTime=1.5;v.volume=0.3;
      return {width:v.videoWidth,height:v.videoHeight,controls:v.controls,fit:getComputedStyle(v).objectFit};
    });
    await page.waitForFunction(()=>{const v=document.querySelector('.video-player-dialog video');return v?.currentTime>=1.4&&!v.seeking});
    check(media.width===160&&media.height===90&&media.controls&&media.fit==='contain','real video not decoded or controls/fit missing');
    check(await video().evaluate(v=>v.paused&&Math.abs(v.currentTime-1.5)<0.1&&Math.abs(v.volume-0.3)<0.01),'seek/pause/volume failed');
    check((await externalCalls()).length===0,'playable video launched external player');
    check(mediaRequests.some(value=>value?.startsWith('bytes=')),'browser media did not request byte ranges');
    const bounds=await dialog().boundingBox();
    check(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=width&&bounds.y+bounds.height<=height,'player escaped viewport');
    await video().evaluate(v=>window.closedVideo=v);
    await page.addStyleTag({content:'.video-player-dialog[data-state="closed"] {animation-duration:600ms!important}'});
    await page.keyboard.press('Escape');await page.locator('.video-player-dialog[data-state="closed"]').waitFor({state:'attached'});
    check(await page.evaluate(()=>window.closedVideo.paused&&!window.closedVideo.hasAttribute('src')),'closing animation retained active media');
    check(Math.abs((await dialog().boundingBox()).height-bounds.height)<1,'player collapsed during exit animation');
    await dialog().waitFor({state:'detached'});
    check(await page.evaluate(()=>window.closedVideo.paused&&!window.closedVideo.hasAttribute('src')),'closed player retained media source');
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-play'));
    check(await page.evaluate(()=>JSON.stringify(window.backgroundTest.snapshot()))===snapshot,'playing mutated task or file metadata');
    await open();await ready();await dialog().evaluate(async node=>{await Promise.all(node.getAnimations().map(animation=>animation.finished.catch(()=>{})))});await page.screenshot({path:`.local/internal-player/player-${width}.png`});await close();
    results.push(`${width}: real decode, seek/pause/volume, native controls, fit/bounds, no external call, close release and return focus`);
  }
  for(const mode of ['unsupported','audio','prepare-error']){
    await load();await override(mode);await open();await dialog().waitFor({state:'detached'});
    check((await externalCalls()).length===1&&(await externalCalls())[0].id==='visual-3','fallback missing/duplicated/wrong task: '+mode);
    await page.locator('[data-sonner-toast]').filter({hasText:'内置播放不可用，已使用系统播放器打开'}).waitFor();
    results.push(mode+': automatically opens system player exactly once');
  }
  await load();await open();await ready();
  await dialog().getByRole('button',{name:'使用系统播放器',exact:true}).click();
  await dialog().waitFor({state:'detached'});
  check((await externalCalls()).length===1,'manual external playback failed');
  results.push('manual system player option remains available');
  await load();await override('unsupported');
  await page.evaluate(async()=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    api.play=async id=>{window.backgroundTest.calls.push({type:'play',id});throw 'PLAYBACK_OPEN_FAILED'};
  });
  await open();await dialog().getByRole('alert').waitFor();
  check((await externalCalls()).length===1,'failed external player retried automatically');
  check((await dialog().getByRole('alert').innerText()).includes('无法启动系统默认播放器'),'external failure not retained inline');
  await dialog().getByRole('button',{name:'使用系统播放器',exact:true}).click();
  await page.waitForFunction(()=>window.backgroundTest.calls.filter(c=>c.type==='play').length===2);
  await close();results.push('external failure persists in dialog; explicit retry works without automatic loop');
  await load();await override('hold');await open();
  await page.waitForFunction(()=>typeof window.releaseVideoSource==='function');await close();
  await page.evaluate(()=>window.releaseVideoSource('/tests/fixtures/media/unsupported.webm'));
  await page.waitForTimeout(100);
  check((await externalCalls()).length===0&&await dialog().count()===0,'late source after close opened player');
  results.push('closing during source preparation ignores late result and prevents fallback');
  await load();await override('unsupported');
  await page.evaluate(async()=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    api.play=async id=>{window.backgroundTest.calls.push({type:'play',id});await new Promise(resolve=>window.releaseExternal=resolve)};
  });
  await open();await page.waitForFunction(()=>typeof window.releaseExternal==='function');
  await video().dispatchEvent('error');await video().dispatchEvent('loadedmetadata');
  check((await externalCalls()).length===1,'multiple media events duplicated fallback');
  await close();
  await page.evaluate(async()=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    api.playbackSource=async id=>{window.backgroundTest.calls.push({type:'prepare-playback',id});return '/tests/fixtures/media/player.webm'};
  });
  await open(9);await ready();await page.evaluate(()=>window.releaseExternal());
  await page.waitForTimeout(50);
  check(await dialog().isVisible()&&await video().evaluate(v=>v.videoWidth===160),'old external completion closed the new video');
  await close();results.push('fallback deduplicates media events; old completion does not close a newer playback session');
  await load();await open();await ready();
  await video().evaluate(v=>window.removedVideo=v);
  await page.evaluate(async()=>{
    const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
    await api.remove('visual-3');
  });
  await dialog().waitFor({state:'detached'});
  check(await page.evaluate(()=>window.removedVideo.paused&&!window.removedVideo.hasAttribute('src')) && (await externalCalls()).length===0,'removed task retained playback or triggered external launch');
  results.push('removed task closes player and releases source without external launch');
  return {passed:results.length,results};
}
