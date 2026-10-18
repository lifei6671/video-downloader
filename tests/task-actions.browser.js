// Real App with isolated IPC mocks; never starts a player or removes local files.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const row=title=>page.locator('[data-slot="card"]').filter({has:page.getByText(title,{exact:true})});
  const calls=()=>page.evaluate(()=>window.backgroundTest.calls);
  const results=[];
  const load=async scenario=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await row('可播放视频').waitFor();
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load('actions');
    const video=row('可播放视频');
    const play=video.getByRole('button',{name:'播放视频',exact:true});
    const folder=video.getByRole('button',{name:'打开文件夹',exact:true});
    const remove=video.getByRole('button',{name:'从列表移除',exact:true});
    const boxes=[];
    for(const [button,tip] of [[play,'使用系统默认播放器播放'],[folder,'打开文件夹'],[remove,'从列表移除']]) {
      check(await button.evaluate(b=>b.textContent.trim()==='' && !!b.querySelector('svg[aria-hidden="true"]')),'expected named icon-only button');
      boxes.push(await button.boundingBox());
      await page.getByRole('heading',{name:'下载任务',exact:true}).hover();
      await button.focus();
      await button.hover();await page.getByRole('tooltip').filter({hasText:tip}).waitFor();
      await page.keyboard.press('Escape');
    }
    check(boxes[0].x<boxes[1].x && boxes[1].x<boxes[2].x,'expected play/folder/remove order');
    for(const title of ['已完成图集','文件缺失视频','下载中视频','失败视频','取消视频'])
      check(await row(title).getByRole('button',{name:'播放视频',exact:true}).count()===0,'unexpected playback for '+title);
    check(await row('已完成图集').getByRole('button',{name:'打开文件夹',exact:true}).count()===1,'album folder missing');
    check(await row('文件缺失视频').getByRole('button',{name:'打开文件夹',exact:true}).count()===0,'missing file has folder action');
    check(await row('文件缺失视频').getByRole('button',{name:'从列表移除',exact:true}).count()===1,'missing file removal unavailable');
    await play.focus();await page.keyboard.press('Enter');
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='play'));
    check((await calls()).filter(c=>c.type==='play').length===1 && (await calls()).find(c=>c.type==='play').id==='done','wrong playback task');
    await folder.click();await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='reveal'));
    check((await calls()).find(c=>c.type==='reveal').id==='done','wrong folder task');
    await remove.focus();await page.keyboard.press('Enter');
    const dialog=page.getByRole('alertdialog');
    await dialog.getByRole('heading',{name:'从列表移除？',exact:true}).waitFor();
    check(!(await calls()).some(c=>c.type==='remove'||c.type==='delete-file'),'removal before confirmation');
    await dialog.getByRole('button',{name:'返回',exact:true}).click();await dialog.waitFor({state:'hidden'});
    await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='从列表移除');
    await video.getByRole('button',{name:'更多操作',exact:true}).click();
    await page.getByRole('menuitem',{name:'删除本地视频',exact:true}).waitFor();
    check(await page.getByRole('menuitem',{name:'从列表移除',exact:true}).count()===0,'remove duplicated in dropdown');
    await page.getByRole('menuitem',{name:'复制原链接',exact:true}).waitFor();
    await page.keyboard.press('Escape');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    await page.screenshot({path:`.local/task-actions-${width}.png`,animations:'disabled'});
    await remove.click();await dialog.getByRole('button',{name:'移除记录',exact:true}).click();
    await video.waitFor({state:'hidden'});
    check((await calls()).some(c=>c.type==='remove' && c.id==='done'),'missing confirmed remove');
    check(!(await calls()).some(c=>c.type==='delete-file'),'record removal deleted a file');
    results.push(width+' icon-only/tooltips/order/playback/folder/confirmation/no-file-delete');
  }
  // Isolate mouse hover from Radix's pointer grace area between adjacent tips.
  for(const [label,tip] of [['播放视频','使用系统默认播放器播放'],['打开文件夹','打开文件夹'],['从列表移除','从列表移除']]) {
    await load('actions');
    await row('可播放视频').getByRole('button',{name:label,exact:true}).hover();
    await page.getByRole('tooltip').filter({hasText:tip}).waitFor();
  }
  results.push('mouse-only hover for all three icons');
  await load('actions-error');
  await row('可播放视频').getByRole('button',{name:'播放视频',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'无法启动系统默认播放器'}).waitFor();
  check(await row('可播放视频').isVisible(),'playback failure removed record');
  check((await calls()).length===1 && (await calls())[0].type==='play','playback failure triggered other operation');
  results.push('playback failure preserves record and reports error');
  return {passed:results.length,cases:results};
}
