// Actual tiny WebM files, synthetic native IPC. No user media is opened.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const results=[];
  page.setDefaultTimeout(8000);
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const dialog=()=>page.locator('.video-player-dialog');
  const measure=()=>dialog().evaluate(node=>{
    const box=element=>{const r=element.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom,right:r.right}};
    const v=node.querySelector('video');
    return {dialog:box(node),stage:box(node.querySelector('.video-player-stage')),video:box(v),details:box(node.querySelector('.task-details-content')),width:v.videoWidth,height:v.videoHeight,fit:getComputedStyle(v).objectFit,controls:v.controls,scroll:node.scrollHeight-node.clientHeight,viewport:{width:innerWidth,height:innerHeight}};
  });
  const assertBounds=value=>{
    const d=value.dialog,s=value.stage;
    check(d.x>=0&&d.y>=0&&d.right<=value.viewport.width+1&&d.bottom<=value.viewport.height+1,'dialog exceeds viewport');
    check(value.scroll<=1,'title or player controls require scrolling: '+JSON.stringify(value));
    check(value.fit==='scale-down'&&value.controls,'video can be enlarged beyond native size or lost its controls');
    check(value.details.x>=s.right-1 && value.video.width<=s.width+1 && value.video.height<=s.height+1,'video or details leave their panes');
  };
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.evaluate(async()=>{
      localStorage.removeItem('yingqu.task-view');
      const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
      api.playbackSource=async id=>({
        'visual-3':'/tests/fixtures/media/player-portrait.webm',
        'visual-9':'/tests/fixtures/media/player.webm',
        'visual-10':'/tests/fixtures/media/player-square.webm',
      })[id];
    });
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    const sizes={};
    for(const [id,label,ratio] of [[3,'portrait',9/16],[9,'landscape',16/9],[10,'square',1]]) {
      await page.locator(`[data-task-id="visual-${id}"] .task-play`).click();
      await page.waitForFunction(()=>document.querySelector('.video-player-dialog.has-media-size video')?.readyState>=2);
      await dialog().evaluate(async node=>{await Promise.all(node.getAnimations().map(animation=>animation.finished.catch(()=>{})))});
      const size=await measure();assertBounds(size);
      check(Math.abs(size.width/size.height-ratio)<0.01,'wrong fixture dimensions');
      sizes[label]={width:size.stage.width,height:size.stage.height};
      check(size.stage.width>=size.width && size.stage.height>=size.height,'native-size video does not fit');
      check(size.stage.width<=280 && size.stage.height<=160 && size.dialog.height<=302,'tiny video occupies an oversized stage');
      if(label==='portrait') {
        await dialog().locator('video').evaluate(video=>{ window.sizedVideo=video; });
        await page.setViewportSize({width:2560,height:1600});
        const larger=await measure();
        assertBounds(larger);
        check(larger.stage.width===size.stage.width&&larger.stage.height===size.stage.height&&larger.dialog.height===size.dialog.height,'tiny video grew with the viewport');
        check(larger.details.width<=320&&larger.dialog.width<=602,'details made the tiny-video dialog too wide');
        check(await dialog().locator('video').evaluate(video=>video===window.sizedVideo&&video.readyState>=2),'resizing replaced or unloaded the video');
        await page.setViewportSize({width,height});
      }
      await page.keyboard.press('Escape');await dialog().waitFor({state:'detached'});
    }
    check(Math.abs(sizes.portrait.width-sizes.landscape.width)<1,'small videos do not share the minimum readable layout');
    results.push({width,height,sizes});
  }
  return {passed:results.length,results};
}
