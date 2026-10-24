// A spinning loading icon must not intermittently create a scrollbar in the compact dialog.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const cases=[];
  const check=(value,message)=>{if(!value)throw Error(message)};
  async function sample(phase,width) {
    const frames=await page.getByRole('dialog',{name:'视频文案'}).evaluate(async dialog=>{
      const body=dialog.querySelector('[data-slot="dialog-header"]').nextElementSibling;
      const spinner=body.querySelector('[data-slot="spinner"]');
      const frames=[],start=performance.now();
      while(performance.now()-start<2100) {
        await new Promise(requestAnimationFrame);
        frames.push({height:body.clientHeight,scrollHeight:body.scrollHeight,width:body.clientWidth,
          scrollWidth:body.scrollWidth,transform:getComputedStyle(spinner).transform});
      }
      return frames;
    });
    check(frames.length>=10,'insufficient animation frames');
    check(new Set(frames.map(f=>f.transform)).size>5,'spinner animation was not exercised');
    check(frames.every(f=>f.height===f.scrollHeight && f.width===f.scrollWidth),phase+' created scrollbar at '+width);
    check(new Set(frames.map(f=>f.height)).size===1,phase+' changed body height at '+width);
    cases.push(phase+' '+width+': '+frames.length+' animated frames without overflow');
  }
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=transcript-loading');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'提取文案',exact:true}).click();
    await page.getByText('正在读取文案…',{exact:true}).waitFor();
    await sample('reading',width);
    await page.evaluate(()=>window.backgroundTest.finishTranscriptRead());
    const dialog=page.getByRole('dialog',{name:'视频文案'});
    const start=dialog.getByRole('button',{name:'开始提取',exact:true});
    await start.click();
    await page.getByText(/正在加载模型并识别/).waitFor();
    await sample('recognizing',width);
    await page.screenshot({path:'.local/transcript-scrollbar-flicker/loading-'+width+'.png'});
    const cancel=dialog.getByRole('button',{name:'取消提取',exact:true});
    await cancel.focus(); await page.keyboard.press('Enter');
    await dialog.getByText('本次文案提取已取消',{exact:true}).waitFor();
    check((await page.evaluate(()=>window.backgroundTest.calls)).some(c=>c.type==='cancel-transcription'),'keyboard cancellation failed');
  }
  return {passed:cases.length,cases};
}
