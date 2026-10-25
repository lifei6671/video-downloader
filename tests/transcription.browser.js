// Real App with isolated IPC; no user video, clipboard, or export file is changed.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const dialog=page.getByRole('dialog',{name:'视频文案'});
  const results=[];
  const check=(value,message)=>{if(!value)throw Error(message)};
  const calls=()=>page.evaluate(()=>window.backgroundTest.calls);
  const load=async scenario=>{
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.getByText('单个视频文案',{exact:true}).waitFor();
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load('transcript');
    await page.locator('[data-task-id=album]').click({button:'right',position:{x:200,y:30}});
    check(await page.getByRole('menuitem',{name:'提取文案',exact:true}).count()===0,'album exposes transcription');
    await page.keyboard.press('Escape');
    await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'提取文案',exact:true}).click();
    const start=dialog.getByRole('button',{name:'开始提取',exact:true});await start.waitFor();
    await page.waitForFunction(()=>document.querySelector('[role="dialog"] button:not([disabled])'));
    await start.click();await page.getByText(/正在加载语音模型/).waitFor();
    check(await dialog.getByRole('progressbar',{name:'文案识别进度'}).count()===0,'model loading shows fabricated progress');
    const reads=(await calls()).filter(v=>v.type==='read-transcript').length;
    for(const completed of [0,1,2]) {
      await page.evaluate(completed=>window.backgroundTest.progressTranscription(completed,3),completed);
      await dialog.getByText(new RegExp('已识别 '+completed+' / 3 段')).waitFor();
      check(await dialog.getByRole('progressbar',{name:'文案识别进度'}).getAttribute('aria-valuenow')===String(Math.floor(completed/3*100)),'wrong segment percentage');
    }
    check((await calls()).filter(v=>v.type==='read-transcript').length===reads,'progress rereads or clears transcript text');
    await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-context-target'));
    check(await page.locator('[data-task-id=speech]').locator('..').evaluate(n=>n===document.activeElement),'transcript did not restore menu trigger focus');
    await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'正在提取文案',exact:true}).click();
    check((await calls()).filter(v=>v.type==='transcribe').length===1,'reopening repeats transcription');
    await page.evaluate(()=>window.backgroundTest.finishTranscription());
    await dialog.getByLabel('识别文案',{exact:true}).waitFor();
    const body=dialog.getByRole('textbox',{name:'识别文案',exact:true});
    await body.focus();await page.keyboard.press('Control+A');
    check(await body.evaluate(el=>el.selectionStart===0 && el.selectionEnd===el.value.length),'Ctrl+A does not select the entire transcript');
    const originalText=await body.inputValue();
    await body.evaluate(el=>el.addEventListener('copy',event=>{window.transcriptSelection=el.value.slice(el.selectionStart,el.selectionEnd);event.preventDefault()},{once:true}));
    await page.keyboard.press('Control+C');
    check(await page.evaluate(()=>window.transcriptSelection)===originalText,'Ctrl+C does not target the selected transcript');
    await page.keyboard.type('must not replace read-only text');
    check(await body.inputValue()===originalText,'transcript became editable');
    await page.keyboard.press('Tab');
    check(await dialog.getByRole('button',{name:'复制全文',exact:true}).evaluate(el=>el===document.activeElement),'Tab does not reach copy action');
    await dialog.getByRole('button',{name:'复制全文',exact:true}).click();await page.getByText('文案已复制',{exact:true}).waitFor();
    await dialog.getByRole('button',{name:'导出 TXT',exact:true}).click();await page.getByText(/已导出：/).waitFor();
    const output=(await calls()).filter(v=>['copy-transcript','export-transcript'].includes(v.type));
    check(output.length===2 && output[0].text===output[1].text,'copy/export text differs');
    await page.evaluate(()=>window.backgroundTest.staleTranscription());
    check(await dialog.getByLabel('识别文案',{exact:true}).isVisible(),'stale update overwrites result');
    const box=await dialog.boundingBox();check(box.x>=0 && box.y>=0 && box.x+box.width<=width && box.y+box.height<=height,'dialog exceeds viewport');
    await page.screenshot({path:'.local/transcription-delivery/dialog-'+width+'.png'});
    await dialog.getByRole('button',{name:'重新提取',exact:true}).click();
    await dialog.getByRole('button',{name:'取消提取',exact:true}).click();
    await page.getByText('本次文案提取已取消',{exact:true}).waitFor();
    check(await dialog.getByLabel('识别文案',{exact:true}).isVisible(),'cancel erased successful text');
    results.push(width+'x'+height+' lifecycle, copy/export, stale events, cancel');
  }
  await load('transcript-model');await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'提取文案',exact:true}).click();
  await dialog.getByRole('button',{name:'下载或修复语音模型',exact:true}).click();
  await dialog.getByRole('progressbar',{name:'依赖下载进度'}).waitFor();
  await page.evaluate(()=>window.backgroundTest.finishInstall());
  await dialog.getByRole('button',{name:'开始提取',exact:true}).waitFor();
  check((await calls()).filter(v=>v.type==='transcribe').length===0,'model install starts transcription automatically');
  results.push('on-demand model download requires explicit start');
  await load('transcript-missing-file');await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'查看文案',exact:true}).click();
  await dialog.getByLabel('识别文案',{exact:true}).waitFor();
  check(await dialog.getByRole('button',{name:'重新提取',exact:true}).isDisabled(),'missing video can be transcribed');
  check(await dialog.getByRole('button',{name:'复制全文',exact:true}).isEnabled(),'saved transcript inaccessible after video loss');
  results.push('missing source preserves saved transcript');
  return {passed:results.length,cases:results};
}
