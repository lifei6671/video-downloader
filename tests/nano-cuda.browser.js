async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const cases = [];
  const card = () => page.locator('[data-slot="card"]').filter({has:page.locator('[data-slot="card-title"]').getByText('语音转录模型 Fun-ASR-Nano',{exact:true})});
  async function settings(scenario) {
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('button',{name:'运行依赖',exact:true}).click();
    await card().getByText('模型大小：约 1.2 GiB',{exact:true}).waitFor();
  }
  async function choose(value) {
    await card().getByRole('combobox',{name:'语音模型版本'}).click();
    await page.getByRole('option',{name:value+' 版',exact:true}).click();
  }
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await settings('runtime-missing');
    await choose('CUDA');
    await card().getByText('模型大小：约 1.0 GiB',{exact:true}).waitFor();
    await card().getByRole('button',{name:'下载语音模型',exact:true}).click();
    await card().getByRole('progressbar').waitFor();
    const calls = await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='install-runtime'));
    if(calls.length!==1||calls[0].tool!=='fun-asr-nano'||calls[0].variant!=='cuda')throw Error('CUDA must be a version of the model dependency');
    if(!await card().getByRole('combobox').isDisabled())throw Error('version changed during installation');
    await page.evaluate(()=>window.backgroundTest.finishRuntimeInstall());
    await card().getByText('CUDA 加速识别',{exact:true}).waitFor();
    await choose('CPU');
    await card().getByRole('button',{name:'安装并切换版本',exact:true}).click();
    await card().getByRole('progressbar').waitFor();
    await card().getByRole('button',{name:'取消下载',exact:true}).click();
    await page.getByRole('alert').filter({hasText:'操作已取消'}).waitFor();
    await card().getByText('CUDA 加速识别',{exact:true}).waitFor();
    await card().scrollIntoViewIfNeeded();
    await page.screenshot({path:'.local/nano-cuda-delivery/browser/cuda-card-'+width+'.png'});
    cases.push(width+': CUDA selected and installed; canceled CPU switch preserves current CUDA');
  }
  await settings('runtime-cuda-incompatible');await choose('CUDA');
  await card().getByRole('button',{name:'下载语音模型',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'当前显卡不支持'}).waitFor();
  await choose('CPU');
  if(!await card().getByRole('button',{name:'下载语音模型',exact:true}).isEnabled())throw Error('incompatible GPU blocks CPU');
  cases.push('CUDA incompatibility stays visible and CPU remains available');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=transcript-cuda');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByRole('button',{name:'提取文案',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'视频文案'});
  await dialog.getByRole('button',{name:'开始提取',exact:true}).click();
  await dialog.getByText(/正在检测语音片段/).waitFor();
  await dialog.getByText(/Fun-ASR-Nano CUDA 加速识别/).waitFor();
  await page.evaluate(()=>window.backgroundTest.finishTranscription());
  await dialog.getByLabel('识别文案',{exact:true}).waitFor();
  await dialog.getByRole('button',{name:'复制全文',exact:true}).click();
  await dialog.getByRole('button',{name:'导出 TXT',exact:true}).click();
  await page.keyboard.press('Escape');
  await page.getByRole('button',{name:'查看文案',exact:true}).click();
  await dialog.getByText(/Fun-ASR-Nano CUDA 加速识别/).waitFor();
  cases.push('independent VAD progress, saved CUDA identity, copy/export/reopen');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=transcript-model');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByRole('button',{name:'提取文案',exact:true}).click();
  await dialog.getByRole('combobox',{name:'语音模型版本'}).click();
  await page.getByRole('option',{name:'CUDA 版',exact:true}).click();
  await dialog.getByRole('button',{name:'下载或修复语音模型',exact:true}).click();
  const install=await page.evaluate(()=>window.backgroundTest.calls.find(c=>c.type==='install'));
  if(install?.variant!=='cuda')throw Error('first-use dialog lost CUDA choice');
  await page.evaluate(()=>window.backgroundTest.finishInstall());
  cases.push('first-use model installation shares CUDA choice');
  return {passed:cases.length,cases};
}
