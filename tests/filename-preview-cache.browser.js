// Isolated IPC fixture; verifies navigation reuse without caching invalid drafts.
async page => {
  page.setDefaultTimeout(8000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const input=()=>page.getByRole('textbox',{name:'文件命名规则',exact:true});
  const preview=()=>page.getByRole('status',{name:'实时预览',exact:true});
  const text=async expected=>page.waitForFunction(expected=>document.querySelector('[aria-labelledby="filename-preview-label"]')?.textContent===expected,expected);
  const calls=()=>page.evaluate(()=>window.filenameSettingsTest.previewCalls.length);
  const settings=()=>page.getByRole('button',{name:'系统设置',exact:true}).click();
  const fresh=async()=>{
    await page.goto(base+'/tests/fixtures/filename-settings.html');
    await page.evaluate(()=>sessionStorage.clear());await page.reload();await settings();
    await text('youtube/creator-sample-video.mp4');
  };
  const leave=()=>page.getByRole('button',{name:'下载任务',exact:true}).click();
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});await fresh();
    const count=await calls();
    await page.evaluate(()=>{
      window.navigationPreviewFlashes=[];
      window.navigationPreviewObserver=new MutationObserver(()=>{
        const node=document.querySelector('[aria-labelledby="filename-preview-label"]');
        if(node && (!node.textContent || node.textContent.includes('正在预览') || node.getAttribute('aria-busy')==='true'))window.navigationPreviewFlashes.push(node.textContent);
      });
      window.navigationPreviewObserver.observe(document.body,{childList:true,subtree:true,attributes:true,attributeFilter:['aria-busy'],characterData:true});
    });
    for(let i=0;i<3;i++){
      await leave();await settings();
      check(await preview().textContent()==='youtube/creator-sample-video.mp4' && await preview().getAttribute('aria-busy')==='false','navigation did not immediately restore preview');
    }
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    await page.getByRole('button',{name:'下载设置',exact:true}).click();
    await page.waitForTimeout(220);
    check(await calls()===count,'page switches repeated preview IPC');
    check(await page.evaluate(()=>window.navigationPreviewFlashes.length===0),'navigation preview flashed loading or busy');
    await page.evaluate(()=>window.navigationPreviewObserver.disconnect());
    // A draft and its late result cannot evict the saved preview.
    await page.evaluate(()=>window.filenameSettingsTest.holdPreview=true);
    await input().fill('{link_id}');
    await page.waitForFunction(()=>window.filenameSettingsTest.pendingPreviews.length===1);
    const pendingCount=await calls();
    await leave();await settings();await text('youtube/creator-sample-video.mp4');
    check(await input().inputValue()==='{platform}/{user_name}-{content}' && await calls()===pendingCount,'draft evicted saved cache or was incorrectly persisted');
    await page.evaluate(()=>{window.filenameSettingsTest.releasePreview(0);window.filenameSettingsTest.holdPreview=false});
    await page.waitForTimeout(30);
    check(await preview().textContent()==='youtube/creator-sample-video.mp4','late unmounted draft replaced saved preview');
    // Save replaces the single cached rule; returning renders it without IPC.
    await input().fill('{link_id}');await text('sample-id.mp4');
    await page.getByRole('button',{name:'保存命名规则',exact:true}).click();
    await page.waitForFunction(()=>window.filenameSettingsTest.snapshot().filename_template==='{link_id}');
    const savedCount=await calls();await leave();await settings();await text('sample-id.mp4');
    await page.waitForTimeout(200);
    check(await calls()===savedCount && await page.getByRole('button',{name:'保存命名规则',exact:true}).isDisabled(),'saved rule cache is stale or marked dirty');
    await input().fill('../invalid');
    await page.waitForFunction(()=>document.querySelector('#filename-template')?.getAttribute('aria-invalid')==='true');
    const invalidCount=await calls();await leave();await settings();await text('sample-id.mp4');
    check(await calls()===invalidCount && await input().getAttribute('aria-invalid')==='false','invalid draft contaminated cached success');
    results.push(`${width}: immediate reuse/no IPC or placeholder, draft/late-result isolation, save refresh, invalid draft isolation`);
  }
  // No cache survives an application reload; current saved settings are rechecked.
  await page.reload();await settings();await text('sample-id.mp4');
  check(await calls()===1,'fresh app reused old cache or repeated initial request');
  results.push('new app session computes one fresh preview for persisted rule');
  return {passed:results.length,results};
}
