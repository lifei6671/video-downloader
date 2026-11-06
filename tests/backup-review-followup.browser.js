// Browser media decoding and scale checks; native IPC is isolated.
async page => {
  const base = page.url().split('/').slice(0,3).join('/');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=unified-card-status');
  await page.evaluate(() => {
    localStorage.removeItem('yingqu.playback-position.cloud-only');
    window.__TAURI_INTERNALS__ = {...window.__TAURI_INTERNALS__,
      convertFileSrc: () => '/tests/fixtures/media/player.webm',
      invoke: async command => {
        if (command === 'prepare_asset_playback') return 'C:/fixture/restored.mp4';
        throw Error('unexpected IPC: '+command);
      }};
  });
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  const row = page.locator('[data-asset-id="cloud-only"]');
  const video = page.locator('.video-player-dialog video');
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await row.getByRole('button',{name:'播放视频',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('.video-player-dialog video')?.readyState>=2);
    await video.evaluate(v=>{v.pause();v.currentTime=1.25});
    await page.waitForFunction(()=>JSON.parse(localStorage.getItem('yingqu.playback-position.cloud-only')??'null')?.seconds>=1.2);
    const saved = await page.evaluate(()=>JSON.parse(localStorage.getItem('yingqu.playback-position.cloud-only')));
    if (saved.path !== 'C:/fixture/restored.mp4') throw Error('restored session saved stale path');
    await page.keyboard.press('Escape'); await page.locator('.video-player-dialog').waitFor({state:'detached'});
    await page.evaluate(()=>Object.assign(window.unifiedTest.assets.find(a=>a.asset_id==='cloud-only'),{path:'C:/fixture/restored.mp4',local_state:'present'}));
    await page.waitForTimeout(1100);
    await row.getByRole('button',{name:'播放视频',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('.video-player-dialog video')?.readyState>=2);
    const position = await video.evaluate(v=>{v.pause();return v.currentTime});
    if (position<1.2 || position>2) throw Error('restored progress lost: '+position);
    await page.keyboard.press('Escape'); await page.locator('.video-player-dialog').waitFor({state:'detached'});
  }
  const timings = await page.evaluate(async()=>{
    const {indexBackup} = await import('/src/components/backup-tasks');
    const {taskCardStatus} = await import('/src/lib/task-card-status');
    return [5000,10000].map(size=>{
      const template=window.unifiedTest.assets[0];
      const assets=Array.from({length:size},(_,i)=>({...template,asset_id:'scale-'+i}));
      const ops=assets.map(a=>({operation_id:'op-'+a.asset_id,space_id:template.space_id,asset_id:a.asset_id,kind:'upload',state:'running',phase:'captured',payload:{},attempt:0,last_error:null}));
      const start=performance.now();
      const index=indexBackup(assets,ops);
      for(const asset of assets) {
        const status=taskCardStatus({asset,assetId:asset.asset_id,operations:index.operationsByAsset.get(asset.asset_id),spaceId:asset.space_id,backupEnabled:true});
        if(!status.label) throw Error('missing status');
      }
      return {size,elapsedMs:performance.now()-start};
    });
  });
  return {playback:'restored path persists across reopen and list refresh at three viewports',timings};
}
