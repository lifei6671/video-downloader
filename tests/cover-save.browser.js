// Run with the existing Playwright CLI run-code --filename, with npm run dev.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const menu=page.getByRole('menu',{name:'封面操作',exact:true});
  const save=menu.getByRole('menuitem',{name:'保存封面',exact:true});
  const trigger=id=>page.locator('[data-slot="card"]').filter({has:page.getByText(id+' cover',{exact:true})}).locator('[data-cover-context-menu]');
  const load=async()=>{await page.goto(base+'/tests/fixtures/cover-save.html');await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.waitForFunction(()=>[...document.querySelectorAll('[data-cover-context-menu] img')].length===6)};
  const open=async id=>{await trigger(id).click({button:'right',force:true});await menu.waitFor()};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    await trigger('douyin').getByRole('img').click({button:'right'});await menu.waitFor();
    check((await menu.getByRole('menuitem').allTextContents()).join(',')==='保存封面,复制封面','unexpected menu entries');
    const bounds=await menu.boundingBox();check(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=width&&bounds.y+bounds.height<=height,'menu outside viewport');
    await save.click();await page.getByText('封面已保存',{exact:true}).waitFor();
    check((await page.evaluate(()=>window.coverSaveTest.calls)).join(',')==='douyin','wrong task exported');
    results.push('right-click image/save/viewport '+width);
  }
  await load();
  for(const id of ['tiktok','youtube','telegram','legacy','album']) {await open(id);check(await save.isEnabled(),'save disabled for '+id);await save.click()}
  check((await page.evaluate(()=>window.coverSaveTest.calls)).join(',')==='tiktok,youtube,telegram,legacy,album','platform/legacy/album export mismatch');
  results.push('platforms, queued task, legacy cover and album first image');
  for(const id of ['missing','broken','broken-album']) {await open(id);check(await save.isDisabled(),'unavailable cover export enabled');await page.keyboard.press('Escape')}
  results.push('placeholder and failed cover disabled');
  await load();await page.evaluate(()=>window.coverSaveTest.result=false);await open('douyin');await save.click();
  check(await page.getByText('封面已保存',{exact:true}).count()===0,'cancel produced success toast');
  results.push('cancel stays quiet');
  await page.evaluate(()=>window.coverSaveTest.error=true);await open('douyin');await save.click();
  await page.getByText('文件操作失败，请检查目录权限、文件占用和磁盘空间',{exact:true}).waitFor();
  results.push('save failure is visible');
  await load();await page.evaluate(()=>window.coverSaveTest.pending=true);await open('douyin');await save.click();
  await open('douyin');check(await menu.getByRole('menuitem',{name:'保存中…',exact:true}).isDisabled(),'pending save enabled');
  await page.keyboard.press('Escape');await page.evaluate(()=>window.coverSaveTest.release());
  await page.getByText('封面已保存',{exact:true}).waitFor();results.push('pending save cannot repeat');
  await load();await trigger('douyin').focus();await page.keyboard.press('Shift+F10');await menu.waitFor();
  await page.keyboard.press('Escape');await menu.waitFor({state:'hidden'});results.push('keyboard context menu and Escape');
  await page.getByText('douyin cover',{exact:true}).click({button:'right'});
  check(await menu.count()===0,'context menu enabled outside cover');results.push('cover-only scope');
  return {passed:results.length,cases:results};
}
