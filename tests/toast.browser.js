// npm run dev, then open /tests/fixtures/toast.html with the existing Playwright CLI.
async page => {
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (value, message) => { if (!value) throw Error(message); };
  const notifications = page.locator('[data-sonner-toast]');
  const results = [];
  const colors = new Set();
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/toast.html');
    for (const type of ['default','success','error','warning','info']) {
      await page.getByRole('button',{name:type,exact:true}).click();
      const item = notifications.filter({has:page.getByRole('button',{name:'关闭通知',exact:true})});
      await item.waitFor();
      const close = item.getByRole('button',{name:'关闭通知',exact:true});
      const metrics = await item.evaluate(node=>{
        const text=node.querySelector('[data-content]').getBoundingClientRect();
        const close=node.querySelector('[data-close-button]').getBoundingClientRect();
        const bounds=node.getBoundingClientRect();
        return {background:getComputedStyle(node).backgroundColor,color:getComputedStyle(node).color,
          upperRight:close.left>bounds.left+bounds.width/2 && close.right<=bounds.right &&
            close.top>=bounds.top && close.bottom<=bounds.top+36,
          overlap:text.right>close.left && text.top<close.bottom,icon:!!node.querySelector('[data-icon]')};
      });
      check(metrics.upperRight,type+' close button outside upper right');
      check(!metrics.overlap,'close overlaps description');
      if(type!=='default') {check(metrics.icon,'missing status icon');colors.add(metrics.background+'|'+metrics.color);}
      if(width===1440)await page.screenshot({path:`.local/toast-${type}.png`,animations:'disabled'});
      await close.focus(); await page.keyboard.press('Enter');
      await notifications.waitFor({state:'detached'});
    }
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    results.push(width+' variants, icons, upper-right close, text spacing and keyboard');
  }
  check(colors.size===4,'status styles are not distinct');
  await page.getByRole('button',{name:'loading',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="loading"]').waitFor();
  await page.getByRole('button',{name:'success',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="success"]').waitFor();
  check(await notifications.count()===1,'loading update duplicated notification');
  await page.getByRole('button',{name:'关闭通知'}).click();await notifications.waitFor({state:'detached'});
  results.push('loading becomes success in place');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=failure');
  await page.getByRole('button',{name:'后台下载',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'操作失败'}).waitFor();
  await page.getByRole('button',{name:'关闭通知'}).click();await notifications.waitFor({state:'detached'});
  await page.getByRole('alert').filter({hasText:'网络连接失败'}).waitFor();
  results.push('action error toast retains persistent error after close');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=telegram');
  await page.getByRole('button',{name:'后台下载',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="success"]').waitFor();
  await page.evaluate(()=>window.backgroundTest.fail('original'));
  const failed = page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'下载失败'});
  await failed.waitFor();
  await failed.getByRole('button',{name:'关闭通知'}).click();
  await failed.waitFor({state:'detached'});
  await page.evaluate(()=>window.backgroundTest.fail('original'));
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByText('网络连接失败，请检查网络或代理设置',{exact:true}).waitFor();
  check(await failed.count()===0,'task error repeated after being dismissed');
  results.push('task error arrives once and stays in task row');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=duplicate');
  await page.getByRole('button',{name:'后台下载',exact:true}).waitFor();
  check(await notifications.count()===0,'replayed a restored completion');
  results.push('restored completion does not replay a toast');
  return {passed:results.length,cases:results};
}
