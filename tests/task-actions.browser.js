// Real App with isolated IPC mocks; never starts a player or removes user files.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const row=title=>page.locator('.task-row').filter({has:page.getByText(title,{exact:true})});
  const calls=()=>page.evaluate(()=>window.backgroundTest.calls);
  const results=[];
  const load=async(scenario='actions')=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await row('可播放视频').waitFor();
  };
  const open=async target=>{await target.click({button:'right',position:{x:200,y:30}});const menu=page.getByRole('menu',{name:'任务操作'});await menu.waitFor();return menu};
  const act=async(target,label)=>{const menu=await open(target);await menu.getByRole('menuitem',{name:label,exact:true}).click()};
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    const video=row('可播放视频');
    const play=video.getByRole('button',{name:'播放视频',exact:true});
    const box=await play.boundingBox();
    check(box.width===32 && box.height===32 && (await play.innerText())==='', 'play must be icon only');
    check(await play.evaluate(n=>getComputedStyle(n).borderWidth==='0px'), 'play retains border');
    check(await video.getByRole('button',{name:'更多操作',exact:true}).count()===0,'more icon remains');
    for(const label of ['打开文件夹','从列表移除','复制原链接','删除本地视频'])
      check(await video.getByRole('button',{name:label,exact:true}).count()===0,'secondary action remains inline');
    let menu=await open(video);
    check((await menu.getByRole('menuitem').allTextContents()).map(text=>text.trim()).join(',')==='打开文件夹,提取文案,复制原链接,复制本地地址,从列表移除,删除本地视频','menu order or missing operation');
    const danger=menu.getByRole('menuitem',{name:'删除本地视频',exact:true});
    check(await danger.getAttribute('data-variant')==='destructive','dangerous action loses semantics');
    await page.keyboard.press('Escape');
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-context-target'));
    const menuTarget=video.locator('..');
    await menuTarget.focus(); await page.keyboard.press('Shift+F10');
    await page.getByRole('menu',{name:'任务操作'}).waitFor();
    await page.keyboard.press('Escape');
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-context-target'));
    for(const title of ['已完成图集','文件缺失视频','下载中视频','失败视频','取消视频'])
      check(await row(title).getByRole('button',{name:'播放视频',exact:true}).count()===0,'invalid playback '+title);
    for(const title of ['文件缺失视频','下载中视频','失败视频','取消视频']) {
      menu=await open(row(title));
      check(await menu.getByRole('menuitem',{name:/删除本地/}).count()===0,'invalid file deletion '+title);
      if(title==='下载中视频')check(await menu.getByRole('menuitem',{name:'从列表移除',exact:true}).count()===0,'active task removable');
      if(title==='文件缺失视频')check(await menu.getByRole('menuitem',{name:'打开文件夹',exact:true}).count()===0,'missing file has folder action');
      await page.keyboard.press('Escape');
    }
    await video.getByRole('button',{name:'播放视频',exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='prepare-playback'));
    await page.locator('.video-player-dialog').waitFor();await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
    await act(video,'打开文件夹');await act(video,'复制原链接');
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='copy'));
    const recorded=await calls();
    check(recorded.find(c=>c.type==='prepare-playback').id==='done' && recorded.find(c=>c.type==='reveal').id==='done','wrong task targeted');
    check(recorded.find(c=>c.type==='copy').text==='https://t.me/c/100/9','wrong copied canonical URL');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    await act(video,'从列表移除');await video.waitFor({state:'hidden'});
    check(await page.getByRole('alertdialog').count()===0,'record removal asks for confirmation');
    check((await calls()).filter(c=>c.type==='remove' && c.id==='done').length===1,'expected exactly one removal');
    check(!(await calls()).some(c=>c.type==='delete-file'),'record removal deleted a file');
    await page.waitForFunction(()=>document.activeElement!==document.body && (document.activeElement?.closest('[data-task-row]') || document.activeElement?.getAttribute('role')==='tabpanel'));
    check(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')!=='搜索作品标题或作者'),'removal stole search focus');
    results.push(width+' menu/conditions/keyboard/copy/removal');
  }
  for(const [title,label,id] of [['可播放视频','删除本地视频','done'],['已完成图集','删除本地图集','album']]) {
    await load();await act(row(title),label);await row(title).waitFor({state:'hidden'});
    check(await page.getByRole('alertdialog').count()===0,'file deletion asks for confirmation');
    check((await calls()).length===1 && (await calls())[0].type==='delete-file' && (await calls())[0].id===id,'incorrect delete IPC');
    check(!(await page.evaluate(()=>window.backgroundTest.snapshot())).some(t=>t.id===id),'deleted task survived');
    results.push(label+' removes record without confirmation');
  }
  for(const [scenario,label,type] of [['actions-remove-error','从列表移除','remove'],['actions-delete-error','删除本地视频','delete-file']]) {
    await load(scenario);await act(row('可播放视频'),label);
    await page.locator('[data-sonner-toast][data-type="error"]').waitFor();
    check(await row('可播放视频').isVisible(),'failure removed task');
    const retryMenu=await open(row('可播放视频'));
    check(await retryMenu.getByRole('menuitem',{name:label,exact:true}).isEnabled(),'failure prevents retry');
    await page.keyboard.press('Escape');
    check((await calls()).length===1 && (await calls())[0].type===type,'failure triggered another operation');
    results.push(label+' failure preserved task');
  }
  for(const [scenario,label,type] of [['actions-remove-pending','从列表移除','remove'],['actions-delete-pending','删除本地视频','delete-file']]) {
    await load(scenario);const video=row('可播放视频');
    await act(video,label);await page.waitForFunction(()=>window.backgroundTest.calls.length===1);
    check(await video.getByRole('button',{name:'播放视频',exact:true}).isDisabled(),'pending play enabled');
    const pendingMenu=await open(video);
    check(await pendingMenu.getByRole('menuitem').evaluateAll(nodes=>nodes.every(n=>n.getAttribute('aria-disabled')==='true')),'pending menu action enabled');
    await page.keyboard.press('Enter');check((await calls()).filter(c=>c.type===type).length===1,'duplicate operation');
    await page.evaluate(()=>window.backgroundTest.release());await video.waitFor({state:'hidden'});
    results.push(label+' pending prevents duplicate action');
  }
  for(const label of ['从列表移除','删除本地视频']) {
    await load('actions-no-event');await act(row('可播放视频'),label);await row('可播放视频').waitFor({state:'hidden'});
    await page.evaluate(()=>window.backgroundTest.replay());
    check(await row('可播放视频').count()===0,'stale event resurrected task');
    results.push(label+' acknowledged without event, ignores stale snapshots');
  }
  await load('actions-error');await row('可播放视频').getByRole('button',{name:'播放视频',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'无法启动系统默认播放器'}).waitFor();
  await page.keyboard.press('Escape');await page.locator('.video-player-dialog').waitFor({state:'detached'});
  check(await row('可播放视频').isVisible(),'player error removed record');
  await load();await row('下载中视频').getByRole('button',{name:'取消',exact:true}).click();
  const dialog=page.getByRole('alertdialog');await dialog.getByRole('heading',{name:'取消当前任务？',exact:true}).waitFor();
  check((await calls()).length===0,'cancellation before confirmation');
  await dialog.getByRole('button',{name:'返回',exact:true}).click();await dialog.waitFor({state:'hidden'});
  results.push('playback error/cancel confirmation preserved');
  return {passed:results.length,cases:results};
}
