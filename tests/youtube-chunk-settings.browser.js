// Isolated platform-settings IPC fixture; no real credentials or platform requests.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const mode=name=>page.getByRole('radio',{name,exact:true});
  const size=()=>page.getByRole('spinbutton',{name:'自定义分块大小（MiB）',exact:true});
  const save=()=>page.getByRole('button',{name:'保存平台配置',exact:true});
  const snapshot=()=>page.evaluate(()=>window.platformSettingsTest.snapshot());
  const stored=async()=>(await snapshot()).settings.profiles.find(p=>p.platform==='youtube').http_chunk_size;
  const accounts=async(loaded=true)=>{
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    if(loaded)await page.waitForFunction(()=>!document.querySelector('#youtube-clear')?.disabled);
  };
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/platform-settings.html');
    await page.evaluate(()=>sessionStorage.clear());
    await page.reload();await accounts();
  };
  const saved=()=>page.getByRole('status').filter({hasText:'平台配置已保存，仅影响新任务'}).waitFor();
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await fresh();
    check(await mode('继承上游默认').isChecked(),'existing settings must inherit');
    check(await save().isDisabled() && await size().count()===0,'inherited mode must start clean');
    await mode('自定义').focus();await page.keyboard.press('Space');
    check(await size().inputValue()==='10','initial custom size must be 10 MiB');
    for(const invalid of ['', '0', '-1', '1.5', '1025']) {
      await size().fill(invalid);
      check(await size().getAttribute('aria-invalid')==='true' && await save().isDisabled(),'invalid size allowed: '+invalid);
      await page.getByText('请输入 1–1024 之间的整数。',{exact:true}).waitFor();
    }
    await size().fill('16');await save().click();await saved();
    check(await stored()===16*1048576 && await save().isDisabled(),'custom MiB was not persisted as bytes');
    check((await snapshot()).cookies.youtube==='session=fixture','chunk setting changed Cookie');
    await page.reload();await accounts();
    check(await mode('自定义').isChecked() && await size().inputValue()==='16','custom setting lost on reload');
    await size().fill('24');
    await page.getByRole('tab',{name:'TikTok',exact:true}).click();
    check(await mode('自定义').count()===0,'chunk setting exposed outside YouTube');
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    check(await size().inputValue()==='24' && !await save().isDisabled(),'tab switch lost unsaved draft');
    await page.screenshot({path:'.local/youtube-chunk-settings-20261021/settings-'+width+'.png',animations:'disabled'});
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow at '+width);
    await mode('关闭分块').click();await save().click();await saved();
    check(await stored()===0 && await size().count()===0,'disabled mode must save explicit zero');
    await page.reload();await accounts();
    check(await mode('关闭分块').isChecked(),'disabled mode lost on reload');
    await mode('继承上游默认').click();await save().click();await saved();
    check(await stored()===null,'inherit must clear explicit override');
    await page.reload();await accounts();
    check(await mode('继承上游默认').isChecked(),'inherit lost on reload');
    cases.push(width+': three modes/keyboard/validation/MiB/save/reload/tab draft/isolation/layout');
  }
  await fresh();await mode('自定义').click();await size().fill('32');
  await page.evaluate(()=>window.platformSettingsTest.failSave=true);
  await save().click();await page.getByRole('alert').filter({hasText:'无法保存任务记录'}).waitFor();
  check(await stored()===null && await size().inputValue()==='32' && !await save().isDisabled(),'failed save changed mode or lost draft');
  await page.evaluate(()=>window.platformSettingsTest.failSave=false);
  await save().click();await saved();
  check(await stored()===32*1048576,'failed draft cannot be retried');
  cases.push('save failure preserves persisted mode and retryable draft');
  await mode('关闭分块').click();
  await page.evaluate(()=>window.platformSettingsTest.holdSave=true);
  await save().click();
  await page.getByRole('button',{name:'保存中…',exact:true}).waitFor();
  check(await mode('自定义').isDisabled(),'pending save permits mode changes');
  await page.evaluate(()=>{window.platformSettingsTest.holdSave=false;window.platformSettingsTest.release()});
  await saved();check(await stored()===0,'pending save changed submitted value');
  cases.push('pending save freezes mode and size');
  await fresh();await page.evaluate(()=>window.platformSettingsTest.failCookie=true);
  await page.getByRole('button',{name:'下载任务',exact:true}).click();await accounts(false);
  await page.getByRole('alert').filter({hasText:'Cookie'}).waitFor();
  await mode('自定义').click();await size().fill('1024');await save().click();await saved();
  check(await stored()===1073741824 && (await snapshot()).cookies.youtube==='session=fixture','Cookie read failure prevents safe chunk save');
  check((await snapshot()).settings.profiles.filter(p=>p.platform!=='youtube').every(p=>p.http_chunk_size===null),'platform isolation failed');
  cases.push('Cookie read failure preserves credentials while saving maximum custom size');
  return {passed:cases.length,cases};
}
