// Run with the existing Playwright CLI after `npm run dev`:
// playwright-cli -s=background-download open http://127.0.0.1:1420/tests/fixtures/background-download.html
// $code = Get-Content tests/background-download.browser.js -Raw
// playwright-cli -s=background-download run-code $code
async page => {
  const fixture = page.url().split('?')[0];
  const input = page.locator('#video-input');
  const background = page.getByRole('button', {name:'后台下载',exact:true});
  const parse = page.locator('form').filter({has:input}).getByRole('button', {name:/^(解析链接|重新解析)$/});
  const dialog = page.getByRole('alertdialog');
  const check = (condition, message) => {if(!condition)throw Error(message+' @ '+page.url())};
  const calls = () => page.evaluate(()=>window.backgroundTest.calls);
  const load = async scenario => {
    await page.goto(fixture+'?scenario='+scenario);
    await page.waitForFunction(()=>window.backgroundTest?.loaded);
    await background.waitFor();
  };
  const empty = async (focus = true) => {
    await page.getByText('粘贴链接开始解析',{exact:true}).waitFor();
    check(await input.inputValue()==='', 'input was not reset');
    if(focus)await page.waitForFunction(()=>document.activeElement?.id==='video-input');
    await page.getByRole('heading',{name:'下载中心',exact:true}).waitFor();
  };
  const results=[];
  for(const platform of ['telegram','douyin','tiktok','youtube','album']) {
    await load(platform);
    const order=await page.locator('[data-background-download-task], [data-download-task]').evaluateAll(nodes=>nodes.map(n=>n.textContent.trim()));
    check(order.join(',')==='后台下载,立即下载','button order and labels');
    check(await background.getAttribute('data-variant')==='default','background action is not primary');
    check(await page.locator('[data-download-task]').getAttribute('data-variant')==='outline','immediate action is highlighted');
    await background.click();await empty();
    check((await calls()).filter(c=>c.type==='download').length===1,'unexpected submission count');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.getByText('原作品',{exact:true}).waitFor();
    check(await background.count()===0,'background action should only be on preview');
    results.push(platform);
  }
  await load('multiple');await background.click();await empty();results.push('reset hides older restored previews');
  await load('queued');await background.click();await empty();
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.locator('.task-row-status').getByText('队列第 1 位',{exact:true}).waitFor();results.push('existing queue');
  await load('failure');const originalInput=await input.inputValue();
  await background.click();await page.getByRole('alert').filter({hasText:'网络连接失败'}).waitFor();
  check(await input.inputValue()===originalInput && await background.isEnabled(),'failure discarded preview');
  await page.getByText('原作品',{exact:true}).waitFor();results.push('failure retains preview');
  await load('pending');await background.click();
  await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='download'));
  check(!await background.isEnabled(),'pending submit not disabled');
  check(await input.inputValue()!=='','reset before acknowledgement');
  check(await input.isDisabled(),'input enabled during acknowledgement');
  await input.evaluate(node=>node.form.requestSubmit());
  check(!(await calls()).some(c=>c.type==='resolve'||c.type==='cancel'),'Enter bypassed busy guard during background acknowledgement');
  check(await page.getByText('原作品',{exact:true}).count()===1,'pending Enter replaced original preview');
  await page.evaluate(()=>window.backgroundTest.release());await empty();
  check((await calls()).filter(c=>c.type==='download').length===1,'double submit');results.push('acknowledgement and busy');
  await load('delayed');await background.click();await empty();
  await page.evaluate(()=>window.backgroundTest.stale());
  await input.fill('https://t.me/c/100/10');await parse.click();
  await page.getByText('新作品 1',{exact:true}).waitFor();
  check(!(await calls()).some(c=>c.type==='cancel'&&c.id==='original'),'next parse canceled background task');
  await page.evaluate(()=>window.backgroundTest.flush());
  await page.evaluate(()=>window.backgroundTest.complete('original'));
  await page.getByText('新作品 1',{exact:true}).waitFor();
  check(await input.inputValue()==='https://t.me/c/100/10','old completion cleared new input');results.push('late events and next parse');
  await load('refresh-error');await background.click();await empty();results.push('accepted despite optional refresh failure');
  await load('duplicate');await input.fill('https://t.me/c/100/9');await parse.click();await dialog.waitFor();await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
  await background.click();await dialog.waitFor();
  await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
  check(await input.inputValue()!=='' && (await calls()).every(c=>c.type!=='download'),'duplicate cancel reset/submitted');
  await page.waitForFunction(()=>document.activeElement?.textContent.trim()==='后台下载');
  await background.click();await dialog.getByRole('button',{name:'重新下载',exact:true}).click();await empty();
  check((await calls()).filter(c=>c.type==='download').every(c=>c.confirm),'duplicate bypassed confirmation');results.push('duplicate consent and focus');
  await load('race');await input.fill('https://t.me/c/100/9');await parse.click();await page.getByText('新作品 1',{exact:true}).waitFor();await background.click();await dialog.waitFor();
  check(await input.inputValue()!=='','race reset without confirmation');
  await dialog.getByRole('button',{name:'重新下载',exact:true}).click();await empty();
  check((await calls()).filter(c=>c.type==='download').map(c=>c.confirm).join(',')==='false,true','race intent lost');results.push('completion-record race');
  await load('normal');await input.fill('https://t.me/c/100/10');await parse.click();
  await page.getByText('新作品 1',{exact:true}).waitFor();
  await page.getByRole('button',{name:'立即下载',exact:true}).click();
  await page.getByText('新作品 1',{exact:true}).waitFor();
  check(await input.inputValue()!=='','normal download reset too soon');
  await page.evaluate(()=>window.backgroundTest.complete('next-1'));await empty(false);results.push('normal download unchanged');
  await load('telegram');
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await background.scrollIntoViewIfNeeded();
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:`output/playwright/background-download-${width}.png`,animations:'disabled'});
  }
  await background.focus();await page.keyboard.press('Enter');await empty();results.push('3 viewports and keyboard');
  return {passed:results.length,cases:results};
}
