// Isolated clipboard IPC, pixel and dialog checks via the existing Playwright CLI.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const menu=page.getByRole('menu',{name:'封面操作',exact:true});
  const copy=menu.getByRole('menuitem',{name:'复制封面',exact:true});
  const dialog=page.getByRole('dialog',{name:'任务详情',exact:true});
  const trigger=id=>page.locator('[data-slot="card"]').filter({has:page.getByText(id+' cover',{exact:true})}).locator('[data-cover-context-menu]');
  const load=async()=>{await page.goto(base+'/tests/fixtures/cover-save.html');await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.waitForFunction(()=>[...document.querySelectorAll('[data-cover-context-menu] img')].filter(image=>image.complete&&image.naturalWidth).length===6)};
  const open=async id=>{await trigger(id).click({button:'right',force:true});await menu.waitFor();check(await dialog.count()===0,'right click opened the preview')};
  const results=[];
  for(const id of ['douyin','tiktok','youtube','telegram','legacy','album']) {
    await load();
    const expected=await trigger(id).getByRole('img').evaluate(image=>{const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;const context=canvas.getContext('2d');context.drawImage(image,0,0);return {width:canvas.width,height:canvas.height,rgba:Array.from(context.getImageData(0,0,canvas.width,canvas.height).data)}});
    await open(id);await copy.click();await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();
    const actual=await page.evaluate(()=>window.coverSaveTest.copies);
    check(actual.length===1&&JSON.stringify(actual[0])===JSON.stringify(expected),'clipboard did not receive original-sized RGBA pixels for '+id);
    check(await page.evaluate(()=>window.coverSaveTest.calls.length)===0,'copy opened save dialog');
    results.push(id+': actual pixels, original dimensions, no save dialog');
  }
  await load();await page.evaluate(()=>window.coverSaveTest.copyError=true);await open('douyin');await copy.click();
  await page.getByText('无法复制封面，请稍后重试',{exact:true}).waitFor();check(await page.getByText('封面已复制到剪贴板',{exact:true}).count()===0,'failure produced success');results.push('clipboard failure visible');
  await load();await page.evaluate(()=>window.coverSaveTest.copyPending=true);await open('douyin');await copy.click();
  await open('douyin');check(await menu.getByRole('menuitem',{name:'复制中…',exact:true}).isDisabled(),'pending copy enabled');
  check(await menu.getByRole('menuitem',{name:'保存封面',exact:true}).isDisabled(),'save enabled during copy');
  await page.keyboard.press('Escape');await page.evaluate(()=>window.coverSaveTest.releaseCopy());await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();
  check(await page.evaluate(()=>window.coverSaveTest.copies.length)===1,'duplicate clipboard write');results.push('pending copy blocks repeated actions');
  for(const id of ['missing','broken','broken-album']) {
    await load();await open(id);check(await copy.isDisabled(),'missing cover copy enabled');await page.keyboard.press('Escape');
    await trigger(id).click({force:true});await dialog.waitFor();
    check(await dialog.getByRole('region',{name:'任务详细信息'}).isVisible(),'missing cover lost details');
    check(await dialog.getByRole('img',{name:'封面大图',exact:true}).count()===0,'failed image shown in details');
    await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
  }
  results.push('missing and failed video/album disable copy but retain details');
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    for(const id of ['douyin','legacy','youtube','album']) {
      const source=await trigger(id).getByRole('img').getAttribute('src');
      await trigger(id).click();await dialog.waitFor();
      const image=dialog.getByRole('img',{name:'封面大图',exact:true});await image.evaluate(image=>image.decode());
      check(await image.getAttribute('src')===source,'preview fetched another image');
      const bounds=await dialog.boundingBox();check(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=width&&bounds.y+bounds.height<=height,'dialog outside viewport');
      await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
      await page.waitForFunction(id=>document.activeElement?.closest('[data-slot="card"]')?.textContent.includes(id+' cover'),id);
      check(await trigger(id).evaluate(node=>document.activeElement===node),'focus was not restored');
    }
    results.push('PNG/JPEG/WebP/album preview, viewport and focus '+width);
  }
  await load();await trigger('douyin').focus();await page.keyboard.press('Enter');await dialog.waitFor();
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();await dialog.waitFor({state:'hidden'});
  await trigger('douyin').focus();await page.keyboard.press('Space');await dialog.waitFor();
  await page.locator('[data-slot="dialog-overlay"]').click({position:{x:5,y:5}});await dialog.waitFor({state:'hidden'});results.push('Enter/Space, close button and backdrop');
  return {passed:results.length,cases:results};
}
