// Synthetic credentials and IPC only. No account or network is accessed.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());await page.reload();
    const open=async()=>{
      await page.getByRole('button',{name:'系统设置',exact:true}).click();
      await page.getByRole('button',{name:'平台账号',exact:true}).click();
      await page.getByRole('tab',{name:'B 站',exact:true}).click();
    };
    await open();
    const panel=()=>page.getByRole('tabpanel',{name:'B 站',exact:true}).locator('form');
    await panel().getByRole('tab',{name:'手动 Cookie',exact:true}).click();
    await panel().locator('#bilibili-cookie:enabled').fill('SESSDATA=fixture-manual');
    await panel().getByRole('button',{name:'保存并切换',exact:true}).click();
    await panel().getByRole('status').filter({hasText:'当前使用：手动 Cookie'}).waitFor();
    await panel().getByRole('tab',{name:'内置登录',exact:true}).click();
    await panel().getByRole('button',{name:'登录B 站',exact:true}).click();
    await panel().getByRole('status').filter({hasText:'当前使用：内置登录'}).waitFor();
    check(await page.evaluate(()=>window.platformSettingsTest.loginCalls.join(',')==='bilibili'),'wrong login platform');
    await panel().getByRole('tab',{name:'手动 Cookie',exact:true}).click();
    check(await panel().locator('#bilibili-cookie').inputValue()==='SESSDATA=fixture-manual','login overwrote manual Cookie');
    await panel().getByRole('button',{name:'切换到手动 Cookie',exact:true}).click();
    await panel().getByRole('status').filter({hasText:'当前使用：手动 Cookie'}).waitFor();
    await page.reload();await open();
    await panel().locator('#bilibili-cookie:enabled').waitFor();
    check(await panel().locator('#bilibili-cookie').inputValue()==='SESSDATA=fixture-manual','Cookie did not survive reload');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'settings viewport overflow');
    results.push(`${width}x${height}: Bilibili manual Cookie, login, switch, persistence and layout`);
  }
  await page.goto(base+'/tests/fixtures/task-platforms.html?bilibili');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  const tabs=page.getByRole('tablist',{name:'按平台筛选下载任务'});
  await tabs.getByRole('tab',{name:'B 站',exact:true}).click();
  await page.getByText('bilibili-done',{exact:true}).waitFor();
  check(!await page.getByText('youtube-done',{exact:true}).isVisible(),'Bilibili filter included YouTube');
  check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'task filters viewport overflow');
  results.push('Bilibili task filter isolates its downloads');
  return results;
}
