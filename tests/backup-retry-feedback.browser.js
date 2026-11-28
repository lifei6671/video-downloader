// Full App with isolated IPC: never reads a real session or deletes cloud data.
async page => {
  page.setDefaultTimeout(10000);
  const base=page.url().split('/').slice(0,3).join('/'), results=[];
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const row=id=>page.locator(`[data-task-id="${id}"]`);
  const errors=()=>page.locator('[data-sonner-toast][data-type="error"]');
  const label=async(id,text)=>page.waitForFunction(({id,text})=>document.querySelector(`[data-task-id="${id}"] .task-feed-status`)?.textContent===text,{id,text});
  const load=async()=>{
    await page.evaluate(()=>localStorage.setItem('yingqu.task-view','feed'));
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=unified-card-status');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await label('retry','等待重试');
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});await load();
    const before=await row('retry').evaluate(n=>n.getBoundingClientRect().height);
    check(await row('retry').getByRole('alert').count()===0,'automatic retry displayed its previous error');
    await page.evaluate(()=>{
      const deadline=Math.floor(Date.now()/1000)+120;
      window.unifiedTest.change('op-upload',{state:'retry_wait',last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:deadline});
      window.unifiedTest.change('op-retry',{last_error:null,next_retry_at:deadline});
    });
    await row('upload').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
    await label('retry','等待重试');
    check(await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).count()===0,'account cooldown spread flood notice to another video');
    check(await row('pending').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).count()===0,'queued video displayed flood notice');
    await page.evaluate(()=>window.unifiedTest.change('op-upload',{state:'running',last_error:null,next_retry_at:null}));
    await row('upload').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
    await page.evaluate(()=>window.unifiedTest.change('op-retry',{last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:Math.floor(Date.now()/1000)+120}));
    await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
    check(await row('retry').getByRole('alert').count()===0 && await errors().count()===0,'flood cooldown was presented as a failure');
    await row('retry').getByRole('button',{name:'取消备份',exact:true}).waitFor();
    for (const patch of [
      {next_retry_at:Math.floor(Date.now()/1000)-1},
      {next_retry_at:null},
      {state:'running',next_retry_at:Math.floor(Date.now()/1000)+120},
      {state:'pending'},
      {last_error:'TELEGRAM_FLOOD_WAIT:0'},
    ]) {
      await page.evaluate(()=>window.unifiedTest.change('op-retry',{state:'retry_wait',last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:Math.floor(Date.now()/1000)+120}));
      await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
      await page.evaluate(patch=>window.unifiedTest.change('op-retry',patch),patch);
      await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
    }
    await page.evaluate(()=>window.unifiedTest.change('op-retry',{state:'retry_wait',last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:Date.now()/1000+2}));
    await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
    await row('retry').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
    for(const attempt of [2,3]){
      await page.evaluate(attempt=>window.unifiedTest.change('op-retry',{attempt,last_error:attempt===2?'TELEGRAM_TIMEOUT':'TELEGRAM_CONNECTION_ERROR'}),attempt);
      await page.waitForTimeout(1100);
      check(await row('retry').getByRole('alert').count()===0,'intermediate attempt displayed an error');
      check(await errors().count()===0,'automatic retry emitted an error Toast');
    }
    await page.evaluate(()=>window.unifiedTest.change('op-retry',{state:'blocked',attempt:5}));
    await label('retry','备份失败');
    await row('retry').getByRole('alert').waitFor();
    const icon=row('retry').locator('.backup-sync-glyph');
    check(await icon.locator('[role="progressbar"]').count()===0,'final failure kept a progress spinner');
    check(await icon.locator('svg.lucide-circle-alert').count()===1,'final failure has no static error icon');
    await errors().waitFor();
    check(await errors().count()===1,'final failure Toast was missing or duplicated');
    await page.waitForTimeout(1200);
    check(await errors().count()===1,'unchanged terminal failure repeated its Toast');
    check(Math.abs((await row('retry').evaluate(n=>n.getBoundingClientRect().height))-before)<1,'error changed card height');
    await row('retry').locator('.backup-sync-button').click();
    await label('retry','待备份');
    check(await row('retry').getByRole('alert').count()===0,'explicit retry retained the old inline failure');
    check(await page.evaluate(()=>window.unifiedTest.calls.filter(c=>c.type==='retry'&&c.id==='op-retry').length)===1,'title retry did not preserve its operation');
    results.push(`${width}: automatic retries quiet; terminal error static and visible once; title retries original operation; fixed height`);
  }
  await load();
  const nonce=await page.evaluate(()=>window.unifiedTest.operations.find(o=>o.operation_id==='op-upload').payload.operation_nonce);
  await row('upload').locator('.task-row-title').click({button:'right'});
  await page.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).click();
  await label('upload','删除中');
  check(await row('upload').getByRole('alert').count()===0,'delete intent exposed its stopped upload as failure');
  check(await errors().count()===0,'delete intent emitted an upload-failure Toast');
  await page.evaluate(()=>window.unifiedTest.finishDelete('asset-upload'));
  await label('upload','删除完成');
  check(await errors().count()===0,'completed deletion emitted BACKUP_ASSET_DELETED');
  check(await row('upload').getByRole('alert').count()===0,'completed deletion kept an upload error');
  await page.evaluate(()=>{window.unifiedTest.assets=window.unifiedTest.assets.filter(a=>a.asset_id!=='asset-upload');});
  await page.waitForTimeout(1100);
  check(await errors().count()===0,'removed asset caused its old upload cleanup to emit another failure');
  check(await page.evaluate(()=>window.unifiedTest.operations.find(o=>o.operation_id==='op-upload').payload.operation_nonce)===nonce,'deleting changed the original publication identity');
  check(await page.evaluate(()=>window.unifiedTest.calls.filter(c=>c.type==='delete').length)===1,'deletion was replayed');
  check(await page.evaluate(()=>window.unifiedTest.calls.some(c=>c.type==='start'))===false,'deleting reuploaded the video');
  results.push('deleting during upload: expected blocked/deleted stop is silent, exact deletion once, nonce retained, no new upload');
  await load();
  await row('upload').locator('.task-row-title').click({button:'right'});
  await page.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).click();
  await label('upload','删除中');
  await page.evaluate(()=>window.unifiedTest.change('delete-asset-upload',{state:'retry_wait',attempt:2,last_error:'TELEGRAM_CONNECTION_ERROR'}));
  await page.waitForTimeout(1100);
  check(await errors().count()===0&&await row('upload').getByRole('alert').count()===0,'delete backoff displayed an intermediate error');
  await page.evaluate(()=>window.unifiedTest.change('delete-asset-upload',{last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:Date.now()/1000+2}));
  await row('upload').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
  check(await errors().count()===0 && await row('upload').getByRole('alert').count()===0,'delete flood cooldown was presented as a failure');
  await row('upload').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
  await page.evaluate(()=>window.unifiedTest.change('delete-asset-upload',{last_error:'TELEGRAM_CONNECTION_ERROR'}));
  await page.evaluate(()=>window.unifiedTest.change('delete-asset-upload',{state:'blocked',attempt:5,last_error:'TELEGRAM_CONNECTION_ERROR'}));
  await label('upload','删除失败');await row('upload').getByRole('alert').waitFor();
  check(await errors().count()===1,'actual terminal delete failure was hidden');
  results.push('deletion retry remains quiet; terminal deletion failure is still visible');
  await load();
  await page.evaluate(()=>window.unifiedTest.change('op-retry',{state:'blocked',attempt:5,last_error:'BACKUP_ASSET_DELETED'}));
  await label('retry','备份失败');await row('retry').getByRole('alert').waitFor();
  check(await errors().count()===1,'an uncorrelated asset-deleted failure was globally suppressed');
  results.push('unrelated asset-deleted failure remains visible without matching deletion facts');
  await load();
  await page.evaluate(()=>{
    window.unifiedTest.operations.push({operation_id:'historical-delete',space_id:'status-space',asset_id:'asset-retry',kind:'delete',state:'completed',phase:'completed',payload:{local_cleanup_done:true},attempt:1,last_error:null});
    window.unifiedTest.change('op-retry',{state:'blocked',attempt:5,last_error:'BACKUP_ASSET_DELETED'});
  });
  await label('retry','备份失败');await row('retry').getByRole('alert').waitFor();
  check(await errors().count()===1,'historical completed delete suppressed a newer active generation failure');
  check(await page.evaluate(()=>window.unifiedTest.assets.some(a=>a.asset_id==='asset-retry'&&a.asset_state==='active'&&!a.remote_deleted)),'history counterexample lacks a new active asset');
  check(await row('retry').locator('.backup-sync-glyph svg.lucide-circle-alert').count()===1,'new generation terminal failure kept the spinner');
  results.push('completed delete history cannot suppress a new active upload generation failure');
  await load();
  await page.evaluate(()=>window.unifiedTest.change('op-cleaning',{payload:{cleanup_error:'TELEGRAM_FLOOD_WAIT:120'},next_retry_at:Date.now()/1000+2}));
  await row('cleaning').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
  await row('cleaning').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
  await page.evaluate(()=>window.unifiedTest.change('old-dependency',{payload:{cleanup_error:'TELEGRAM_FLOOD_WAIT:120',cleanup_blocked:false},next_retry_at:Date.now()/1000+2}));
  await label('dependency','清理中');
  check(await row('dependency').getByText('Telegram 请求受限，等待后自动继续',{exact:true}).count()===0,'replacement upload inherited a previous cleanup flood notice');
  results.push('only the original cleanup shows its own flood notice; dependent upload remains a normal wait');
  const remote=page.locator('[data-asset-id="remote-upload"]');
  await page.evaluate(()=>window.unifiedTest.change('op-remote-upload',{state:'retry_wait',last_error:'TELEGRAM_FLOOD_WAIT:120',next_retry_at:Date.now()/1000+2}));
  await remote.getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
  await remote.getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
  await page.evaluate(()=>window.unifiedTest.change('op-remote-upload',{state:'running',next_retry_at:Math.floor(Date.now()/1000)+120}));
  await remote.getByText('备份中 40%',{exact:true}).waitFor();
  await page.getByRole('button',{name:'切换为列表',exact:true}).focus();
  await page.keyboard.press('Enter');
  await page.getByLabel('按状态筛选下载任务').getByLabel('同步中',{exact:true}).click();
  await remote.getByText('上传进度 40%',{exact:true}).waitFor();
  await page.evaluate(()=>window.unifiedTest.change('op-remote-upload',{state:'retry_wait',next_retry_at:Date.now()/1000+2}));
  await remote.getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor();
  await remote.getByText('Telegram 请求受限，等待后自动继续',{exact:true}).waitFor({state:'hidden'});
  await page.evaluate(()=>window.unifiedTest.change('op-remote-upload',{state:'running',next_retry_at:Math.floor(Date.now()/1000)+120}));
  await remote.getByText('上传进度 40%',{exact:true}).waitFor();
  results.push('cloud asset sync feedback hides expired and historical flood errors after resuming');
  return {passed:results.length,results};
}
