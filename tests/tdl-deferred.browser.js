// Run with the existing Playwright CLI against npm run dev; all IPC is isolated mock data.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  await page.goto(base+'/tests/fixtures/tdl-deferred.html');
  await page.waitForFunction(()=>window.deferredTest.loaded);
  const input=page.getByRole('textbox',{name:'作品分享链接',exact:true});
  const parse=page.locator('form').filter({has:input}).getByRole('button',{name:/^(解析链接|重新解析)$/});
  await input.fill('https://t.me/new_channel/9');
  await parse.focus();await page.keyboard.press('Enter');
  const warning=page.getByRole('alert').filter({hasText:'当前 Telegram 服务正被其他任务占用'});
  await warning.waitFor();
  const queue=page.getByRole('button',{name:'加入待下载队列',exact:true});
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});
    await queue.scrollIntoViewIfNeeded();
    check(await queue.isVisible(),'queue action hidden');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:`.local/tdl-deferred-warning-${width}.png`,animations:'disabled'});
  }
  await queue.focus();await page.keyboard.press('Enter');
  await page.waitForFunction(()=>window.deferredTest.snapshot().find(t=>t.id==='pending-1').status==='queued');
  check(await input.inputValue()==='','submitted link should leave preview');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByText('等待解析',{exact:true}).waitFor();
  check(await page.getByText('等待 Telegram 服务空闲后自动解析并下载',{exact:false}).count()>0,'missing deferred explanation');
  check(await page.evaluate(()=>window.deferredTest.snapshot().find(t=>t.id==='running').status==='downloading'),'existing task interrupted');
  check((await page.evaluate(()=>window.deferredTest.calls)).filter(c=>c.type==='resolve').length===1,'queue reparsed prematurely');
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'queue view overflow');
    await page.screenshot({path:`.local/tdl-deferred-queue-${width}.png`,animations:'disabled'});
  }
  const row=page.locator('[data-slot="card"]').filter({hasText:'https://t.me/new_channel/9'});
  await row.getByRole('button',{name:'取消',exact:true}).click();
  await page.getByRole('button',{name:'确认取消',exact:true}).click();
  await row.getByText('已取消',{exact:true}).waitFor();
  await row.getByRole('button',{name:'重试',exact:true}).click();
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await row.getByText('等待解析',{exact:true}).waitFor();
  await page.evaluate(()=>window.deferredTest.advance('pending-1'));
  await row.getByText('解析中',{exact:true}).waitFor();
  await page.evaluate(()=>window.deferredTest.fail('pending-1'));
  await row.getByRole('alert').waitFor();
  await row.getByRole('button',{name:'重试',exact:true}).click();
  await page.evaluate(()=>window.deferredTest.ready('pending-1'));
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByText('新频道作品',{exact:true}).waitFor();
  check((await page.evaluate(()=>window.deferredTest.calls)).filter(c=>c.type==='cancel').every(c=>c.id==='pending-1'),'canceled another task');
  return {passed:true,keyboard:true,viewports:3,cases:['busy warning and queue','no early resolve','independent cancel','same-task retry','deferred parse and failure','download display']};
}
