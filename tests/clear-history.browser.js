// Existing Playwright CLI + npm run dev. All IPC/file operations are isolated mocks.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const clear=page.locator('header button[aria-label="清理历史"]');
  const search=page.getByRole('textbox',{name:'搜索作品标题或作者'});
  const dialog=page.getByRole('alertdialog');
  const calls=()=>page.evaluate(()=>window.backgroundTest.calls);
  const results=[];
  const load=async scenario=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await search.waitFor();
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load('clear');
    await page.locator('.task-row').filter({has:page.getByText('已完成作品',{exact:true})}).click({button:'right',position:{x:200,y:30}});
    const remove=page.getByRole('menuitem',{name:'从列表移除',exact:true});
    await remove.waitFor();
    check(await remove.evaluate(item=>item.firstElementChild?.tagName.toLowerCase()==='svg'
      && item.textContent.trim()==='从列表移除'),'remove must remain a named menu action');
    await page.keyboard.press('Escape');
    const buttonBox=await clear.boundingBox();const searchBox=await search.boundingBox();
    check(buttonBox.x<searchBox.x,'button must precede search');
    check(await clear.evaluate(button=>button.textContent.trim()==='' && !!button.querySelector('svg')),'expected icon only');
    await clear.hover();await page.getByRole('tooltip').filter({hasText:'清理全部平台下载历史'}).waitFor();
    await page.keyboard.press('Escape');await clear.focus();await page.keyboard.press('Enter');
    await dialog.getByRole('heading',{name:'清理下载历史？'}).waitFor();
    check((await calls()).length===0,'clear before confirmation');
    await dialog.getByRole('button',{name:'返回',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='清理历史');
    check(await clear.evaluate(button=>button===document.activeElement),'cancel focus restoration');
    await search.fill('已完成作品');await clear.click();
    await dialog.getByRole('button',{name:'清理历史',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    check((await calls()).filter(call=>call.type==='clear-history').length===1,'expected one batch IPC');
    check(!(await calls()).some(call=>call.type==='delete-file'),'file deletion requested');
    await page.waitForFunction(()=>document.querySelectorAll('[data-task-row][data-removing]').length===0);
    await search.fill('');
    for(const title of ['进行中的作品','等待中的作品','解析中的下载'])await page.getByText(title,{exact:true}).waitFor();
    for(const title of ['已完成作品','失败作品','取消作品'])check(await page.getByText(title,{exact:true}).count()===0,'history survived clear');
    await page.evaluate(()=>window.backgroundTest.replay());
    await search.focus();await page.keyboard.press('Tab');
    check(await page.getByText('已完成作品',{exact:true}).count()===0,'late event restored cleared record');
    check(await clear.isDisabled(),'no history should disable clear');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    await page.screenshot({path:`.local/clear-history-${width}.png`,animations:'disabled'});
    results.push(width+' remove-icon + clear-history confirmation regression');
  }
  await load('clear-empty');check(await clear.isDisabled(),'empty history enabled');
  await clear.locator('..').hover();await page.getByRole('tooltip').filter({hasText:'暂无可清理'}).waitFor();results.push('empty state');
  await load('clear-failure');await clear.click();await dialog.getByRole('button',{name:'清理历史',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').waitFor();
  check(await dialog.isVisible(),'failure closed confirmation');
  check((await page.evaluate(()=>window.backgroundTest.snapshot())).some(t=>t.id==='done'),'failure changed records');
  await dialog.getByRole('button',{name:'返回',exact:true}).click();results.push('failure retains records and retry');
  await load('clear-pending');await clear.click();await dialog.getByRole('button',{name:'清理历史',exact:true}).click();
  await page.waitForFunction(()=>window.backgroundTest.calls.some(call=>call.type==='clear-history'));
  check(await dialog.getByRole('button',{name:'清理中…',exact:true}).isDisabled(),'pending action enabled');
  await page.keyboard.press('Escape');check(await dialog.isVisible(),'pending escape dismissed');
  check((await calls()).filter(c=>c.type==='clear-history').length===1,'duplicate pending batch');
  await page.evaluate(()=>window.backgroundTest.release());await dialog.waitFor({state:'hidden'});
  await page.waitForFunction(()=>document.activeElement?.getAttribute('role')==='tabpanel');
  results.push('pending guard and final focus');
  return {passed:results.length,cases:results};
}
