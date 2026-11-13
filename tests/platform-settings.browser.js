// Run using the existing Playwright CLI run-code --filename with npm run dev.
// IPC persistence is isolated in sessionStorage; real OS/browser credentials are untouched.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const authStatus=()=>page.getByRole('status').filter({hasText:'当前使用'});
  const cookie=()=>page.getByRole('textbox',{name:'网页端 Cookie',exact:true});
  const cookieDisplay=()=>cookie().evaluate(node=>getComputedStyle(node).getPropertyValue('-webkit-text-security'));
  const proxy=()=>page.getByRole('textbox',{name:'HTTP / SOCKS5 地址',exact:true});
  const save=()=>page.getByRole('button',{name:'保存平台配置',exact:true});
  const snapshot=()=>page.evaluate(()=>window.platformSettingsTest.snapshot());
  const accounts=async(loaded=true)=>{
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    if(loaded)await page.waitForFunction(()=>document.querySelector('#youtube-cookie:enabled'));
  };
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());
    await page.reload();await accounts();
  };
  const notice=async(type,text)=>{
    const notification=page.locator('[data-sonner-toast][data-front=true][data-type="'+type+'"]').filter({hasText:text});
    await notification.waitFor();
    check(await page.locator('form').getByText(text,{exact:true}).count()===0,'save result rendered inline');
    await notification.getByRole('button',{name:'关闭通知',exact:true}).click();
    await notification.waitFor({state:'hidden'});
  };
  const saved=()=>notice('success','平台配置已保存，仅影响新任务');
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await fresh();
    check(await cookie().inputValue()==='session=fixture','manual Cookie missing');
    await authStatus().getByText('当前使用：手动 Cookie').waitFor();
    check(await page.getByRole('checkbox',{name:/使用系统默认浏览器 Cookie/}).count()===0,'browser import control remains');
    await page.getByRole('button',{name:'打开 YouTube 登录'}).press('Enter');
    await page.waitForFunction(()=>window.platformSettingsTest.youtubeOpens===1);
    await notice('success','已打开 YouTube 登录页');
    check((await snapshot()).cookies.youtube==='session=fixture','opening external login changed credentials');
    await page.getByRole('radio',{name:'自定义代理',exact:true}).click();
    await proxy().fill('  socks5://127.0.0.1:7890  ');
    check(await proxy().getAttribute('type')==='text','proxy address is masked');
    await save().click();await saved();
    check((await snapshot()).settings.profiles.find(p=>p.platform==='youtube').proxy==='socks5://127.0.0.1:7890/','proxy not trimmed/persisted');
    check(await proxy().inputValue()==='socks5://127.0.0.1:7890','saved proxy address disappeared from editor');
    await authStatus().getByText('当前使用：手动 Cookie').waitFor();
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:'.local/platform-settings-'+width+'.png',animations:'disabled'});
    await page.getByRole('radio',{name:'直连',exact:true}).click();
    await save().click();await saved();
    check((await snapshot()).settings.profiles.find(p=>p.platform==='youtube').proxy===null,'direct mode did not clear proxy');
    await cookie().fill('session=manual-new');
    await page.getByRole('button',{name:'保存 Cookie',exact:true}).press('Enter');
    await notice('success','认证方式已保存');
    await page.reload();await accounts();
    check(await cookie().inputValue()==='session=manual-new','manual Cookie did not survive reload');
    check((await snapshot()).cookies.tiktok==='','platform isolation lost');
    cases.push(width+': YouTube external launch/proxy/manual Cookie/reload/layout');
  }
  await fresh();
  await page.getByRole('tab',{name:'抖音',exact:true}).click();
  const douyinForm=page.locator('#profile-douyin');
  const authTab=name=>douyinForm.getByRole('tab',{name,exact:true});
  const networkPosition=()=>douyinForm.locator('label[for="douyin-bypass-proxy"]').evaluate(label=>{
    const scroller=label.closest('[role=tabpanel]');
    const box=label.getBoundingClientRect();
    const tabs=scroller.querySelector('[role="tablist"]');
    const panel=label.closest('form').querySelector('[role="tabpanel"][data-state="active"]');
    return {x:box.x,y:box.y+scroller.scrollTop,width:scroller.clientWidth,
      tabsY:tabs.getBoundingClientRect().y+scroller.scrollTop,
      gap:box.y-panel.getBoundingClientRect().bottom};
  });
  const initialPosition=await networkPosition();
  check(initialPosition.gap<40,'login leaves a blank area above network settings');
  await authTab('手动 Cookie').press('Enter');
  await cookie().fill('session=fixture');
  const manualPosition=await networkPosition();
  check(manualPosition.width===initialPosition.width && manualPosition.x===initialPosition.x
    && manualPosition.tabsY===initialPosition.tabsY,'switching auth tabs shifted the settings frame');
  check(manualPosition.y>initialPosition.y,'login still reserves room for the manual editor');
  await page.evaluate(()=>window.platformSettingsTest.holdAuth=true);
  await douyinForm.getByRole('button',{name:'保存并切换',exact:true}).click();
  await page.waitForFunction(()=>!!window.platformSettingsTest.releaseAuth);
  check(JSON.stringify(await networkPosition())===JSON.stringify(manualPosition),'pending auth action moved network settings');
  await page.evaluate(()=>window.platformSettingsTest.releaseAuth());
  await notice('success','认证方式已保存');
  await authTab('内置登录').press('Enter');
  check(JSON.stringify(await networkPosition())===JSON.stringify(initialPosition),'returning to login moved network settings');
  cases.push('compact login, stable settings frame and pending save');
  await fresh();
  for(const [platform,name] of [['douyin','抖音'],['xiaohongshu','小红书'],['tiktok','TikTok'],['youtube','YouTube']]) {
    await page.getByRole('tab',{name,exact:true}).click();
    const form=page.locator('#profile-'+platform);
    if(platform!=='youtube')await form.getByRole('tab',{name:'手动 Cookie',exact:true}).press('Enter');
    const expected='session='+platform+'-fixture; preference=a b; padded=abc+/==';
    if(platform==='tiktok') {
      await page.evaluate(()=>window.platformSettingsTest.failSave=true);
      await cookie().fill(expected);
      await form.getByRole('button',{name:'保存并切换'}).click();
      await notice('error','无法保存任务记录');
      check((await snapshot()).cookies.tiktok==='' && await cookie().inputValue()===expected,'failed save overwrote credentials or lost draft');
      await page.evaluate(()=>window.platformSettingsTest.failSave=false);
    }
    const pasted=platform==='douyin'?'douyin.com; '+expected:expected;
    await cookie().fill('\r\n \t '+pasted+' \r\n\t');
    check(await cookieDisplay()==='none','entered Cookie should be displayed as plain text');
    const manualSave=()=>form.getByRole('button',{name:'保存 Cookie',exact:true});
    await form.getByRole('button',{name:platform==='youtube'?'保存 Cookie':'保存并切换',exact:true}).click();
    await notice('success','认证方式已保存');
    await page.waitForFunction(id=>document.querySelector('#'+id+'-cookie:enabled'), platform);
    check(await cookie().inputValue()===expected && await manualSave().isDisabled(),'trimmed Cookie not reflected in editor');
    check(await cookieDisplay()==='none','saving Cookie should keep plain text visible');
    check((await snapshot()).cookies[platform]===expected,'Cookie boundary whitespace not removed');
    if(platform!=='youtube'){
      await form.getByRole('tab',{name:'内置登录',exact:true}).click();
      const login=form.getByRole('button',{name:platform==='tiktok'?'登录 TikTok':'登录'+name,exact:true});
      await page.evaluate(()=>window.platformSettingsTest.cancelLogin=true);
      await login.click();
      await page.waitForFunction(id=>window.platformSettingsTest.loginCalls.includes(id),platform);
      await authStatus().getByText('当前使用：手动 Cookie').waitFor();
      await page.evaluate(()=>window.platformSettingsTest.cancelLogin=false);
      await login.click();
      await authStatus().getByText('当前使用：内置登录').waitFor();
      await form.getByRole('tab',{name:'手动 Cookie',exact:true}).click();
      check(await cookie().inputValue()===expected,'switching to login erased manual Cookie');
      await form.getByRole('button',{name:'切换到手动 Cookie'}).click();
      await authStatus().getByText('当前使用：手动 Cookie').waitFor();
    }
    await page.reload();await accounts();
    await page.getByRole('tab',{name,exact:true}).click();
    await page.waitForFunction(id=>document.querySelector('#'+id+'-cookie:enabled'), platform);
    check(await cookie().inputValue()===expected,'normalized Cookie did not survive reload');
    check(await cookieDisplay()==='none','saved Cookie should be visible after reload');
    await cookie().fill('session=one\nother=two');await manualSave().click();
    await notice('error','配置格式有误');
    check((await snapshot()).cookies[platform]===expected,'internal newline corrupted saved Cookie');
    await form.getByRole('button',{name:'清除 Cookie',exact:true}).click();
    await authStatus().getByText('当前使用：匿名访问').waitFor();
    check((await snapshot()).cookies[platform]==='','clear did not remove manual Cookie');
    if(platform!=='youtube'){
      await form.getByRole('tab',{name:'内置登录',exact:true}).click();
      await form.getByRole('button',{name:'切换到内置登录'}).click();
      await authStatus().getByText('当前使用：内置登录').waitFor();
      await form.getByRole('button',{name:'清除会话'}).click();
      await authStatus().getByText('当前使用：匿名访问').waitFor();
    }
    cases.push(platform+': auth storage/trim/reload/validation/clear');
  }
  await fresh();await page.getByRole('radio',{name:'自定义代理',exact:true}).click();
  await proxy().fill('7890');await save().click();
  await notice('error','配置格式有误');
  check((await snapshot()).cookies.youtube==='session=fixture','invalid proxy cleared Cookie');
  await proxy().fill('http://127.0.0.1:7890');await save().click();await saved();
  cases.push('invalid proxy save preserves Cookie; corrected address saves');
  await fresh();
  await page.reload();
  await page.evaluate(()=>window.platformSettingsTest.failCookie=true);
  await accounts(false);
  await page.getByRole('alert').getByText('无法保存任务记录').waitFor();
  await page.getByRole('textbox',{name:'User-Agent',exact:true}).fill('updated-fixture-agent');
  await save().click();await saved();
  const last=await page.evaluate(()=>window.platformSettingsTest.calls.at(-1));
  check(last.cookie===null && (await snapshot()).cookies.youtube==='session=fixture','Cookie read failure overwrote credentials');
  cases.push('Cookie read failure preserves stored credentials while saving other settings');
  await page.getByRole('button',{name:'下载作品',exact:true}).click();
  await page.getByRole('textbox').first().fill('https://www.youtube.com/watch?v=a6cnYFcd7JQ');
  await page.getByRole('button',{name:'解析链接',exact:true}).click();
  await notice('error','切换 TUN/代理出口');
  cases.push('risk-control error gives verification and TUN exit guidance');
  return {passed:cases.length,cases};
}
