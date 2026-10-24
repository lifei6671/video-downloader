// Real App with isolated IPC: Nano is current; historical CPU/CUDA results retain their identity.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const dialog = page.getByRole('dialog', {name:'视频文案'});
  const cases = [];
  for (const [scenario, label] of [['transcript-legacy-cuda','SenseVoiceSmall CUDA 加速识别'], ['transcript-legacy-cpu','SenseVoiceSmall CPU 本地识别'], ['transcript','Fun-ASR-Nano CPU 本地识别'], ['transcript-legacy-cuda-failed','SenseVoiceSmall CUDA 加速识别'], ['transcript-legacy-cuda-unavailable','SenseVoiceSmall CUDA 加速识别']]) {
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=' + scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:scenario==='transcript'?'提取文案':'查看文案',exact:true}).click();
    if (scenario !== 'transcript') {
      await dialog.getByText(new RegExp(label)).waitFor();
      if (scenario.endsWith('failed') || scenario.endsWith('unavailable')) {
        await dialog.getByRole('alert').filter({hasText:'运行依赖'}).waitFor();
      }
      await dialog.getByRole('button',{name:'复制全文',exact:true}).click();
      await dialog.getByRole('button',{name:'导出 TXT',exact:true}).click();
      await dialog.getByRole('button',{name:'重新提取',exact:true}).click();
    } else {
      await dialog.getByRole('button',{name:'开始提取',exact:true}).click();
    }
    await dialog.getByText(/Fun-ASR-Nano CPU 本地识别/).waitFor();
    await page.evaluate(()=>window.backgroundTest.finishTranscription());
    await dialog.getByLabel('识别文案',{exact:true}).waitFor();
    await dialog.getByText(/Fun-ASR-Nano CPU 本地识别/).waitFor();
    await page.keyboard.press('Escape');
    await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'查看文案',exact:true}).click();
    await dialog.getByText(/Fun-ASR-Nano CPU 本地识别/).waitFor();
    cases.push(scenario + ': correct model identity, re-extraction and reopen');
  }
  return {passed:cases.length,cases};
}
