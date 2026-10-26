// Real shared Dialog/AlertDialog animations with isolated task IPC fixtures.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const content=page.locator('[data-slot="dialog-content"]');
  const alert=page.locator('[data-slot="alert-dialog-content"]');
  const overlay=page.locator('[data-slot="dialog-overlay"]');
  const row=id=>page.locator('[data-task-id="'+id+'"]');
  const phase=async(node,name)=>node.evaluate(async(element,name)=>{
    const animation=element.getAnimations().find(animation=>animation.animationName===name);
    if(!animation)throw Error('Missing '+name);
    animation.pause();animation.currentTime=125;
    await new Promise(requestAnimationFrame);
    return {opacity:Number(getComputedStyle(element).opacity),duration:animation.effect.getTiming().duration,pointer:getComputedStyle(element).pointerEvents};
  },name);
  const finish=async name=>page.locator('[data-slot="dialog-content"],[data-slot="dialog-overlay"],[data-slot="alert-dialog-content"],[data-slot="alert-dialog-overlay"]').evaluateAll((nodes,name)=>{
    for(const node of nodes)for(const animation of node.getAnimations())if(animation.animationName===name)animation.finish();
  },name);
  const load=async()=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await row('visual-3').waitFor();
  };
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    const cover=row('visual-3').getByRole('button',{name:'查看封面',exact:true});
    await cover.click();await content.waitFor();
    const entering=await phase(content,'modal-enter');
    check(entering.duration===250 && entering.opacity>0 && entering.opacity<1,'dialog lacks visible enter animation');
    const dim=await phase(overlay,'modal-enter');check(dim.opacity>0&&dim.opacity<1,'overlay did not fade in');
    const bounds=await content.boundingBox();
    check(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=width&&bounds.y+bounds.height<=height,'animation moved dialog outside viewport');
    await finish('modal-enter');
    await page.keyboard.press('Escape');
    await content.filter({has:page.getByRole('heading',{name:'任务详情',exact:true})}).waitFor({state:'attached'});
    const exiting=await phase(content,'modal-exit');
    check(exiting.duration===250 && exiting.opacity>0&&exiting.opacity<1 && exiting.pointer==='none','dialog lacks protected exit animation');
    const count=await page.evaluate(()=>window.backgroundTest.calls.filter(call=>call.type==='prepare-playback').length);
    const play=await row('visual-3').locator('button[aria-label="播放视频"]').boundingBox();
    await page.mouse.click(play.x+play.width/2,play.y+play.height/2);
    check(await page.evaluate(()=>window.backgroundTest.calls.filter(call=>call.type==='prepare-playback').length)===count,'closing overlay allowed click-through');
    await finish('modal-exit');await content.waitFor({state:'detached'});await overlay.waitFor({state:'detached'});
    await page.waitForFunction(()=>document.activeElement?.closest('[data-task-id]')?.dataset.taskId==='visual-3'&&document.activeElement?.hasAttribute('data-media-cover'));
    await row('visual-1').getByRole('button',{name:'取消',exact:true}).click();await alert.waitFor();
    const confirmEntering=await phase(alert,'modal-enter');check(confirmEntering.opacity>0&&confirmEntering.opacity<1,'confirmation lacks enter animation');
    await finish('modal-enter');await alert.getByRole('button',{name:'返回',exact:true}).click();
    const confirmExiting=await phase(alert,'modal-exit');check(confirmExiting.opacity>0&&confirmExiting.opacity<1,'confirmation lacks exit animation');
    await finish('modal-exit');await alert.waitFor({state:'detached'});
    check(!await page.evaluate(()=>window.backgroundTest.calls.some(call=>call.type==='cancel')),'dismissed confirmation executed cancellation');
    await row('visual-6').getByRole('button',{name:'查看图集',exact:true}).click();await content.waitFor();
    check((await phase(content,'modal-enter')).opacity>0,'album lacks shared animation');await finish('modal-enter');
    await page.keyboard.press('Escape');await content.waitFor({state:'detached'});
    cases.push(width+': dialog/overlay/confirmation/album enter+exit, fixed geometry, no click-through and focus');
  }
  for(const key of ['Enter','Space']) {
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.evaluate(async()=>{const url=performance.getEntriesByType('resource').find(entry=>entry.name.includes('/src/api.ts')).name;const{api}=await import(url);api.startUpdateDownload=async()=>{window.backgroundTest.calls.push({type:'closed-update-start'});throw 'NETWORK_ERROR'}});
    await page.getByRole('button',{name:'关于软件',exact:true}).click();
    await page.getByRole('button',{name:'检查更新',exact:true}).click();await content.waitFor();
    await finish('modal-enter');
    const download=content.getByRole('button',{name:'下载更新',exact:true});await download.focus();
    if(key==='Space')await page.keyboard.down('Space');
    await page.keyboard.press('Escape');await phase(content,'modal-exit');
    if(key==='Space')await page.keyboard.up('Space');else await page.keyboard.press('Enter');
    await content.locator('button').filter({hasText:'下载更新'}).evaluate(node=>node.click());
    check(!await page.evaluate(()=>window.backgroundTest.calls.some(call=>call.type==='closed-update-start')),'closed update dialog accepted '+key+' or accessibility click');
    await finish('modal-exit');await content.waitFor({state:'detached'});
    cases.push('closed update blocks '+key+' and accessibility clicks');
  }
  await load();await row('visual-1').getByRole('button',{name:'取消',exact:true}).click();await alert.waitFor();await finish('modal-enter');
  const confirmation=alert.getByRole('button',{name:'确认取消',exact:true});
  await confirmation.evaluate(node=>{window.closedConfirmClicks=0;node.addEventListener('click',()=>window.closedConfirmClicks++)});
  await confirmation.focus();await page.keyboard.press('Escape');await phase(alert,'modal-exit');
  await page.keyboard.press('Enter');await alert.locator('button').filter({hasText:'确认取消'}).evaluate(node=>node.click());
  check(await page.evaluate(()=>window.closedConfirmClicks)===0,'closed confirmation accepted keyboard or accessibility click');
  await finish('modal-exit');await alert.waitFor({state:'detached'});
  cases.push('closed confirmation blocks keyboard and accessibility clicks');
  await page.emulateMedia({reducedMotion:'reduce'});await load();
  await row('visual-3').getByRole('button',{name:'查看封面',exact:true}).click();await content.waitFor();
  check(await content.evaluate(node=>getComputedStyle(node).animationName==='none'),'reduced motion still animates dialogs');
  await page.keyboard.press('Escape');await content.waitFor({state:'detached'});
  await row('visual-1').getByRole('button',{name:'取消',exact:true}).click();await alert.waitFor();
  check(await alert.evaluate(node=>getComputedStyle(node).animationName==='none'),'reduced motion still animates confirmation');
  await alert.getByRole('button',{name:'返回',exact:true}).click();await alert.waitFor({state:'detached'});
  await page.emulateMedia({reducedMotion:'no-preference'});
  cases.push('reduced motion disables all modal enter/exit');
  return {passed:cases.length,cases};
}
