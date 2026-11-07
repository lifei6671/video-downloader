// Isolated IPC fixture; no credentials or actual downloads.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const cases=[];
  const slider=()=>page.getByRole('slider',{name:'同时下载数量',exact:true});
  const stored=()=>page.evaluate(()=>window.filenameSettingsTest.snapshot().max_downloads);
  const calls=()=>page.evaluate(()=>window.filenameSettingsTest.concurrencyCalls.length);
  const open=async()=>{
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'下载设置',exact:true}).click();
    await slider().waitFor();
  };
  const settle=async value=>{
    await page.waitForFunction(value=>window.filenameSettingsTest.snapshot().max_downloads===value,value);
    await page.waitForFunction(()=>document.querySelector('.download-concurrency')?.getAttribute('aria-busy')==='false');
    check(await page.locator('[data-sonner-toast]').filter({hasText:'同时下载数量已设为'}).count()===0,'Slider save still shows a Toast');
    check(await slider().getAttribute('aria-valuenow')===String(value),'display and saved task count diverged');
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/filename-settings.html');
    await page.evaluate(()=>sessionStorage.clear());await page.reload();await open();
    check(await stored()===5&&await slider().getAttribute('aria-valuenow')==='5','default count is not five');
    check(await slider().getAttribute('aria-valuemin')==='1'&&await slider().getAttribute('aria-valuemax')==='10','task count bounds changed');
    const layout=await page.locator('.download-concurrency').evaluate(node=>{
      const bounds=node.getBoundingClientRect(),directory=document.querySelector('#download-directory').closest('[data-slot="field"]').getBoundingClientRect();
      const ticks=Array.from(node.querySelectorAll('.download-concurrency-stops > span'));
      const boxes=ticks.map(tick=>tick.getBoundingClientRect());
      return {sameWidth:Math.abs(bounds.left-directory.left)<1&&Math.abs(bounds.right-directory.right)<1,
        ticks:ticks.length,defaultFive:ticks.some(tick=>tick.textContent==='5（默认）'),
        separated:boxes.every((box,i)=>(!i||box.left>=boxes[i-1].right)&&box.left>=bounds.left&&box.right<=bounds.right)};
    });
    check(layout.sameWidth,'Slider is narrower than the directory field');
    check(layout.ticks===10&&layout.defaultFive&&layout.separated,'ten-step labels missing, overlapping or marking the wrong default');
    await slider().focus();await page.keyboard.press('End');await settle(10);
    await slider().focus();await page.keyboard.press('Home');await settle(1);
    for(const value of [2,3,4,5,6,7,8,9]){await slider().focus();await page.keyboard.press('ArrowRight');await settle(value);}
    const count=await calls();
    const track=await page.locator('.download-concurrency [data-slot="slider"]').boundingBox();
    const thumb=await slider().boundingBox();
    await page.mouse.move(thumb.x+thumb.width/2,thumb.y+thumb.height/2);await page.mouse.down();
    await page.mouse.move(track.x+track.width-2,track.y+track.height/2,{steps:5});
    check(await stored()===9&&await calls()===count,'pointer movement saved before release');
    await page.mouse.up();await settle(10);
    check(await calls()===count+1,'pointer release saved multiple times');
    await slider().click();
    check(await calls()===count+1,'unchanged value saved again');
    await page.screenshot({path:'.local/slider-no-flash/settings-'+width+'.png',animations:'disabled'});
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.reload();await open();
    check(await stored()===10&&await slider().getAttribute('aria-valuenow')==='10','reload lost count');
    cases.push(width+': full width, default five, ten keyboard steps, pointer commit and persistence');
    const positions=[];
    for(const name of ['下载作品','下载任务','系统设置','关于软件','下载任务','下载作品']) {
      await page.getByRole('button',{name,exact:true}).click();
      await page.getByRole('heading',{level:1,name,exact:true}).waitFor();
      positions.push(await page.locator('.app-page-header').evaluate(node=>{
        const box=node.getBoundingClientRect(),title=node.querySelector('h1').getBoundingClientRect();
        return {height:box.height,top:box.top,titleTop:title.top,titleHeight:title.height};
      }));
    }
    const first=positions[0];
    check(positions.every(p=>Math.abs(p.titleTop-first.titleTop)<0.1&&Math.abs(p.height-first.height)<0.1&&p.top===0),'page title moves on navigation: '+JSON.stringify(positions));
    check(first.height<=56.5,'common header no longer compact');
    cases.push(width+': all four menu titles align through repeated navigation');
  }
  await page.reload();await open();
  await page.evaluate(()=>window.filenameSettingsTest.failSave=true);
  await slider().focus();await page.keyboard.press('Home');
  await page.locator('#concurrency-error').filter({hasText:'无法保存任务记录'}).waitFor();
  check(await page.locator('[data-sonner-toast]').count()===0,'Slider failure still shows a Toast');
  check(await slider().getAttribute('aria-invalid')==='true','Slider error is not accessible');
  check(await stored()===10&&await slider().getAttribute('aria-valuenow')==='10','failed save lost stored value or failed to roll back');
  await page.evaluate(()=>window.filenameSettingsTest.failSave=false);
  await slider().focus();await page.keyboard.press('ArrowLeft');await settle(9);
  check(await page.locator('#concurrency-error').count()===0,'retry left a stale error');
  check(await slider().getAttribute('aria-invalid')==='false','retry left invalid state');
  cases.push('failed save rolls back with persistent inline error, no Toast, and retry clears it');
  await page.getByRole('status',{name:'实时预览',exact:true}).locator('[data-filename-variable]').first().waitFor();
  const previewCalls=await page.evaluate(()=>window.filenameSettingsTest.previewCalls.length);
  const measure=()=>page.locator('.download-concurrency').evaluate(node=>({height:node.getBoundingClientRect().height,sliderY:node.querySelector('[data-slot=slider]').getBoundingClientRect().y,opacity:getComputedStyle(node.querySelector('[data-slot=slider]')).opacity,spinner:!!node.querySelector('[data-slot=spinner]')}));
  const before=await measure();
  await page.evaluate(()=>{window.initialConcurrencySlider=document.querySelector('.download-concurrency [role=slider]');window.filenameSettingsTest.holdSave=true});
  await slider().focus();await page.keyboard.press('End');
  await page.locator('.download-concurrency[aria-busy="true"]').waitFor();
  check(await page.locator('.download-concurrency [data-slot="slider"]').getAttribute('data-disabled')!==null,'pending save permits edits');
  const during=await measure();
  check(during.opacity==='1'&&!during.spinner&&during.height===before.height&&during.sliderY===before.sliderY,'pending save flashes or shifts the Slider: '+JSON.stringify({before,during}));
  const pending=await calls();await page.keyboard.press('Home');
  check(await stored()===9&&await calls()===pending,'pending save changed count or duplicated submission');
  await page.evaluate(()=>{window.filenameSettingsTest.holdSave=false;window.filenameSettingsTest.release()});
  await settle(10);
  const after=await measure();
  check(after.opacity==='1'&&!after.spinner&&after.height===before.height&&after.sliderY===before.sliderY,'save completion flashes or shifts the Slider');
  check(await page.evaluate(()=>window.initialConcurrencySlider===document.querySelector('.download-concurrency [role=slider]')),'saving remounted the Slider');
  check(await page.evaluate(()=>window.filenameSettingsTest.previewCalls.length)===previewCalls,'Slider save restarted filename preview');
  check(await page.locator('[data-sonner-toast]').count()===0,'save completion showed a Toast');
  cases.push('pending and completed saves retain opacity, layout, slider node and filename preview without Toast');
  return {passed:cases.length,cases};
}
