// Production App with isolated task snapshots; no native account or media requests.
async page => {
  const base = page.url().split('/').slice(0,3).join('/');
  const input = page.locator('#video-input');
  const check = (value,message) => {if(!value)throw Error(message)};
  const load = async mode => {
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=preview-restart-'+mode);
    await page.waitForFunction(()=>window.backgroundTest?.loaded);
    await page.waitForFunction(()=>Array.from(document.querySelectorAll('#video-input')).some(input=>Array.from(input.closest('form').querySelectorAll('button')).some(button=>/^(解析链接|重新解析)$/.test(button.textContent.trim())&&!button.disabled)));
  };
  const empty = async () => {
    check(await input.inputValue()==='', 'restart restored a handed-off source link');
    check(await page.locator('.media-preview').count()===0, 'restart restored an older parsed preview');
    check(await input.isEnabled(), 'background work blocked the new share link input');
  };
  const results = [];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for(const state of ['failed','queued','downloading','ready','completed']) {
      await load(state);await empty();
      await page.getByRole('button',{name:'下载任务',exact:true}).click();
      await page.locator('[data-task-id="original"]').waitFor();
      if(state==='failed')await page.locator('[data-task-id="original"]').getByText('已中断',{exact:true}).waitFor();
      results.push({width,state,emptyHomepage:true,taskRetained:true});
    }
    await load('auto');await empty();
    await page.evaluate(()=>window.backgroundTest.resumeRestart());await empty();
    await page.evaluate(()=>window.backgroundTest.complete('original'));await empty();
    results.push({width,state:'automatic-resume-and-complete',emptyHomepage:true,taskRetained:true});
    await page.evaluate(()=>sessionStorage.removeItem('preview-restart-tasks'));
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=preview-restart-live');
    await page.getByRole('button',{name:'后台下载',exact:true}).waitFor();
    await page.getByRole('button',{name:'后台下载',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('#video-input').value==='');
    await page.evaluate(()=>window.backgroundTest.saveRestart());
    await page.reload();
    await page.waitForFunction(()=>window.backgroundTest?.loaded);
    await page.locator('form').filter({has:input}).getByRole('button',{name:'解析链接',exact:true}).waitFor();
    await empty();
    await input.fill('https://t.me/c/100/11');
    await page.locator('form').filter({has:input}).getByRole('button',{name:'解析链接',exact:true}).click();
    await page.getByText('新作品 1',{exact:true}).waitFor();
    check(!await page.evaluate(()=>window.backgroundTest.calls.some(call=>call.type==='cancel'&&call.id==='original')), 'new parsing canceled interrupted background task');
    results.push({width,state:'handoff-quit-restart-new-parse',emptyHomepage:true,taskRetained:true});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=preview-restart-draft');
    await page.getByText('尚未提交的最新预览',{exact:true}).waitFor();
    check(await input.inputValue()==='https://t.me/c/100/10', 'latest unsubmitted draft was not restored');
    results.push({width,state:'new-unsubmitted-draft',draftRestored:true});
  }
  return {passed:results.length,results};
}
