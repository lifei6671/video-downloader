// Isolated IPC; no real downloads or clipboard access.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const input = page.locator('#video-input');
  const form = page.locator('form').filter({has:input});
  const menu = page.getByRole('menu',{name:'作品链接编辑'});
  const check = (ok,message) => {if(!ok)throw Error(message)};
  const cases = [];
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for (const status of ['ready','failed','canceled']) {
      await page.goto(base+'/tests/fixtures/background-download.html?scenario=parse-controls');
      await page.getByRole('button',{name:'后台下载',exact:true}).waitFor();
      const link = 'https://t.me/c/100/10';
      await input.fill(link);
      // A context paste started before parsing must not change the submitted link.
      await input.click({button:'right'});await menu.waitFor();
      await menu.getByRole('menuitem',{name:'粘贴',exact:true}).click();
      await page.waitForFunction(()=>typeof window.backgroundTest.releaseClipboard==='function');
      await form.getByRole('button',{name:'重新解析',exact:true}).click();
      await page.waitForFunction(()=>typeof window.backgroundTest.releaseResolve==='function');
      check(await input.isDisabled(),'input unlocked while resolve request is pending');
      await page.evaluate(()=>window.backgroundTest.releaseClipboard('迟到的剪贴板'));
      await input.evaluate(node=>node.form.requestSubmit());
      check(await input.inputValue()===link,'late clipboard overwrote parsing input');
      check(await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='resolve').length)===1,'duplicate resolve request');
      await page.evaluate(()=>window.backgroundTest.releaseResolve());
      await page.getByText('正在获取作品信息',{exact:true}).waitFor();
      check(await input.isDisabled(),'input unlocked while work is parsing');
      check(await form.getByRole('button',{name:/正在解析…/}).isDisabled(),'parse action enabled');
      check(await form.getByRole('button',{name:'粘贴链接',exact:true}).isDisabled(),'paste action enabled');
      check(await form.getByRole('combobox',{name:'解析引擎',exact:true}).isDisabled(),'resolver selection enabled');
      await input.dispatchEvent('contextmenu',{clientX:350,clientY:210});await menu.waitFor();
      check(await menu.getByRole('menuitem').evaluateAll(nodes=>nodes.every(node=>node.getAttribute('aria-disabled')==='true')),'context edit action enabled');
      await page.keyboard.press('Escape');
      check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
      if(status==='ready')await page.screenshot({path:'.local/parse-actions/parsing-'+width+'.png',animations:'disabled'});
      await page.evaluate(status=>window.backgroundTest.finishParsing(status),status);
      await page.waitForFunction(()=>!document.querySelector('#video-input').disabled);
      check(await input.inputValue()===link,'parse result unexpectedly changed input');
      await input.fill(link+'1');
      if(status==='ready') {
        check(await page.getByRole('button',{name:'后台下载',exact:true}).getAttribute('data-variant')==='default','background button is not primary');
        const immediate=page.getByRole('button',{name:'立即下载',exact:true});
        check(await immediate.getAttribute('data-variant')==='outline','immediate button is highlighted');
        await immediate.focus();check(await immediate.evaluate(node=>document.activeElement===node),'immediate button cannot receive keyboard focus');
        await page.screenshot({path:'.local/parse-actions/ready-'+width+'.png',animations:'disabled'});
        await page.evaluate(()=>{window.backgroundTest.fail('next-1');delete window.backgroundTest.releaseClipboard});
        await page.getByRole('button',{name:'重试',exact:true}).waitFor();
        const beforeRetry=await input.inputValue();
        await form.getByRole('button',{name:'粘贴链接',exact:true}).click();
        await page.waitForFunction(()=>typeof window.backgroundTest.releaseClipboard==='function');
        await page.getByRole('button',{name:'重试',exact:true}).click();
        await page.waitForFunction(()=>window.backgroundTest.snapshot().find(t=>t.id==='next-1').status==='parsing' && document.querySelector('#video-input').disabled);
        await page.evaluate(()=>window.backgroundTest.releaseClipboard('迟到的工具栏粘贴'));
        await form.getByRole('button',{name:'粘贴链接',exact:true}).waitFor();
        check(await input.inputValue()===beforeRetry,'late toolbar clipboard overwrote retry parsing input');
        cases.push(width+': retry parsing rejects late toolbar clipboard');
      }
      cases.push(width+': pending request, parsing lock, late paste, '+status+' unlock');
    }
  }
  return {passed:cases.length,cases};
}
