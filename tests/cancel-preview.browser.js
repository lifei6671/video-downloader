// Real App, isolated IPC. No platform request, download or user file is touched.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const input=page.locator('#video-input');
  const dialog=page.getByRole('alertdialog',{name:'取消当前任务？'});
  const check=(ok,message)=>{if(!ok)throw Error(message+' @ '+page.url())};
  const cases=[];
  const load=async scenario=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'后台下载',exact:true}).waitFor();
    await page.waitForFunction(()=>document.querySelector('#video-input')?.value.length>0);
  };
  const cancel=async()=>{
    await page.getByRole('button',{name:'取消',exact:true}).click();
    await dialog.getByRole('button',{name:'确认取消',exact:true}).click();
    await page.locator('[data-slot="alert-dialog-content"]').waitFor({state:'detached'});
  };
  const empty=async(expected='',focus=true)=>{
    await page.getByText('粘贴链接开始解析',{exact:true}).waitFor();
    check(await input.inputValue()===expected,'wrong input after cancellation');
    if(focus)await page.waitForFunction(()=>document.activeElement?.id==='video-input');
    check(await page.getByRole('button',{name:'后台下载',exact:true}).count()===0,'canceled preview remained visible');
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load('douyin');
    const original=await input.inputValue();
    await page.getByRole('button',{name:'取消',exact:true}).click();
    await dialog.getByRole('button',{name:'返回',exact:true}).click();
    await page.locator('[data-slot="alert-dialog-content"]').waitFor({state:'detached'});
    check(await input.inputValue()===original,'return from confirmation cleared input');
    check(await page.getByRole('button',{name:'后台下载',exact:true}).isVisible(),'return discarded preview');
    await page.getByRole('button',{name:'取消',exact:true}).focus();await page.keyboard.press('Enter');
    await dialog.getByRole('button',{name:'确认取消',exact:true}).focus();await page.keyboard.press('Enter');
    await empty();
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    check(await page.evaluate(()=>window.backgroundTest.snapshot().find(t=>t.id==='original')?.status)==='canceled','reset did not cancel authoritative task');
    await page.screenshot({path:'output/playwright/cancel-preview-'+width+'.png'});
    const link='https://www.douyin.com/video/124';await input.fill(link);await input.press('Enter');
    await page.getByText('新作品 1',{exact:true}).waitFor();
    await page.evaluate(()=>window.backgroundTest.stale());
    check(await input.inputValue()===link,'late cancellation cleared new input');
    check(await page.getByText('新作品 1',{exact:true}).isVisible(),'new preview was not restored');
    cases.push(width+'x'+height+': confirmation, keyboard reset/focus, next parse and late event');
  }
  for(const platform of ['telegram','tiktok','youtube','album']) {
    await load(platform);await cancel();await empty();cases.push(platform+' preview resets');
  }
  await load('multiple');await cancel();await empty();
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByRole('button',{name:'下载作品',exact:true}).click();await empty('',false);
  cases.push('older restored previews stay hidden after navigation');
  await load('cancel-preview-no-event');await cancel();await empty();
  await page.evaluate(()=>window.backgroundTest.stale());await empty();
  const next='https://t.me/c/100/10';await input.fill(next);await input.press('Enter');
  await page.getByText('新作品 1',{exact:true}).waitFor();
  check(await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='cancel').length)===1,'next parse canceled the acknowledged preview again');
  cases.push('acknowledgement resets without an event and stale events stay hidden');
  {
    await load('cancel-preview-pending');const before=await input.inputValue();await cancel();
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='cancel'));
    check(await input.inputValue()===before,'input reset before cancellation acknowledgement');
    check(await page.getByText('原作品',{exact:true}).isVisible(),'preview reset before acknowledgement');
    check(await input.isDisabled(),'input enabled during cancellation acknowledgement');
    await page.evaluate(()=>window.backgroundTest.release());await empty();
    cases.push('pending acknowledgement locks input and preserves original state until success');
  }
  await load('cancel-preview-error');const before=await input.inputValue();await cancel();
  await page.getByRole('alert').filter({hasText:'网络连接失败'}).waitFor();
  check(await input.inputValue()===before && await page.getByRole('button',{name:'后台下载',exact:true}).isEnabled(),'cancel failure discarded preview/input');
  cases.push('cancel failure retains preview and error');
  await load('cancel-preview-parsing');await input.fill(next);await input.press('Enter');
  await page.getByText('正在获取作品信息',{exact:true}).waitFor();await cancel();await empty();
  cases.push('unsubmitted parsing cancellation resets');
  await load('queued');const home=await input.inputValue();
  await page.getByRole('button',{name:'下载任务',exact:true}).click();await cancel();
  await page.getByRole('button',{name:'下载作品',exact:true}).click();
  check(await input.inputValue()===home && await page.getByRole('button',{name:'后台下载',exact:true}).isVisible(),'task-list cancellation reset an unrelated home preview');
  cases.push('task-list cancellation keeps unrelated home preview');
  await load('normal');await input.fill(next);await input.press('Enter');
  await page.getByText('新作品 1',{exact:true}).waitFor();
  await page.getByRole('button',{name:'立即下载',exact:true}).click();await cancel();
  await page.getByRole('button',{name:'重试',exact:true}).waitFor();
  check(await input.inputValue()===next && await page.getByText('新作品 1',{exact:true}).isVisible(),'submitted download cancellation lost retry preview');
  cases.push('submitted download cancellation keeps retry behavior');
  return {passed:cases.length,cases};
}
