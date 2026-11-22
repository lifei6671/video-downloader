// Mock authentication only. No third-party login or real credential is used.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.goto(base+'/tests/fixtures/platform-settings.html');
  await page.addScriptTag({url:base+'/src-tauri/src/browser-identity.js'});
  await page.waitForFunction(()=>window.__yingquBrowserIdentity);
  const observed=await page.evaluate(()=>({observed:window.__yingquBrowserIdentity,ua:navigator.userAgent,platform:navigator.platform,width:screen.width}));
  check(observed.observed.user_agent===observed.ua,'native browser UA was changed');
  check(observed.observed.environment.platform===observed.platform,'native platform mismatch');
  check(observed.observed.environment.screen_width===observed.width,'native screen mismatch');
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());await page.reload();
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    for(const [platform,name,button] of [['douyin','抖音','登录抖音'],['xiaohongshu','小红书','登录小红书'],
      ['tiktok','TikTok','登录 TikTok'],['bilibili','B 站','登录B 站'],['weixin','微信视频号','登录腾讯元宝']]) {
      await page.getByRole('tab',{name,exact:true}).click();
      const panel=page.getByRole('tabpanel',{name,exact:true}).locator('form');
      await panel.getByRole('tab',{name:platform==='weixin'?'元宝登录':'内置登录',exact:true}).click();
      await panel.getByRole('button',{name:button,exact:true}).click();
      await panel.getByText('已绑定登录浏览器',{exact:true}).waitFor();
      const input=panel.locator('#'+platform+'-ua');
      await input.fill('Manual source/'+platform);
      await page.getByRole('button',{name:'保存平台配置',exact:true}).click();
      const state=await page.evaluate(()=>window.platformSettingsTest.snapshot());
      const profile=state.settings.profiles.find(p=>p.platform===platform);
      check(profile.login_device.user_agent==='Native browser/'+platform,'manual UA replaced native identity');
      check(profile.user_agent==='Manual source/'+platform,'manual source UA not saved');
      check(profile.account_auth==='login','editing manual UA switched authentication');
      await panel.getByRole('button',{name:'清除会话',exact:true}).click();
      check(!await page.evaluate(p=>window.platformSettingsTest.snapshot().settings.profiles.find(v=>v.platform===p).login_device,platform),'clear retained device binding');
    }
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    const yt=page.getByRole('tabpanel',{name:'YouTube',exact:true}).locator('form');
    await yt.getByRole('button',{name:'打开 YouTube 登录',exact:true}).click();
    check(await page.evaluate(()=>window.platformSettingsTest.youtubeOpens===1),'external browser login changed');
    check(!await page.evaluate(()=>window.platformSettingsTest.snapshot().settings.profiles.find(p=>p.platform==='youtube').login_device),'external browser identity was guessed');
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'layout overflow');
    results.push({width,height,nativePlatforms:5,externalYouTube:true});
  }
  return results;
}
