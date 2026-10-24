// Isolated IPC. Pause actual browser animations to inspect intermediate geometry.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const row=id=>page.locator(`[data-task-row="${id}"]`);
  const act=async(id,label)=>{await row(id).click({button:'right',position:{x:200,y:30}});await page.getByRole('menuitem',{name:label,exact:true}).click()};
  const tabs=page.getByRole('tablist',{name:'按平台筛选下载任务'});
  const clear=page.locator('header button[aria-label="清理历史"]');
  const dialog=page.getByRole('alertdialog');
  const results=[];
  const load=async scenario=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await tabs.waitFor();
    await page.evaluate(()=>{
      window.motion=[];
      const animate=Element.prototype.animate;
      Element.prototype.animate=function(frames,options){
        const animation=animate.call(this,frames,options);
        if(this.hasAttribute('data-task-row')){window.motion.push({node:this,animation});animation.pause();animation.currentTime=0;}
        return animation;
      };
    });
  };
  const frame=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  const finish=async()=>{
    await page.evaluate(()=>window.motion.forEach(({animation})=>animation.finish()));
    await page.waitForFunction(()=>document.querySelectorAll('[data-task-row][data-removing]').length===0);
  };
  await page.emulateMedia({reducedMotion:'no-preference'});
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for(const label of ['从列表移除','删除本地视频']) {
      await load('actions-no-event');
      const before=await row('done').boundingBox();const below=await row('album').boundingBox();
      const tabY=(await tabs.boundingBox()).y;
      await act('done',label);
      await page.waitForFunction(()=>window.motion.length===1);await frame();
      check(Math.abs((await row('done').boundingBox()).y-before.y)<1,'exiting row reordered before animation');
      check(await row('done').getAttribute('inert')!==null,'exiting row remains interactive');
      await page.evaluate(()=>window.motion[0].animation.currentTime=110);await frame();
      const middle=await row('done').boundingBox();const moving=await row('album').boundingBox();
      check(middle.height>0 && middle.height<before.height,'row space did not collapse progressively');
      check(middle.y<before.y,'removed row did not move upward');
      check(moving.y<below.y && moving.y>below.y-before.height,'following row did not slide');
      check(await row('done').evaluate(n=>{const o=+getComputedStyle(n).opacity;return o>0&&o<1}),'row did not fade');
      await page.evaluate(()=>window.backgroundTest.replay());await frame();
      check(await row('done').getAttribute('data-removing')==='true','late update interrupted exit');
      await page.screenshot({path:`.local/task-removal-motion-${width}-${label==='从列表移除'?'record':'file'}.png`,animations:'allow'});
      await finish();await row('done').waitFor({state:'hidden'});
      check(Math.abs((await row('album').boundingBox()).y-(below.y-before.height))<1,'final removal snapped list gap');
      check(Math.abs((await tabs.boundingBox()).y-tabY)<1,'removal moved tab bar');
      check(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')!=='搜索作品标题或作者'),'deletion focused search');
      results.push(width+' '+label+' fade/up/collapse/following slide/no focus theft');
    }
  }
  for(const [name,platform] of [['抖音','douyin'],['TikTok','tiktok'],['YouTube','youtube'],['Telegram','telegram'],['全部',null]]) {
    await load('clear-platforms');await tabs.getByRole('tab',{name,exact:true}).click();
    const before=await page.evaluate(()=>window.backgroundTest.snapshot());
    const expected=before.filter(t=>t.download_requested&&['completed','failed','canceled'].includes(t.status)&&(!platform||(t.video?.platform??(t.telegram_link?'telegram':undefined))===platform)).map(t=>t.id);
    await clear.click();await dialog.getByText(new RegExp(platform?name+'中':'全部平台中')).waitFor();
    await dialog.getByRole('button',{name:'清理历史',exact:true}).click();await dialog.waitFor({state:'hidden'});
    await page.waitForFunction(count=>window.motion.length===count,expected.length);await frame();
    const ids=await page.evaluate(()=>window.motion.map(({node})=>node.dataset.taskRow));
    check(ids.length===expected.length&&expected.every(id=>ids.includes(id)),'batch animated incorrect tasks');
    check(await page.evaluate(p=>window.backgroundTest.calls.filter(c=>c.type==='clear-history').length===1&&window.backgroundTest.calls.find(c=>c.type==='clear-history').platform===p,platform),'wrong platform IPC');
    const remaining=await page.evaluate(()=>window.backgroundTest.snapshot());
    check(before.filter(t=>!expected.includes(t.id)).every(t=>remaining.some(r=>r.id===t.id)),'clear touched another tab or live task');
    check(await clear.isDisabled(),'empty current tab still has clear enabled');
    await finish();
    check(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')!=='搜索作品标题或作者'),'clear focused search');
    if(platform){await tabs.getByRole('tab',{name:platform==='douyin'?'TikTok':'抖音',exact:true}).click();check(await clear.isEnabled(),'other tab history lost');}
    results.push(name+' scoped clear and simultaneous removal animation');
  }
  await load('clear-platforms');await tabs.getByRole('tab',{name:'YouTube',exact:true}).click();
  await page.getByRole('textbox',{name:'搜索作品标题或作者'}).fill('no-match');
  await clear.click();await dialog.getByRole('button',{name:'清理历史',exact:true}).click();await dialog.waitFor({state:'hidden'});
  check(!(await page.evaluate(()=>window.backgroundTest.snapshot())).some(t=>t.id==='yt'),'search incorrectly changed clear platform scope');
  check(await page.evaluate(()=>window.motion.length)===0,'invisible row left an animation');
  results.push('platform scope ignores search and invisible rows do not animate');
  await load('actions-delete-pending');await act('done','删除本地视频');
  check(await page.evaluate(()=>window.motion.length)===0,'animation started before deletion succeeded');
  await page.evaluate(()=>window.backgroundTest.release());await page.waitForFunction(()=>window.motion.length===1);await finish();
  results.push('animation waits for successful acknowledgment');
  await load('actions-delete-error');await act('done','删除本地视频');
  await page.locator('[data-sonner-toast][data-type="error"]').waitFor();
  check(await row('done').isVisible()&&await page.evaluate(()=>window.motion.length)===0,'failed deletion animated or removed row');
  results.push('failed deletion preserves row without animation');
  await load('actions');await act('done','删除本地视频');
  await page.waitForFunction(()=>window.motion.length===1);
  await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'下载任务',exact:true}).click();
  check(await row('done').count()===0,'navigation retained departing snapshot');
  results.push('navigation cleans up unfinished exit');
  await page.emulateMedia({reducedMotion:'reduce'});await load('actions');
  await act('done','删除本地视频');await row('done').waitFor({state:'hidden'});
  check(await page.evaluate(()=>window.motion.length)===0,'reduced motion still animates');
  results.push('reduced motion removes immediately');
  await page.emulateMedia({reducedMotion:'no-preference'});
  return {passed:results.length,cases:results};
}
