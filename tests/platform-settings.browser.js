// Run using the existing Playwright CLI run-code --filename with npm run dev.
// IPC persistence is isolated in sessionStorage; real OS/browser credentials are untouched.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const browser=()=>page.getByRole('checkbox',{name:/使用系统默认浏览器 Cookie/});
  const cookie=()=>page.getByRole('textbox',{name:'网页端 Cookie',exact:true});
  const proxy=()=>page.getByRole('textbox',{name:'HTTP / SOCKS5 地址',exact:true});
  const save=()=>page.getByRole('button',{name:'保存平台配置',exact:true});
  const snapshot=()=>page.evaluate(()=>window.platformSettingsTest.snapshot());
  const accounts=async(loaded=true)=>{
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    if(loaded)await page.waitForFunction(()=>!document.querySelector('#youtube-clear')?.disabled, null, {timeout:10000});
  };
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());
    await page.reload();await accounts();
  };
  const notice=async(type,text)=>{
    const notification=page.locator('[data-sonner-toast][data-type="'+type+'"]').filter({hasText:text});
    await notification.waitFor();
    check(await page.locator('form').getByText(text,{exact:true}).count()===0,'save result rendered inline');
    await notification.getByRole('button',{name:'关闭通知',exact:true}).click();
    await notification.waitFor({state:'hidden'});
  };
  const saved=()=>notice('success','平台配置已保存，仅影响新任务');
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await fresh();
    check(!await browser().isChecked(),'manual Cookie should start unchecked');
    check(await cookie().inputValue()==='session=fixture','manual Cookie missing');
    await browser().focus();await page.keyboard.press('Space');
    check(await cookie().isDisabled(),'browser mode must disable manual Cookie');
    await save().click();await saved();
    check(await browser().isChecked(),'save reset browser mode');
    check(await save().isDisabled(),'saved browser mode remains dirty');
    check((await snapshot()).cookies.youtube==='','manual Cookie was not cleared');
    await page.getByRole('tab',{name:'TikTok',exact:true}).click();
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    await page.waitForFunction(()=>!document.querySelector('#youtube-clear')?.disabled, null, {timeout:10000});
    check(await browser().isChecked(),'tab switch lost browser mode');
    await page.reload();await accounts();
    check(await browser().isChecked(),'reload lost browser mode');
    await page.getByRole('radio',{name:'自定义代理',exact:true}).click();
    await proxy().fill('  socks5://127.0.0.1:7890  ');
    check(await proxy().getAttribute('type')==='text','proxy address is masked');
    await save().click();await saved();
    check((await snapshot()).settings.profiles.find(p=>p.platform==='youtube').proxy==='socks5://127.0.0.1:7890/','proxy not trimmed/persisted');
    check(await proxy().inputValue()==='socks5://127.0.0.1:7890','saved proxy address disappeared from editor');
    check(await browser().isChecked(),'proxy save changed browser mode');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:'.local/platform-settings-'+width+'.png',animations:'disabled'});
    await page.getByRole('radio',{name:'直连',exact:true}).click();
    await save().click();await saved();
    check((await snapshot()).settings.profiles.find(p=>p.platform==='youtube').proxy===null,'direct mode did not clear proxy');
    await browser().click();await cookie().fill('session=manual-new');
    await cookie().press('Tab');
    await save().click();await saved();
    await page.reload();await accounts();
    check(!await browser().isChecked() && await cookie().inputValue()==='session=manual-new','manual mode did not survive reload');
    check((await snapshot()).cookies.tiktok==='','platform isolation lost');
    cases.push(width+': keyboard/browser mode/save/tab/reload/proxy/manual Cookie/isolation/layout');
  }
  await fresh();
  for(const [platform,name] of [['douyin','抖音'],['tiktok','TikTok'],['youtube','YouTube']]) {
    await page.getByRole('tab',{name,exact:true}).click();
    await page.waitForFunction(id=>!document.querySelector('#'+id+'-clear')?.disabled, platform, {timeout:10000});
    if(await browser().isChecked())await browser().click();
    const expected='session='+platform+'-fixture; preference=a b';
    await cookie().fill('\r\n \t '+expected+' \r\n\t');
    await save().click();await saved();
    check(await cookie().inputValue()===expected && await save().isDisabled(),'trimmed Cookie not reflected in editor');
    check((await snapshot()).cookies[platform]===expected,'Cookie boundary whitespace not removed');
    await page.reload();await accounts();
    await page.getByRole('tab',{name,exact:true}).click();
    await page.waitForFunction(id=>!document.querySelector('#'+id+'-clear')?.disabled, platform, {timeout:10000});
    check(await cookie().inputValue()===expected && !await browser().isChecked(),'normalized Cookie did not survive reload');
    await cookie().fill('session=one\nother=two');await save().click();
    await notice('error','配置格式有误');
    check((await snapshot()).cookies[platform]===expected,'internal newline corrupted saved Cookie');
    await cookie().fill('\n \t\n');await save().click();await saved();
    check(await browser().isChecked() && await cookie().inputValue()==='' && await save().isDisabled(),'whitespace-only Cookie did not restore browser mode');
    check((await snapshot()).cookies[platform]==='','whitespace-only Cookie persisted');
    cases.push(platform+': boundary trim/reload/internal-newline rejection/whitespace-only browser mode');
  }
  await fresh();await browser().click();
  await page.evaluate(()=>window.platformSettingsTest.failSave=true);
  await save().click();await notice('error','无法保存任务记录');
  check((await snapshot()).cookies.youtube==='session=fixture','failed save changed persisted Cookie');
  check(await browser().isChecked() && !await save().isDisabled(),'failed save lost draft');
  await page.evaluate(()=>window.platformSettingsTest.failSave=false);
  await save().click();await saved();cases.push('failed save preserves stored Cookie and retryable draft');
  await fresh();await page.getByRole('radio',{name:'自定义代理',exact:true}).click();
  await proxy().fill('7890');await browser().click();await save().click();
  await notice('error','配置格式有误');
  check((await snapshot()).cookies.youtube==='session=fixture','invalid proxy cleared Cookie');
  await proxy().fill('http://127.0.0.1:7890');await save().click();await saved();
  cases.push('invalid proxy save preserves Cookie; corrected address saves browser mode');
  await fresh();
  await page.evaluate(()=>window.platformSettingsTest.failCookie=true);
  await page.getByRole('button',{name:'下载任务',exact:true}).click();await accounts(false);
  await page.getByRole('alert').filter({hasText:'Cookie'}).waitFor();
  await page.getByRole('textbox',{name:'User-Agent',exact:true}).fill('updated-fixture-agent');
  await save().click();await saved();
  const last=await page.evaluate(()=>window.platformSettingsTest.calls.at(-1));
  check(last.cookie===null && (await snapshot()).cookies.youtube==='session=fixture','Cookie read failure overwrote credentials');
  cases.push('Cookie read failure preserves stored credentials while saving other settings');
  await page.getByRole('button',{name:'下载中心',exact:true}).click();
  await page.getByRole('textbox').first().fill('https://www.youtube.com/watch?v=a6cnYFcd7JQ');
  await page.getByRole('button',{name:'解析链接',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'切换 TUN/代理出口'}).waitFor();
  cases.push('risk-control error gives verification and TUN exit guidance');
  return {passed:cases.length,cases};
}
