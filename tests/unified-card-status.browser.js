// Real App/TaskRow components with isolated IPC. No real account or media writes.
async page => {
  page.setDefaultTimeout(10000);
  const base=page.url().split('/').slice(0,3).join('/');
  const results=[];
  const check=(value,message)=>{if(!value)throw Error(message)};
  const row=id=>page.locator(`[data-task-id="${id}"]`);
  const assetRow=id=>page.locator(`[data-asset-id="${id}"]`);
  const expected={reconnecting:'等待连接','backup-reconnecting':'等待连接','restore-reconnecting':'等待连接',waiting:'等待中',download:'下载中',merge:'合并中',pending:'待备份',preparing:'准备备份',upload:'备份中',confirm:'确认备份',done:'已完成','download-failed':'下载失败',canceling:'正在取消',interrupted:'已中断',retry:'等待重试','backup-canceled':'备份已取消','backup-failed':'备份失败',unknown:'待确认',cleaning:'清理中','confirm-video':'确认备份',attachment:'备份中',foreign:'待备份',unregistered:'待备份','download-wins':'下载中',restore:'下载中',dependency:'待确认','old-operation':'已完成'};
  const waitLabel=async(id,label)=>{
    await page.waitForFunction(({id,label})=>document.querySelector(`[data-task-id="${id}"] .task-feed-status`)?.textContent===label,{id,label});
  };
  const load=async(mode='')=>{
    await page.evaluate(()=>localStorage.setItem('yingqu.task-view','feed'));
    await page.goto(`${base}/tests/fixtures/background-download.html?scenario=unified-card-status${mode?'-'+mode:''}`);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await waitLabel('pending',mode==='off'?'已完成':'待备份');
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});await load();
    for(const [id,label] of Object.entries(expected)){
      await waitLabel(id,label);
      check(await row(id).locator('.task-feed-status').count()===1,`${id} has duplicate corner badges`);
      check(await row(id).locator('.task-row-footer').count()===0,`${id} added a footer`);
    }
    const infoHeights=await page.locator('[data-task-id] .task-row-information').evaluateAll(nodes=>nodes.map(n=>n.getBoundingClientRect().height));
    check(Math.max(...infoHeights)-Math.min(...infoHeights)<1,'status changed fixed card information height');
    for(const [id,label] of [['cloud-only','已完成'],['remote-upload','备份中'],['cloud-unavailable','待备份']]){
      await assetRow(id).locator('.task-feed-status').filter({hasText:label}).waitFor();
      check(await assetRow(id).locator('.task-feed-status').count()===1,`${id} has duplicate asset badges`);
      check(await assetRow(id).locator('.task-row-footer').count()===0,`${id} added an asset footer`);
      const info=await assetRow(id).locator('.task-row-information').evaluate(n=>n.getBoundingClientRect().height);
      check(Math.abs(info-infoHeights[0])<1,`${id} asset information height differs from task cards`);
    }
    check(await assetRow('cloud-unavailable').locator('.task-feed-play').isDisabled(),'unavailable remote asset exposed playback');
    check(await assetRow('remote-upload').locator('.task-feed-play').getAttribute('aria-label')==='取消备份','unlinked upload lost its cancel action');
    const notice=row('download-failed').getByRole('alert');
    check(await notice.evaluate(n=>!!n.closest('.task-row-information .task-card-feedback')),'error moved away from the reserved metadata slot');
    check(await row('done').getByRole('alert').count()===0,'completed card inherited an unrelated error');
    for(const [id,label] of [['upload','取消备份'],['unknown','重试确认'],['backup-failed','重试备份'],['backup-canceled','重试备份']]){
      const button=row(id).getByRole('button',{name:label,exact:true}).filter({has:page.locator('svg')});
      const corner=row(id).locator('.task-feed-play');
      check(await corner.getAttribute('aria-label')===label,`${id} lost its matching corner action`);
      check(await button.count()>=1,`${id} action unavailable`);
      const b=await corner.boundingBox(),c=await row(id).locator('.task-row-cover').boundingBox();
      check(b.y<c.y+12&&b.x+b.width>c.x+c.width-12,`${id} action moved from top right`);
    }
    check(await page.evaluate(()=>window.unifiedTest.calls.length)===0,'rendering statuses issued a write command');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:`.local/unified-card-status/states-${width}.png`});
    results.push(`${width}: ${Object.keys(expected).length} task + 3 unlinked asset projections, one badge, fixed heights/error slot, matching top-right actions`);
  }
  await load();
  const before=await row('upload').evaluate(n=>n.getBoundingClientRect().height);
  await page.evaluate(async()=>{
    const {api}=await import('/src/api.ts');
    const read=api.backupOperations;
    const stale=structuredClone(window.unifiedTest.operations);
    let held=false;
    api.backupOperations=async()=>{
      if(!held){held=true;return new Promise(resolve=>window.releaseStaleOperations=()=>resolve(stale));}
      return read();
    };
    window.unifiedTest.holdCancel=true;
  });
  await page.waitForFunction(()=>!!window.releaseStaleOperations);
  await row('upload').locator('.task-feed-play').click();
  await waitLabel('upload','正在取消');
  check(await row('upload').locator('.task-feed-play').isDisabled(),'canceling remained clickable');
  check(await row('download').getByRole('button',{name:'取消下载',exact:true,includeHidden:true}).isEnabled(),'canceling backup disabled unrelated download');
  check(await page.evaluate(()=>window.unifiedTest.calls.filter(c=>c.type==='cancel').length)===1,'duplicate cancel commands');
  await page.evaluate(()=>window.unifiedTest.releaseCancel());
  await waitLabel('upload','清理中');
  await page.evaluate(()=>window.releaseStaleOperations());
  await page.waitForTimeout(150);
  check(await row('upload').locator('.task-feed-status').innerText()==='清理中','older poll response rolled back the canceled state');
  await page.mouse.move(0,0);
  await row('upload').locator('.task-feed-action-tooltip').hover();
  await page.getByRole('tooltip').filter({hasText:'正在安全清理前次操作'}).waitFor();
  check(await row('upload').locator('.task-feed-play').isDisabled(),'cleanup exposed an unsafe retransmit action');
  await page.evaluate(()=>window.unifiedTest.change('op-upload',{phase:'cancel_cleaned'}));
  await waitLabel('upload','备份已取消');
  check(Math.abs((await row('upload').evaluate(n=>n.getBoundingClientRect().height))-before)<1,'cancel/cleanup changed card height');
  results.push('cancel → cleanup → canceled; stale replies ignored; no extra write, no retransmit, unrelated task stays usable');
  const nonce=await page.evaluate(()=>window.unifiedTest.operations.find(o=>o.operation_id==='op-unknown').payload.operation_nonce);
  await row('unknown').locator('.task-feed-play').click();
  await waitLabel('unknown','待备份');
  check(await page.evaluate(()=>window.unifiedTest.calls.filter(c=>c.type==='retry'&&c.id==='op-unknown').length)===1,'receipt retry did not retain its operation');
  check(await page.evaluate(()=>window.unifiedTest.operations.find(o=>o.operation_id==='op-unknown').payload.operation_nonce)===nonce,'receipt retry replaced operation identity');
  check(await page.evaluate(()=>window.unifiedTest.calls.some(c=>c.type==='start'))===false,'receipt retry created a new upload');
  results.push('unknown receipt retry preserves exact operation and nonce instead of creating an upload');
  await row('backup-failed').locator('.task-feed-play').click();
  check(await page.evaluate(()=>window.unifiedTest.calls.filter(c=>c.type==='start'&&c.id==='asset-backup-failed'&&c.space==='status-space').length)===1,'source-changed retry did not recapture through the existing upload entry');
  check(await page.evaluate(()=>window.unifiedTest.calls.some(c=>c.type==='retry'&&c.id==='op-backup-failed'))===false,'source-changed retry reused a stale source capture');
  results.push('source-changed failure recaptures the source through startAssetBackup instead of retrying stale operation data');
  await page.evaluate(()=>window.unifiedTest.change('op-preparing',{phase:'captured',payload:{completed_bytes:50,transfer_started:true}}));
  await waitLabel('preparing','备份中');
  await page.evaluate(()=>window.unifiedTest.change('op-preparing',{phase:'publishing'}));
  await waitLabel('preparing','确认备份');
  await page.evaluate(()=>{
    window.unifiedTest.change('op-preparing',{state:'completed',phase:'completed'});
    Object.assign(window.unifiedTest.assets.find(a=>a.asset_id==='asset-preparing'),{remote_state:'backed_up',has_pending_local_changes:false});
  });
  await waitLabel('preparing','已完成');
  results.push('prepare → upload → publish confirmation → complete only after authoritative acknowledgement');
  await row('preparing').locator('.task-row-title').click({button:'right'});
  await page.getByRole('menuitem',{name:'播放视频',exact:true}).click();
  await page.locator('.video-player-dialog').waitFor();
  await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
  results.push('playback remains reachable from the task context menu');
  await assetRow('cloud-unavailable').locator('.task-row-title').click({button:'right'});
  check(await page.getByRole('menuitem',{name:'播放视频',exact:true}).count()===0,'unavailable cloud asset bypassed its playback guard through the menu');
  await page.keyboard.press('Escape');
  await assetRow('cloud-only').locator('.task-row-title').click({button:'right'});
  check(await page.getByRole('menuitem',{name:'播放视频',exact:true}).count()===1,'available cloud-only asset lost its playback menu');
  await page.keyboard.press('Escape');
  results.push('unlinked asset context playback follows the same availability guard as the original corner button');
  const pure=await page.evaluate(async()=>{
    const {taskCardStatus}=await import('/src/lib/task-card-status.ts');
    const task=window.backgroundTest.snapshot().find(t=>t.id==='done');
    const asset=window.unifiedTest.assets.find(a=>a.asset_id===task.asset_id);
    const get=overrides=>taskCardStatus({task,asset,spaceId:'status-space',backupEnabled:true,...overrides}).label;
    const completed={operation_id:'completed',space_id:'status-space',asset_id:asset.asset_id,kind:'upload',state:'completed',phase:'completed',payload:{cleanup_dependency:'obsolete'},last_error:null};
    return {
      unknown:get({backupEnabled:undefined}),
      pendingChange:get({asset:{...asset,has_pending_local_changes:true}}),
      foreign:get({asset:{...asset,space_id:'other-space'}}),
      oldDependency:get({operations:[completed]}),
      cleanedOldGeneration:get({operations:[completed,{...completed,operation_id:'old-deleted',state:'blocked',phase:'cancel_cleaned',last_error:'BACKUP_ASSET_DELETED'}]}),
      wait:get({waiting:true}),
      album:get({task:{...task,video:{...task.video,kind:'image_album'}},asset:undefined}),
    };
  });
  check(pure.unknown==='待备份'&&pure.pendingChange==='待备份'&&pure.foreign==='待备份','missing/dirty/other-space facts falsely completed');
  check(pure.oldDependency==='已完成'&&pure.cleanedOldGeneration==='已完成'&&pure.wait==='等待中'&&pure.album==='已完成','terminal, resource wait or supported media priority regressed');
  results.push('unknown configuration, dirty and foreign-space facts stay pending; obsolete cleanup cannot mask completion');
  await load('connecting');await waitLabel('done','待备份');
  check(await page.evaluate(()=>window.unifiedTest.assets.some(a=>a.asset_id==='asset-done'&&a.remote_state==='backed_up')),'connecting fixture lacks retained backup evidence');
  check(await page.evaluate(()=>window.unifiedTest.calls.length)===0,'restoring configuration created operations');
  await page.evaluate(()=>{window.unifiedTest.settings.enabled=true;window.unifiedTest.settings.connection_pending=false;});
  await waitLabel('done','已完成');await waitLabel('pending','待备份');
  check(await page.evaluate(()=>window.unifiedTest.calls.length)===0,'configuration recovery created a duplicate operation');
  results.push('provider configuration recovery: existing remote proof stays pending until ready, then completes automatically without writes');
  await load('off');await waitLabel('pending','已完成');await waitLabel('done','已完成');
  check(await page.evaluate(()=>window.unifiedTest.calls.length)===0,'disabled backup created operations');
  results.push('backup disabled: completed download is terminal without scheduling writes');
  return {passed:results.length,results};
}
