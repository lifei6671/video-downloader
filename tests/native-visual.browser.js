// Only run against the separately built visual-validation instance with empty AppData.
// IPC is real. No real credentials, platform requests, installs or user file changes.
async page => {
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const nav=name=>page.getByRole('button',{name,exact:true});
  await nav('下载作品').waitFor();
  const native=await page.evaluate(async()=>({
    tauri:!!window.__TAURI_INTERNALS__,scale:devicePixelRatio,viewport:[innerWidth,innerHeight],
    tasks:await window.__TAURI_INTERNALS__.invoke('list_tasks'),
    settings:await window.__TAURI_INTERNALS__.invoke('get_settings')
  }));
  check(native.tauri && native.tasks.length===0,'not the isolated empty native instance');
  check(native.settings.profiles.length===3 && native.settings.profiles.every(p=>!p.cookie_configured),'native settings unexpectedly contain credentials');
  check(await page.getByText('本地运行',{exact:true}).isVisible(),'native IPC not connected');
  await page.screenshot({path:'output/playwright/visual-redesign/native-home.png'});
  await nav('下载任务').click();
  await page.getByText('还没有下载记录',{exact:true}).waitFor();
  await nav('系统设置').click();
  await page.getByRole('textbox',{name:'下载目录',exact:true}).waitFor();
  await nav('运行依赖').click();
  await nav('关于软件').click();
  await nav('检查更新').waitFor();
  await page.screenshot({path:'output/playwright/visual-redesign/native-about.png'});
  await nav('下载作品').click();
  return {nativeTauri:'PASS',realIPC:'PASS',pages:'4 PASS',scale:native.scale,viewport:native.viewport,emptyDatabase:true,platformRequests:0};
}
