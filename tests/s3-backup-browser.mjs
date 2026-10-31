// playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs
async (page) => {
  const checkTooltipBounds=async()=>{if(!await page.locator('[data-slot="tooltip-content"]').last().evaluate(el=>{const r=el.getBoundingClientRect();return r.left>=0&&r.top>=0&&r.right<=innerWidth&&r.bottom<=innerHeight;}))throw new Error('tooltip overflows viewport');if(!await page.locator('[data-slot="tooltip-content"]').last().evaluate(el=>{const s=getComputedStyle(el),arrow=el.querySelector('svg');return s.backgroundColor==='rgb(28, 32, 36)'&&s.color==='rgb(252, 252, 253)'&&(!arrow||getComputedStyle(arrow).fill===s.backgroundColor);}))throw new Error('tooltip colors are not unified');};
  await page.emulateMedia({reducedMotion:'reduce'});
  const errors=[];const onError=e=>errors.push(e.message);page.on('pageerror',onError);
  await page.addInitScript(()=>{
    localStorage.setItem('yingqu.task-view','list');
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
    window.backupCalls=[];window.backupOffline=false;window.coverFails=true;
    window.backupSettings={space_id:'space-1',library_id:'library-1',endpoint:'https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com',bucket:'test-bucket',prefix:'test/',provider_type:'s3',profile:'r2',region:'auto',path_style:true,protocol:2,last_error:null,enabled:true,credentials_configured:true};
    window.assets=Array.from({length:4},(_,i)=>({asset_id:`asset-${i}`,task_id:i<3?`local-${i}`:null,metadata:{title:i<3?`本机视频 ${i}`:'云端测试视频',author:'测试作者',platform:'youtube'},original_filename:'test.mp4',content_sha256:'a'.repeat(64),content_bytes:100,asset_state:'active',path:i<3?'C:/fixture/video.mp4':null,local_state:i<3?'present':'missing',space_id:'space-1',remote_state:i<2?'pending':'backed_up',has_cover:i===3,has_transcript:true,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0}));
    window.ops=[{operation_id:'op-0',space_id:'space-1',asset_id:'asset-0',kind:'upload',state:'blocked',phase:'queued',payload:{},attempt:5,last_error:'BACKUP_SERVICE_UNAVAILABLE'},{operation_id:'op-1',space_id:'space-1',asset_id:'asset-1',kind:'upload',state:'running',phase:'uploading_media',payload:{completed_bytes:50},attempt:0,last_error:null}];
    window.tasks=window.assets.slice(0,3).map((a,i)=>({id:a.task_id,asset_id:a.asset_id,revision:1,status:'completed',video:{...a.metadata,video_id:'12345678901',kind:'video',canonical_url:'https://www.youtube.com/watch?v=12345678901',resolver:'yt-dlp',duration_ms:1000,width:1920,height:1080},bytes:100,total_bytes:100,speed_bps:null,error:null,path:a.path,created_at:3-i,completed_at:4,file_missing:false,canceling:false,download_requested:true,cover_cached:false,files:[]}));
    window.tasks[2].video.title='长标题'.repeat(1500);
    window.tasks[0].video.author_profile={nickname:'测试作者'.repeat(500)};
    window.__TAURI_INTERNALS__={convertFileSrc:path=>`http://asset.localhost/${encodeURIComponent(path)}`,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args)=>{
      window.backupCalls.push(command);
      if(command==='list_tasks')return window.tasks;
      if(command==='list_video_transcripts')return [];
      if(command==='list_backup_operations')return window.ops;
      if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[],tdl:{enabled:false,path:null}};
      if(command==='list_backup_spaces')return [window.backupSettings];
      if(command==='select_backup_space')return window.backupSettings;
      if(command==='get_backup_sync_enabled')return true;
      if(command==='get_backup_settings')return window.backupSettings;
      if(command==='get_backup_credentials')return {access_key_id:'saved-key',secret_access_key:'saved-secret'};
      if(command==='test_backup_connection'){window.testDraft=args.config;return new Promise((resolve,reject)=>{window.finishTest=()=>resolve();window.failTest=()=>reject('BACKUP_NETWORK_ERROR');});}
      if(command==='save_backup_settings'||command==='rename_backup_space'){const {credentials,...config}=command==='save_backup_settings'?args.config:{display_name:args.displayName};const save=()=>{window.backupSettings={...window.backupSettings,...config};return window.backupSettings;};if(window.holdSave)return new Promise((resolve,reject)=>{window.finishSave=()=>resolve(save());window.failSave=()=>reject('BACKUP_NETWORK_ERROR');});return save();}
      if(command==='list_backup_assets')return window.assets;
      if(command==='refresh_backup_assets'){if(window.backupOffline)throw 'BACKUP_NETWORK_ERROR';return {imported:0,unchanged:4,rejected:[]};}
      if(command==='get_asset_cover'){if(window.coverFails)throw 'BACKUP_NETWORK_ERROR';return Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j7mQAAAAASUVORK5CYII='),c=>c.charCodeAt(0)).buffer;}
      if(command==='get_asset_transcript')return {asset_id:args.assetId,content_sha256:'a'.repeat(64),text:'无需下载视频即可读取的完整文案',model_revision:'test',model_name:'测试模型',pipeline_revision:'test',completed_at:1};
      if(command==='ensure_local_video'){window.assets.find(a=>a.asset_id===args.assetId).local_state='restoring';window.ops.unshift({operation_id:'restore',space_id:'space-1',asset_id:args.assetId,kind:'restore',state:'running',phase:'downloading',payload:{completed_bytes:25},attempt:0,last_error:null});return new Promise((resolve,reject)=>{window.cancelRestore=()=>{window.assets.find(a=>a.asset_id===args.assetId).local_state='missing';window.ops[0].state='canceled';reject('CANCELED');};});}
      if(command==='cancel_asset_restore'){window.cancelRestore?.();return;}
      if(command==='prepare_asset_playback')return new Promise(resolve=>{window.completeRestore=()=>resolve('C:/fixture/restored.mp4');});
      if(command==='delete_asset_and_backup'){const a=window.assets.find(a=>a.asset_id===args.assetId);a.remote_deleted=true;a.remote_state='pending_delete';window.ops.unshift({operation_id:'delete',space_id:'space-1',asset_id:args.assetId,kind:'delete',state:'retry_wait',phase:'tombstone',payload:{},attempt:1,last_error:'BACKUP_NETWORK_ERROR'});return;}
      if(command==='start_asset_backup'){window.lastUpload=args;const start=()=>{const op=window.ops.find(o=>o.asset_id===args.assetId);if(op){op.state='pending';op.last_error=null;op.attempt=0;}};if(window.holdUpload)return new Promise(resolve=>{window.finishUpload=()=>{start();resolve();};});start();return;}
      if(command==='cancel_backup_operation'){window.ops.find(o=>o.operation_id===args.operationId).state='canceled';return;}
      if(command==='retry_backup_operation'){const op=window.ops.find(o=>o.operation_id===args.operationId);op.state='pending';op.last_error=null;return;}
      if(command==='hide_asset_history'){window.assets=window.assets.filter(a=>a.asset_id!==args.assetId);return;}
      if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};
  });
  await page.reload();await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.getByText('云端测试视频',{exact:true}).waitFor();
  if(await page.getByRole('button',{name:'云端备份',exact:true}).count())throw new Error('unexpected cloud navigation');
  const pending=page.locator('[data-task-id="local-0"]'),uploading=page.locator('[data-task-id="local-1"]'),done=page.locator('[data-task-id="local-2"]');
  await pending.getByRole('button',{name:'上传备份',exact:true}).waitFor();
  await page.waitForFunction(()=>window.backupCalls.filter(c=>c==='list_backup_operations').length>=3);
  if(await page.locator('[data-sonner-toast]').filter({hasText:'远端服务暂时不可用'}).count())throw new Error('startup replayed historical failure toast');
  await pending.locator('.backup-sync-tooltip').hover();
  await page.getByRole('tooltip').filter({hasText:'远端服务暂时不可用'}).waitFor();
  await checkTooltipBounds();
  if(await pending.getByRole('button',{name:'上传备份',exact:true}).getAttribute('title')!==null)throw new Error('backup still uses native title');
  await page.keyboard.press('Escape');await page.mouse.move(0,0);
  if(!await pending.locator('svg.lucide-cloud').count())throw new Error('pending hollow cloud missing');
  if(await uploading.getByRole('progressbar').getAttribute('aria-valuenow')!=='50')throw new Error('upload byte progress missing');
  await done.getByRole('img',{name:'已备份',exact:true}).waitFor();
  if(!await done.locator('svg.lucide-circle-check circle').count())throw new Error('circled check missing');
  if(!(await uploading.locator('[data-slot="progress-indicator"]').getAttribute('style')).includes('180deg'))throw new Error('upload ring not half complete');
  await page.evaluate(()=>{window.holdUpload=true;});
  await pending.getByRole('button',{name:'上传备份',exact:true}).click();
  await pending.locator('[data-slot="spinner"]').waitFor();
  if(!await pending.getByRole('button').filter({has:page.locator('.backup-sync-glyph')}).isDisabled()||await pending.getByRole('progressbar').getAttribute('aria-valuenow')!==null)throw new Error('upload request has no immediate indeterminate feedback');
  await page.evaluate(()=>{window.holdUpload=false;window.finishUpload();});
  await pending.getByRole('button',{name:'取消上传',exact:true}).waitFor();
  if(!await pending.locator('[data-slot="spinner"]').count())throw new Error('queued upload flashed back to idle');
  await page.waitForFunction(()=>window.lastUpload?.assetId==='asset-0');
  if(await page.getByRole('menu').count())throw new Error('upload opened a menu');
  await page.evaluate(()=>{window.lastUpload=null;});
  await pending.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'从远端下载',exact:true}).getAttribute('aria-disabled')!=='true')throw new Error('local copy can be overwritten from menu');
  await page.getByRole('menuitem',{name:'上传云端',exact:true}).click();
  await page.waitForFunction(()=>window.lastUpload?.assetId==='asset-0');
  await pending.getByRole('button',{name:'取消上传',exact:true}).click();
  await pending.locator('svg.lucide-cloud').waitFor();
  await done.click({button:'right'});
  for(const name of ['上传云端','从远端下载'])if(await page.getByRole('menuitem',{name,exact:true}).getAttribute('aria-disabled')!=='true')throw new Error('completed local backup allows redundant transfer');
  await page.keyboard.press('Escape');
  await uploading.getByRole('button',{name:'取消上传',exact:true}).click();
  await uploading.getByRole('button',{name:'上传备份',exact:true}).waitFor();
  await uploading.getByRole('button',{name:'上传备份',exact:true}).click();
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').state='running';});
  await uploading.getByRole('button',{name:'取消上传',exact:true}).waitFor();
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').payload.completed_bytes=75;});
  await page.waitForFunction(()=>document.querySelector('[data-task-id="local-1"] [role="progressbar"]')?.getAttribute('aria-valuenow')==='75');
  if(!(await uploading.locator('[data-slot="progress-indicator"]').getAttribute('style')).includes('270deg'))throw new Error('upload ring does not advance with bytes');
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').payload.completed_bytes=100;});
  await page.waitForFunction(()=>document.querySelector('[data-task-id="local-1"] [role="progressbar"]')?.getAttribute('aria-valuenow')==='99');
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').state='completed';window.assets[1].remote_state='backed_up';});
  await uploading.getByRole('img',{name:'已备份',exact:true}).waitFor();
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').state='running';window.assets[1].remote_state='pending';});
  await page.evaluate(()=>{window.ops.find(o=>o.asset_id==='asset-1').payload.completed_bytes=50;});
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){await page.setViewportSize({width,height});if(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw new Error('list overflow');}
  const remote=page.locator('[data-asset-id="asset-3"]');
  for(const row of [pending,remote]) {
    const title=row.locator('.task-row-title');
    const text=await title.textContent();
    if(await title.getAttribute('title')!==null)throw new Error('video title still uses native title');
    await title.hover();await page.getByRole('tooltip').filter({hasText:text}).waitFor();
    await checkTooltipBounds();
    await page.keyboard.press('Escape');await page.mouse.move(0,0);
    await title.focus();await page.getByRole('tooltip').filter({hasText:text}).waitFor();
    await page.keyboard.press('Escape');await title.evaluate(el=>el.blur());
    const author=row===pending?row.locator('[data-author-info] button'):row.locator('.task-row-byline [data-slot="tooltip-trigger"]');
    if(await author.getAttribute('title')!==null)throw new Error('author still uses native title');
    await author.hover();await page.getByRole('tooltip').filter({hasText:'测试作者'}).waitFor();
    await checkTooltipBounds();
    await page.keyboard.press('Escape');await page.mouse.move(0,0);
  }
  await done.locator('.backup-sync-tooltip').hover();await page.getByRole('tooltip').filter({hasText:'已备份'}).waitFor();
  await checkTooltipBounds();
  await page.keyboard.press('Escape');await page.mouse.move(0,0);
  await done.locator('.backup-sync-tooltip').focus();await page.getByRole('tooltip').filter({hasText:'已备份'}).waitFor();
  await page.keyboard.press('Escape');await done.locator('.backup-sync-tooltip').evaluate(el=>el.blur());
  const longTitle=done.locator('.task-row-title');
  await longTitle.hover();await page.getByRole('tooltip').filter({hasText:'长标题'}).waitFor();await checkTooltipBounds();
  if(!await page.locator('[data-slot="tooltip-content"]').last().evaluate(el=>el.scrollHeight>el.clientHeight&&getComputedStyle(el).overflowY==='auto'))throw new Error('long tooltip is not scrollable');
  await page.keyboard.press('Escape');await page.mouse.move(0,0);
  await longTitle.focus();await page.getByRole('tooltip').filter({hasText:'长标题'}).waitFor();await checkTooltipBounds();
  await page.keyboard.press('Escape');await longTitle.evaluate(el=>el.blur());
  if(!await remote.locator('svg.lucide-circle-arrow-down circle').count())throw new Error('circled download icon missing');
  await remote.getByRole('img',{name:'封面暂不可用',exact:true}).waitFor();
  await page.evaluate(()=>{window.coverFails=false;});await page.getByRole('button',{name:'刷新备份',exact:true}).click();await remote.locator('img').waitFor();
  await page.getByRole('tab',{name:'抖音',exact:true}).click();
  if(!(await page.locator('.task-status-tabs [aria-label="全部"]').textContent()).endsWith('0'))throw new Error('platform count includes remote from other platform');
  await page.getByRole('tab',{name:'全部',exact:true}).click();
  await remote.getByRole('button',{name:'查看文案',exact:true}).click();await page.getByRole('textbox',{name:'识别文案'}).waitFor();
  if(await page.getByRole('textbox',{name:'识别文案'}).inputValue()!=='无需下载视频即可读取的完整文案')throw new Error('text missing');
  await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'hidden'});
  if(await page.evaluate(()=>window.backupCalls.some(c=>c==='ensure_local_video'||c==='prepare_asset_playback')))throw new Error('browsing restored video');
  await remote.focus();await page.keyboard.press('Shift+F10');
  if(await page.getByRole('menuitem',{name:'上传云端',exact:true}).getAttribute('aria-disabled')!=='true')throw new Error('remote-only asset can upload');
  await page.getByRole('menuitem',{name:'从远端下载',exact:true}).click();
  await remote.getByRole('button',{name:'取消下载',exact:true}).waitFor();
  await remote.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'从远端下载',exact:true}).getAttribute('aria-disabled')!=='true')throw new Error('menu allows duplicate restore');
  await page.keyboard.press('Escape');
  await remote.getByRole('button',{name:'取消下载',exact:true}).click();
  await remote.getByRole('button',{name:'下载到本机',exact:true}).waitFor();
  await remote.getByRole('button',{name:'下载到本机',exact:true}).click();
  if(await page.getByRole('menu').count())throw new Error('download opened a menu');
  await remote.getByRole('button',{name:'取消下载',exact:true}).waitFor();
  await page.waitForFunction(()=>document.querySelector('[data-asset-id="asset-3"] [role="progressbar"]')?.getAttribute('aria-valuenow')==='25');
  await page.evaluate(()=>{window.ops[0].payload.completed_bytes=75;});
  await page.waitForFunction(()=>document.querySelector('[data-asset-id="asset-3"] [role="progressbar"]')?.getAttribute('aria-valuenow')==='75');
  if(await remote.locator('.backup-sync-icon svg').last().evaluate(el=>getComputedStyle(el).clipPath)!=='inset(25% 0px 0px)')throw new Error('circle download fill not tied to bytes');
  if(!(await page.locator('.task-status-tabs [aria-label="下载中"]').textContent()).endsWith('1'))throw new Error('restore omitted from downloading count');
  await page.locator('.task-status-tabs [aria-label="下载中"]').click();await remote.waitFor();await page.locator('.task-status-tabs [aria-label="全部"]').click();
  await remote.getByRole('button',{name:'取消下载',exact:true}).click();
  await remote.getByRole('button',{name:'下载到本机',exact:true}).waitFor();
  await remote.getByRole('button',{name:'播放视频',exact:true}).click();await page.getByRole('dialog').waitFor();await page.waitForFunction(()=>typeof window.completeRestore==='function');
  await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'hidden'});await page.evaluate(()=>window.completeRestore());
  if(await page.getByRole('dialog').count())throw new Error('late restore reopened player');
  await page.getByRole('button',{name:'切换为信息流',exact:true}).click();
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});if(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw new Error('feed overflow');
    for(const row of [pending,remote]) {
      const {title,icon}=await row.evaluate(el=>({title:el.querySelector('.task-row-title').getBoundingClientRect().toJSON(),icon:el.querySelector('.task-title-backup').getBoundingClientRect().toJSON()}));
      if(!title||!icon||Math.abs(title.x-icon.x-icon.width-4)>1||Math.abs(title.y+10-icon.y-icon.height/2)>1)throw new Error(`backup icon not aligned before first title line: ${JSON.stringify({width,title,icon})}`);
      if(await row.locator('.task-row-heading .backup-sync-button').count()!==1||await row.locator('.task-row-cover .backup-sync-button').count())throw new Error('backup icon duplicated or still on cover');
      const spacing=await row.evaluate(el=>{const heading=el.querySelector('.task-row-heading').getBoundingClientRect(),byline=el.querySelector('.task-row-byline').getBoundingClientRect();return {height:heading.height,gap:byline.top-heading.bottom};});
      if(spacing.height!==20||Math.abs(spacing.gap-6)>0.1)throw new Error('single-line title reserves extra space');
      const size=await row.locator('.backup-sync-icon').boundingBox();if(size.width!==14||size.height!==14)throw new Error('sync icon too large');
    }
  }
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for(const row of [done,remote]) {
      const title=row.locator('.task-row-title'),original=await title.textContent();
      await title.evaluate(el=>{el.textContent='思想超越认知，其实是一种危险的早熟。#哲学 #心理学 #精神分析 #认知 #早熟'.repeat(5);});
      const layout=await row.evaluate(el=>{const t=el.querySelector('.task-row-title'),r=t.getBoundingClientRect(),h=el.querySelector('.task-row-heading').getBoundingClientRect(),icon=el.querySelector('.task-title-backup').getBoundingClientRect();return {height:r.height,heading:h.height,clipped:t.scrollWidth>t.clientWidth,ellipsis:getComputedStyle(t).textOverflow,aligned:Math.abs(icon.y+icon.height/2-r.y-r.height/2)<0.1};});
      if(layout.height!==20||layout.heading!==20||!layout.clipped||layout.ellipsis!=='ellipsis'||!layout.aligned)throw new Error('long title wraps or displaces icon');
      await title.evaluate((el,text)=>{el.textContent=text;},original);
    }
  }
  await pending.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'上传云端',exact:true}).getAttribute('aria-disabled')==='true')throw new Error('feed menu blocks eligible upload');
  await page.keyboard.press('Escape');
  await remote.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'从远端下载',exact:true}).getAttribute('aria-disabled')==='true')throw new Error('feed menu blocks eligible restore');
  await page.keyboard.press('Escape');
  const backupStyle=await pending.locator('.task-title-backup button').evaluate(el=>({bg:getComputedStyle(el).backgroundColor,color:getComputedStyle(el).color}));
  if(backupStyle.bg!=='rgba(0, 0, 0, 0)'||backupStyle.color!=='rgb(58, 91, 199)')throw new Error('backup should be transparent with contrasting color');
  if(await pending.locator('.task-title-backup button').evaluate(el=>getComputedStyle(el).borderTopWidth)!=='0px')throw new Error('backup button still has a border');
  const completeStyle=await done.locator('.task-title-backup button').evaluate(el=>({bg:getComputedStyle(el).backgroundColor,opacity:getComputedStyle(el).opacity}));
  if(completeStyle.bg!==backupStyle.bg||completeStyle.opacity!=='1')throw new Error('completed backup background faded');
  if(await done.locator('.task-title-backup svg').evaluate(el=>getComputedStyle(el).color)!=='rgb(32, 131, 104)')throw new Error('completed check lost green color');
  await page.emulateMedia({reducedMotion:'no-preference'});
  for(const row of [pending,done]) {
    const button=row.locator('.task-title-backup button');
    const glyph=row.locator('.backup-sync-glyph');
    await glyph.scrollIntoViewIfNeeded();
    const before=await glyph.boundingBox();
    const buttonBefore=await button.boundingBox();
    await row.locator('.task-title-backup').hover();
    const stationary=await glyph.evaluate(async(el)=>{
      const start=el.getBoundingClientRect(),center=[start.x+start.width/2,start.y+start.height/2];
      const until=performance.now()+300;
      while(performance.now()<until){await new Promise(requestAnimationFrame);const r=el.getBoundingClientRect();if(Math.abs(r.x+r.width/2-center[0])>0.1||Math.abs(r.y+r.height/2-center[1])>0.1)return false;}return true;
    });
    if(!stationary)throw new Error('backup icon center moves during hover');
    await page.waitForFunction(el=>getComputedStyle(el).transform==='matrix(1.08, 0, 0, 1.08, 0, 0)',await glyph.elementHandle());
    const after=await glyph.boundingBox();
    if(Math.abs(before.x+before.width/2-after.x-after.width/2)>0.1||Math.abs(before.y+before.height/2-after.y-after.height/2)>0.1||JSON.stringify(buttonBefore)!==JSON.stringify(await button.boundingBox()))throw new Error(`hover shifts icon or button: ${JSON.stringify({before,after,buttonBefore,buttonAfter:await button.boundingBox()})}`);
    if(await glyph.evaluate(el=>getComputedStyle(el).transitionDuration)!=='0.25s')throw new Error('backup hover transition missing');
    await page.mouse.move(0,0);
    await page.waitForFunction(el=>getComputedStyle(el).transform==='none',await glyph.elementHandle());
  }
  await page.emulateMedia({reducedMotion:'reduce'});
  await done.locator('.task-title-backup').hover();
  if(await done.locator('.backup-sync-glyph').evaluate(el=>getComputedStyle(el).transform)!=='none')throw new Error('backup hover ignores reduced motion');
  if(!await done.locator('.task-title-backup button').isDisabled())throw new Error('completed status became actionable');
  await page.mouse.move(0,0);
  await page.screenshot({path:'output/playwright/s3-feed-icons-fixed.png'});
  await page.getByRole('button',{name:'切换为列表',exact:true}).click();
  for(const row of [pending,remote]) {
    const title=await row.locator('.task-row-title').boundingBox(),icon=await row.locator('.task-title-backup').boundingBox();
    if(!title||!icon||Math.abs(title.x-icon.x-icon.width-4)>1)throw new Error('list backup icon not before title');
  }
  await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'远端备份',exact:true}).click();
  const keyInput=page.getByLabel('Access Key ID',{exact:true}),secretInput=page.getByLabel('Secret Access Key',{exact:true});
  await page.waitForFunction(()=>document.querySelector('#backup-secret').value==='saved-secret');
  if(await keyInput.inputValue()!=='saved-key'||await secretInput.getAttribute('type')!=='password')throw new Error('masked credentials not populated');
  await page.getByRole('button',{name:'显示 Secret Access Key',exact:true}).click();
  if(await secretInput.getAttribute('type')!=='text'||await secretInput.inputValue()!=='saved-secret')throw new Error('reveal failed');
  await page.getByRole('button',{name:'隐藏 Secret Access Key',exact:true}).click();
  await page.getByRole('button',{name:'显示 Access Key ID',exact:true}).click();
  if(await keyInput.getAttribute('type')!=='text')throw new Error('key reveal failed');
  await page.getByRole('button',{name:'隐藏 Access Key ID',exact:true}).click();
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});
    for(const selector of ['#backup-endpoint','#backup-bucket','#backup-prefix','#backup-key','#backup-secret']){
      await page.locator(selector).focus();
      const clear=await page.locator(selector).evaluate(el=>{
        const control=el.closest('[data-slot="input-group"]')||el;
        const rect=control.getBoundingClientRect();
        const clip=el.closest('.backup-settings-scroll').getBoundingClientRect();
        return rect.left-clip.left>=3.9 && clip.right-rect.right>=3.9;
      });if(!clear)throw new Error('focused input outline clipped');
    }
  }
  await page.screenshot({path:'output/playwright/s3-settings-focus-fixed.png'});
  await page.getByLabel('Bucket',{exact:true}).fill('unsaved-bucket');
  await page.getByLabel('Access Key ID',{exact:true}).fill('fixture-key');await page.getByLabel('Secret Access Key',{exact:true}).fill('fixture-secret');
  await page.getByRole('button',{name:'测试连接',exact:true}).click();
  const testing=page.getByRole('button',{name:'测试中…',exact:true});await testing.waitFor();
  if(await testing.getAttribute('aria-busy')!=='true'||!await testing.isDisabled()||!await testing.locator('[data-slot="spinner"]').count())throw new Error('test loading feedback missing');
  if(await page.evaluate(()=>window.testDraft.bucket!=='unsaved-bucket'||window.backupCalls.includes('save_backup_settings')))throw new Error('test did not use unsaved draft');
  await page.evaluate(()=>window.finishTest());await page.locator('[data-sonner-toast]').filter({hasText:'连接正常'}).waitFor();
  if(await page.locator('form[aria-label="远端备份设置"] [role="alert"]').count())throw new Error('inline feedback present');
  await page.getByRole('button',{name:'测试连接',exact:true}).click();await page.waitForFunction(()=>document.querySelector('[aria-busy="true"]'));
  await page.evaluate(()=>window.failTest());await page.locator('[data-sonner-toast]').filter({hasText:'无法连接远端存储'}).first().waitFor();
  if(await page.getByRole('combobox',{name:'本机角色'}).count())throw new Error('legacy role selector remains');
  await page.getByLabel('Access Key ID',{exact:true}).fill('fixture-key');await page.getByLabel('Secret Access Key',{exact:true}).fill('fixture-secret');
  // Saving a new target intentionally removes its creation-only name field.
  await page.getByRole('textbox',{name:'空间名称',exact:true}).waitFor();
  await page.getByRole('button',{name:'新建配置',exact:true}).click();
  await page.getByRole('button',{name:'保存中…',exact:true}).waitFor();
  await page.getByRole('button',{name:'保存配置',exact:true}).waitFor();
  if(await page.getByRole('textbox',{name:'空间名称',exact:true}).count())throw new Error('saved space retains creation-only name field');
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.evaluate(()=>{window.holdSave=true;});
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.getByRole('button',{name:'保存配置',exact:true}).scrollIntoViewIfNeeded();
    const snapshot=async()=>page.evaluate(()=>{
      const form=document.querySelector('form[aria-label="远端备份设置"]');
      const button=form.querySelector('.backup-save-button'),input=form.querySelector('#backup-bucket'),group=form.querySelector('[data-slot="input-group"]');
      const box=el=>{const r=el.getBoundingClientRect();return [r.x,r.y,r.width,r.height];};
      return {form:box(form),button:box(button),inputOpacity:getComputedStyle(input).opacity,inputBackground:getComputedStyle(input).backgroundColor,groupOpacity:getComputedStyle(group).opacity,groupBackground:getComputedStyle(group).backgroundColor,scroll:form.parentElement.scrollTop};
    });
    await page.evaluate(()=>{window.savedFormNode=document.querySelector('form[aria-label="远端备份设置"]');window.savedInputNode=document.querySelector('#backup-bucket');});
    const before=await snapshot();
    await page.getByRole('button',{name:'保存配置',exact:true}).click();
    const saving=page.getByRole('button',{name:'保存中…',exact:true});await saving.waitFor();
    if(await saving.getAttribute('aria-busy')!=='true'||!await saving.isDisabled()||!await saving.locator('[data-slot="spinner"]').count())throw new Error('save loading feedback missing');
    const during=await snapshot();
    if(JSON.stringify(before)!==JSON.stringify(during))throw new Error(`save dims fields or shifts layout: ${JSON.stringify({before,during})}`);
    await page.evaluate(()=>window.finishSave());
    await page.getByRole('button',{name:'保存配置',exact:true}).waitFor();
    if(JSON.stringify(before)!==JSON.stringify(await snapshot()))throw new Error('save completion shifts layout');
    if(!await page.evaluate(()=>window.savedFormNode===document.querySelector('form[aria-label="远端备份设置"]')&&window.savedInputNode===document.querySelector('#backup-bucket')))throw new Error('save remounted form');
  }
  await page.getByText('备份设置已保存',{exact:true}).first().waitFor();
  if(await page.locator('.backup-settings-form button[type="submit"] .backup-save-label').evaluate(el=>getComputedStyle(el).transitionDuration)!=='0.25s')throw new Error('save fade missing');
  await page.getByRole('button',{name:'保存配置',exact:true}).click();
  await page.getByRole('button',{name:'保存中…',exact:true}).waitFor();
  await page.evaluate(()=>window.failSave());
  await page.getByRole('button',{name:'保存配置',exact:true}).waitFor();
  await page.locator('[data-sonner-toast]').filter({hasText:'无法连接远端存储'}).first().waitFor();
  if(await page.locator('form[aria-label="远端备份设置"] [role="alert"]').count())throw new Error('save failure rendered inline');
  await page.emulateMedia({reducedMotion:'reduce'});
  if(await page.locator('.backup-settings-form button[type="submit"] .backup-save-label').evaluate(el=>getComputedStyle(el).transitionDuration)!=='0s')throw new Error('save ignores reduced motion');
  if(await secretInput.inputValue()!=='fixture-secret'||await secretInput.getAttribute('type')!=='password')throw new Error('saved secret not masked and retained');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  if(!await pending.getByRole('button',{name:'上传备份',exact:true}).isEnabled()||!await remote.getByRole('button',{name:'下载到本机',exact:true}).isEnabled())throw new Error('reader action permissions incorrect');
  await pending.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'上传云端',exact:true}).getAttribute('aria-disabled')==='true')throw new Error('equal member menu blocks upload');
  await page.keyboard.press('Escape');
  await remote.click({button:'right'});
  if(await page.getByRole('menuitem',{name:'从远端下载',exact:true}).getAttribute('aria-disabled')==='true')throw new Error('reader menu blocks restore');
  await page.keyboard.press('Escape');
  await remote.getByRole('button',{name:'更多操作',exact:true}).click();
  if(!await page.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).count())throw new Error('equal member delete missing');await page.keyboard.press('Escape');
  await page.evaluate(()=>{window.backupOffline=true;});await page.getByRole('button',{name:'刷新备份',exact:true}).click();await page.locator('[data-sonner-toast]').filter({hasText:'无法连接远端存储'}).first().waitFor();await remote.waitFor();
  await page.evaluate(()=>{window.backupOffline=false;window.ops.find(o=>o.asset_id==='asset-1').last_error='BACKUP_SERVICE_UNAVAILABLE';});
  const serviceToast=page.locator('[data-sonner-toast]').filter({hasText:'远端服务暂时不可用'});await serviceToast.waitFor();
  if(await page.getByRole('alert').filter({hasText:'远端服务暂时不可用'}).count())throw new Error('remote error rendered inline');
  await serviceToast.waitFor({state:'hidden',timeout:10000});
  await page.emulateMedia({reducedMotion:'reduce'});await page.screenshot({path:'output/playwright/s3-task-icons-1024.png'});
  page.off('pageerror',onError);if(errors.length)throw new Error(errors.join('\n'));
  return {result:'PASS',viewports:3,views:2,uploadPercent:50,restorePercent:[25,75],browsingVideoRequests:0};
}
