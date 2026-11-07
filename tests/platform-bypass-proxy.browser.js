// Isolated IPC and synthetic credentials; no native credentials or platform requests.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const cases=[];
  const platforms=[['douyin','抖音'],['tiktok','TikTok'],['youtube','YouTube'],['telegram','Telegram']];
  const bypass=()=>page.getByRole('checkbox',{name:'解析与下载不走代理',exact:true});
  const snapshot=()=>page.evaluate(()=>window.platformSettingsTest.snapshot());
  const open=async name=>{
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    await page.getByRole('tab',{name,exact:true}).click();
  };
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());await page.reload();
  };
  const notice=async(type,text)=>{
    const item=page.locator(`[data-sonner-toast][data-type="${type}"]`).filter({hasText:text});
    await item.waitFor();await item.getByRole('button',{name:'关闭通知'}).click();
    await item.waitFor({state:'detached'});
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await fresh();
    await open('抖音');
    for(const [platform,name] of platforms) {
      await page.getByRole('tab',{name,exact:true}).click();
      const form=page.locator('#profile-'+platform);
      const save=page.getByRole('button',{name:platform==='telegram'?'保存代理':'保存平台配置',exact:true});
      const address=form.getByRole('textbox',{name:platform==='telegram'?'SOCKS5 地址':'HTTP / SOCKS5 地址',exact:true});
      const success=platform==='telegram'?'代理配置已保存':'平台配置已保存，仅影响新任务';
      check(!await bypass().isChecked(),platform+' new setting should be off');
      await form.getByRole('radio',{name:'自定义代理',exact:true}).click();
      const url='socks5://fixture-user:fixture-pass@127.0.0.1:7890';
      await address.fill(url);await save.click();await notice('success',success);
      await bypass().focus();await page.keyboard.press('Space');
      check(await address.isDisabled(),platform+' proxy editor still active');
      check(await form.getByRole('radio',{name:'直连',exact:true}).isDisabled(),platform+' mode still active');
      await save.click();await notice('success',success);
      check(await save.isDisabled(),platform+' remains dirty after save');
      const state=await snapshot();
      check(state.settings.profiles.find(p=>p.platform===platform).bypass_proxy===true,platform+' flag was not saved');
      check(state.proxies[platform]===url,platform+' cleared saved authenticated proxy');
      check(await page.evaluate(()=>window.platformSettingsTest.calls.at(-1).proxy===null),'bypass save should preserve proxy');
      const other=platform==='douyin'?'TikTok':'抖音';
      await page.getByRole('tab',{name:other,exact:true}).click();
      check(!await bypass().isChecked(),'platform bypass setting leaked');
      await page.getByRole('tab',{name,exact:true}).click();
      check(await bypass().isChecked(),platform+' tab switch lost bypass');
      await page.reload();await open(name);
      check(await bypass().isChecked(),platform+' reload lost bypass');
      check(await address.inputValue()===url,platform+' reload lost proxy address');
      if(width===1440)await page.screenshot({path:'.local/bypass-proxy/'+platform+'.png',animations:'disabled'});
      await bypass().click();check(await address.isEnabled(),platform+' original proxy not editable again');
      await save.click();await notice('success',success);
      check((await snapshot()).proxies[platform]===url,platform+' disabling bypass lost credentials');
      check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
      cases.push(width+': '+name+' persists, isolates, restores proxy and supports keyboard');
    }
  }
  await fresh();await open('YouTube');
  await bypass().click();await page.evaluate(()=>window.platformSettingsTest.failSave=true);
  await page.getByRole('button',{name:'保存平台配置',exact:true}).click();
  await notice('error','无法保存任务记录，请检查磁盘空间并重启应用');
  check(!(await snapshot()).settings.profiles.find(p=>p.platform==='youtube').bypass_proxy,'failed save changed stored flag');
  check(await bypass().isChecked(),'failed save lost draft');
  cases.push('failed save preserves stored flag and draft');
  for(const [platform,name] of [platforms[2],platforms[3]]) {
    await fresh();
    await page.evaluate(()=>window.platformSettingsTest.failProxy=true);
    await open(name);
    await page.getByRole('alert').filter({hasText:'系统凭据'}).waitFor();
    await bypass().click();
    await page.getByRole('button',{name:platform==='telegram'?'保存代理':'保存平台配置',exact:true}).click();
    await notice('success',platform==='telegram'?'代理配置已保存':'平台配置已保存，仅影响新任务');
    check((await snapshot()).settings.profiles.find(p=>p.platform===platform).bypass_proxy,'cannot bypass unreadable proxy');
    check(await page.evaluate(()=>window.platformSettingsTest.calls.at(-1).proxy===null),'unreadable proxy overwritten');
    if(platform==='telegram') {
      check(await page.getByRole('alert').filter({hasText:'系统凭据'}).isVisible(),'bypass save hid proxy read failure');
      const retry=page.getByRole('button',{name:'重新读取代理',exact:true});
      check(await retry.isVisible(),'bypass save removed proxy read retry');
      await bypass().click();
      check(await page.getByRole('radio',{name:'直连',exact:true}).isDisabled(),'unreadable proxy editor became active');
      await page.evaluate(()=>window.platformSettingsTest.failProxy=false);
      await retry.click();
      await page.waitForFunction(()=>!document.querySelector('#telegram-proxy-mode + [data-slot="toggle-group"] button')?.disabled);
      check(await page.getByRole('radio',{name:'直连',exact:true}).isEnabled(),'proxy reread did not restore editor');
      await page.getByRole('button',{name:'保存代理',exact:true}).click();await notice('success','代理配置已保存');
    }
    cases.push(name+': bypass works while proxy credential read fails');
  }
  return {passed:cases.length,cases};
}
