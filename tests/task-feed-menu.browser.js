// Real App with isolated cover/file/clipboard IPC fixtures.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const row=id=>page.locator('[data-task-id="'+id+'"]');
  const menu=()=>page.getByRole('menu',{name:'任务操作',exact:true});
  const cover=id=>row(id).getByRole('button',{name:'查看封面',exact:true});
  const open=async target=>{await target.click({button:'right',force:true});await menu().waitFor();check(await page.getByRole('menu').count()===1,'multiple menus opened')};
  const close=async()=>{await page.keyboard.press('Escape');await menu().waitFor({state:'hidden'})};
  const load=async()=>{
    await page.evaluate(()=>localStorage.setItem('yingqu.task-view','feed'));
    await page.goto(base+'/tests/fixtures/cover-save.html');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.waitForFunction(()=>[...document.querySelectorAll('[data-media-cover] img')].filter(image=>image.complete&&image.naturalWidth).length===6);
  };
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    await open(cover('douyin'));
    const entries=await menu().getByRole('menuitem').allTextContents();
    for(const label of ['保存封面','复制封面','复制原链接','从列表移除','删除本地视频'])check(entries.includes(label),'missing '+label);
    const box=await menu().boundingBox();check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'menu outside viewport');
    await close();await open(row('douyin').locator('.task-row-title'));
    check(JSON.stringify(await menu().getByRole('menuitem').allTextContents())===JSON.stringify(entries),'cover/body menus differ');
    await menu().getByRole('menuitem',{name:'保存封面',exact:true}).click();
    await page.getByText('封面已保存',{exact:true}).waitFor();
    check((await page.evaluate(()=>window.coverSaveTest.calls)).join(',')==='douyin','wrong save target');
    await cover('douyin').focus();await page.keyboard.press('Shift+F10');await menu().waitFor();await close();
    await page.waitForFunction(()=>document.activeElement?.closest('[data-task-row]')?.dataset.taskRow==='douyin');
    await cover('douyin').click();await page.getByRole('dialog',{name:'任务详情',exact:true}).waitFor();
    await page.getByRole('img',{name:'封面大图',exact:true}).click({button:'right'});
    await page.getByRole('menu',{name:'封面预览操作',exact:true}).waitFor();
    check(await menu().count()===0,'preview opened task menu');
    await page.keyboard.press('Escape');await page.keyboard.press('Escape');
    results.push(width+': same menu, save, viewport, keyboard, preview');
  }
  for(const id of ['youtube','album']) {
    await load();
    const expected=await cover(id).locator('img').evaluate(image=>({width:image.naturalWidth,height:image.naturalHeight}));
    await open(row(id).locator('.task-row-title'));
    await menu().getByRole('menuitem',{name:'复制封面',exact:true}).click();await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();
    const copies=await page.evaluate(()=>window.coverSaveTest.copies);
    check(copies.length===1&&copies[0].width===expected.width&&copies[0].height===expected.height,'wrong copy dimensions/target '+id);
  }
  for(const id of ['missing','broken','broken-album']) {
    await load();await open(cover(id));
    for(const label of ['保存封面','复制封面'])check(await menu().getByRole('menuitem',{name:label,exact:true}).isDisabled(),'unavailable cover action enabled');
    check(await menu().getByRole('menuitem',{name:'从列表移除',exact:true}).isEnabled(),'unavailable cover blocked task action');await close();
  }
  await load();await page.evaluate(()=>window.coverSaveTest.copyPending=true);await open(cover('douyin'));
  await menu().getByRole('menuitem',{name:'复制封面',exact:true}).click();await open(row('douyin').locator('.task-row-title'));
  check(await menu().getByRole('menuitem',{name:'复制中…',exact:true}).isDisabled()&&await menu().getByRole('menuitem',{name:'保存封面',exact:true}).isDisabled(),'pending state not shared');
  await close();await page.evaluate(()=>window.coverSaveTest.releaseCopy());await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();
  results.push('video/album copy, unavailable covers, shared pending state');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  check(await cover('visual-1').isDisabled(),'download cover became clickable');
  await open(cover('visual-1'));
  check(await menu().getByRole('menuitem',{name:'保存封面',exact:true}).isVisible(),'disabled download cover lost merged menu');
  await menu().getByRole('menuitem',{name:'取消下载',exact:true}).click();
  await page.getByRole('alertdialog').waitFor();
  check((await page.getByRole('alertdialog').innerText()).includes('取消'),'download cancel lost confirmation');
  results.push('downloading cover keeps merged menu and cancel confirmation');
  return {passed:results.length,cases:results};
}
