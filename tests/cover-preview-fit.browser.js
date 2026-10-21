// Browser-only dialog geometry and preview-image context menu checks; IPC is mocked.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const dialog=page.getByRole('dialog',{name:'封面预览',exact:true});
  const image=dialog.getByRole('img',{name:'封面大图',exact:true});
  const menu=page.getByRole('menu',{name:'封面预览操作',exact:true});
  const trigger=id=>page.locator('[data-slot="card"]').filter({has:page.getByText(new RegExp('^'+id+' cover'))}).locator('[data-cover-context-menu]');
  const load=async(fit=false)=>{await page.goto(base+'/tests/fixtures/cover-save.html'+(fit?'?preview-fit=1':''));await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.waitForFunction(()=>[...document.querySelectorAll('[data-cover-context-menu] img')].filter(image=>image.complete&&image.naturalWidth).length===6)};
  const preview=async id=>{await trigger(id).click();await dialog.waitFor();await image.evaluate(image=>image.decode());await dialog.evaluate(node=>Promise.all(node.getAnimations({subtree:true}).map(animation=>animation.finished)))};
  const open=async()=>{await image.click({button:'right'});await menu.waitFor()};
  // Modal menu exit temporarily hides the parent dialog from accessibility queries.
  const menuClosed=async()=>{await menu.waitFor({state:'hidden'});await dialog.waitFor()};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load(true);
    const widths=[];
    for(const id of ['youtube','legacy']) {
      await preview(id);
      const box=await dialog.boundingBox(),picture=await image.boundingBox();
      check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'preview outside viewport');
      check(box.width-picture.width<64,'dialog stays wide independently of image or title');
      check(picture.y>=box.y&&picture.y+picture.height<=box.y+box.height,'image clipped vertically');
      const ratio=await image.evaluate(image=>image.naturalWidth/image.naturalHeight);
      check(Math.abs(picture.width/picture.height-ratio)<0.01,'image aspect ratio changed');
      check(await dialog.evaluate(node=>node.scrollHeight<=node.clientHeight+1),'preview needs vertical scrolling');
      widths.push(box.width);await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
    }
    check(widths[0]<widths[1]*0.7,'portrait and landscape dialogs have the same fixed width');
    results.push('large portrait/landscape and long-title fit '+width);
  }
  await load();await preview('youtube');
  // Make the underlying thumbnail different: copying from the preview must use the preview itself.
  await trigger('youtube').locator('img').evaluate(async image=>{const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const context=canvas.getContext('2d');context.fillStyle='#0000ff';context.fillRect(0,0,1,1);image.src=canvas.toDataURL();await image.decode()});
  await open();check((await menu.getByRole('menuitem').allTextContents()).join(',')==='另存为…,复制封面','preview menu actions differ');
  await menu.getByRole('menuitem',{name:'复制封面',exact:true}).click();await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();
  const copied=await page.evaluate(()=>({width:window.coverSaveTest.copies[0].width,height:window.coverSaveTest.copies[0].height}));
  check(copied.width===360&&copied.height===640,'copy used underlying thumbnail instead of preview');await menuClosed();results.push('preview copy uses original-sized preview image');
  await open();await menu.getByRole('menuitem',{name:'另存为…',exact:true}).click();await page.getByText('封面已保存',{exact:true}).waitFor();
  check((await page.evaluate(()=>window.coverSaveTest.calls)).join(',')==='youtube','save-as exported wrong task');await menuClosed();results.push('preview save-as preserves dialog');
  await load();await page.evaluate(()=>window.coverSaveTest.result=false);await preview('legacy');await open();await menu.getByRole('menuitem',{name:'另存为…',exact:true}).click();
  check(await page.getByText('封面已保存',{exact:true}).count()===0,'cancel produced success');await menuClosed();results.push('cancel save-as stays quiet');
  await load();await page.evaluate(()=>window.coverSaveTest.copyError=true);await preview('youtube');await open();await menu.getByRole('menuitem',{name:'复制封面',exact:true}).click();
  await page.getByText('无法复制封面，请稍后重试',{exact:true}).waitFor();await menuClosed();results.push('clipboard error stays visible in preview');
  await load();await page.evaluate(()=>window.coverSaveTest.copyPending=true);await preview('youtube');await open();await menu.getByRole('menuitem',{name:'复制封面',exact:true}).click();await open();
  check(await menu.getByRole('menuitem',{name:'复制中…',exact:true}).isDisabled(),'pending copy enabled');check(await menu.getByRole('menuitem',{name:'另存为…',exact:true}).isDisabled(),'save-as enabled during copy');
  await page.keyboard.press('Escape');await page.evaluate(()=>window.coverSaveTest.releaseCopy());await page.getByText('封面已复制到剪贴板',{exact:true}).waitFor();results.push('pending preview copy prevents duplicate operations');
  await image.focus();await page.keyboard.press('Shift+F10');await menu.waitFor();await page.keyboard.press('Escape');await menuClosed();
  await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});check(await trigger('youtube').evaluate(node=>document.activeElement===node),'dialog focus did not return');results.push('keyboard preview menu and layered Escape/focus');
  await preview('youtube');await dialog.getByRole('heading',{name:'封面预览',exact:true}).click({button:'right'});check(await menu.count()===0,'preview header got image menu');results.push('preview context menu is image-only');
  return {passed:results.length,cases:results};
}
