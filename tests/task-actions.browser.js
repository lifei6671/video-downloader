// Real App with isolated IPC mocks; never starts a player or removes user files.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const row=title=>page.locator('[data-slot="card"]').filter({has:page.getByText(title,{exact:true})});
  const calls=()=>page.evaluate(()=>window.backgroundTest.calls);
  const results=[];
  const labels=['播放视频','打开文件夹','从列表移除','复制原链接','删除本地视频'];
  const tips=['使用系统默认播放器播放','打开文件夹','从列表移除（保留本地文件）','复制原链接','删除本地视频及下载记录'];
  const load=async(scenario='actions')=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await row('可播放视频').waitFor();
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    const video=row('可播放视频');
    let previous;
    for(let i=0;i<labels.length;i++) {
      const button=video.getByRole('button',{name:labels[i],exact:true});
      check(await button.evaluate(b=>b.textContent.trim()==='' && !!b.querySelector('svg[aria-hidden="true"]')),'expected named icon-only button');
      const box=await button.boundingBox();
      check(box.width===28 && box.height===28,'inconsistent icon dimensions');
      if(previous)check(box.x>previous.x && box.y===previous.y,'icons wrapped or out of order');
      previous=box;
      await page.getByRole('heading',{name:'下载任务',exact:true}).hover();
      await button.focus();await button.hover();
      await page.getByRole('tooltip').filter({hasText:tips[i]}).waitFor();
      await page.keyboard.press('Escape');
    }
    await page.getByRole('heading',{name:'下载任务',exact:true}).hover();
    const red=video.getByRole('button',{name:'删除本地视频',exact:true});
    await red.evaluate(b=>Promise.all(b.getAnimations().map(animation=>animation.finished)));
    check(await red.evaluate(b=>{
      const probe=document.createElement('span');probe.style.color='var(--destructive)';b.append(probe);
      const expected=getComputedStyle(probe).color;probe.remove();
      return getComputedStyle(b).color===expected && getComputedStyle(b).backgroundColor==='rgba(0, 0, 0, 0)';
    }),'delete icon must be red by default with ghost styling');
    check(await page.getByRole('button',{name:'更多操作',exact:true}).count()===0,'secondary menu remains');
    for(const title of ['已完成图集','文件缺失视频','下载中视频','失败视频','取消视频'])
      check(await row(title).getByRole('button',{name:'播放视频',exact:true}).count()===0,'unexpected playback for '+title);
    for(const title of ['文件缺失视频','下载中视频','失败视频','取消视频'])
      check(await row(title).getByRole('button',{name:'删除本地视频',exact:true}).count()===0,'invalid local deletion for '+title);
    check(await row('下载中视频').getByRole('button',{name:'从列表移除',exact:true}).count()===0,'active task removable');
    check(await row('已完成图集').getByRole('button',{name:'打开文件夹',exact:true}).count()===1,'album folder missing');
    check(await row('文件缺失视频').getByRole('button',{name:'打开文件夹',exact:true}).count()===0,'missing file has folder action');
    await video.getByRole('button',{name:labels[0],exact:true}).focus();
    for(let i=1;i<labels.length;i++) {
      await page.keyboard.press('Tab');
      check(await video.getByRole('button',{name:labels[i],exact:true}).evaluate(b=>b===document.activeElement),'icon tab order');
    }
    await video.getByRole('button',{name:'播放视频',exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='play'));
    await video.getByRole('button',{name:'打开文件夹',exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='reveal'));
    await video.getByRole('button',{name:'复制原链接',exact:true}).focus();await page.keyboard.press('Enter');
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='copy'));
    const recorded=await calls();
    check(recorded.find(c=>c.type==='play').id==='done' && recorded.find(c=>c.type==='reveal').id==='done','wrong player/folder task');
    check(recorded.find(c=>c.type==='copy').text==='https://t.me/c/100/9','wrong copied canonical link');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    await page.getByRole('heading',{name:'下载任务',exact:true}).hover();
    await page.screenshot({path:`.local/task-delete-icons-${width}.png`,animations:'disabled'});
    await video.getByRole('button',{name:'从列表移除',exact:true}).focus();await page.keyboard.press('Enter');
    await video.waitFor({state:'hidden'});
    check(await page.getByRole('alertdialog').count()===0,'record removal asks for confirmation');
    check((await calls()).filter(c=>c.type==='remove' && c.id==='done').length===1,'expected one direct removal');
    check(!(await calls()).some(c=>c.type==='delete-file'),'record removal deletes a file');
    check(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')!=='搜索作品标题或作者'),'record removal stole search focus');
    results.push(width+' direct icons/order/tooltips/keyboard/red/copy/record removal');
  }
  for(const [title,label,id] of [['可播放视频','删除本地视频','done'],['已完成图集','删除本地图集','album']]) {
    await load();await row(title).getByRole('button',{name:label,exact:true}).focus();await page.keyboard.press('Enter');
    await row(title).waitFor({state:'hidden'});
    check(await page.getByRole('alertdialog').count()===0,'file deletion asks for confirmation');
    check((await calls()).length===1 && (await calls())[0].type==='delete-file' && (await calls())[0].id===id,'incorrect deletion IPC');
    check(!(await page.evaluate(()=>window.backgroundTest.snapshot())).some(t=>t.id===id),'deleted task survived');
    check(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')!=='搜索作品标题或作者'),'local deletion stole search focus');
    results.push(label+' removes task without confirmation');
  }
  for(const [scenario,label,type] of [['actions-remove-error','从列表移除','remove'],['actions-delete-error','删除本地视频','delete-file']]) {
    await load(scenario);await row('可播放视频').getByRole('button',{name:label,exact:true}).click();
    await page.locator('[data-sonner-toast][data-type="error"]').waitFor();
    check(await row('可播放视频').isVisible(),'failure removed record');
    check(await row('可播放视频').getByRole('button',{name:label,exact:true}).isEnabled(),'failure prevents retry');
    check((await calls()).length===1 && (await calls())[0].type===type,'failure triggered other operations');
    results.push(label+' failure preserves task and permits retry');
  }
  for(const [scenario,label,type] of [['actions-remove-pending','从列表移除','remove'],['actions-delete-pending','删除本地视频','delete-file']]) {
    await load(scenario);const video=row('可播放视频');
    await video.getByRole('button',{name:label,exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.calls.length===1);
    for(const name of labels)check(await video.getByRole('button',{name,exact:true}).isDisabled(),'pending action enabled: '+name);
    await page.keyboard.press('Enter');check((await calls()).filter(c=>c.type===type).length===1,'duplicate operation');
    await page.evaluate(()=>window.backgroundTest.release());await video.waitFor({state:'hidden'});
    results.push(label+' pending prevents repeat and conflicting actions');
  }
  for(const label of ['从列表移除','删除本地视频']) {
    await load('actions-no-event');await row('可播放视频').getByRole('button',{name:label,exact:true}).click();
    await row('可播放视频').waitFor({state:'hidden'});
    await page.evaluate(()=>window.backgroundTest.replay());
    await page.getByRole('heading',{name:'下载任务',exact:true}).click();
    check(await row('可播放视频').count()===0,'late event resurrected task');
    results.push(label+' acknowledgment without event and stale updates');
  }
  await load('actions-error');await row('可播放视频').getByRole('button',{name:'播放视频',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'无法启动系统默认播放器'}).waitFor();
  check(await row('可播放视频').isVisible(),'playback failure removed record');
  results.push('playback failure preserves record');
  await load();await row('下载中视频').getByRole('button',{name:'取消',exact:true}).click();
  const dialog=page.getByRole('alertdialog');await dialog.getByRole('heading',{name:'取消当前任务？',exact:true}).waitFor();
  check((await calls()).length===0,'cancel before confirmation');
  await dialog.getByRole('button',{name:'返回',exact:true}).click();await dialog.waitFor({state:'hidden'});
  results.push('running task cancellation retains its existing confirmation');
  return {passed:results.length,cases:results};
}
