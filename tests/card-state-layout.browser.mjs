// Full application with isolated IPC. No real account, media, filesystem or network transfer.
async page => {
  const origin = page.url().startsWith('http') ? page.url().split('/').slice(0,3).join('/') : 'http://127.0.0.1:1432';
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  for (const [width,height] of [[1440,900],[1920,1080],[2560,1440],[1280,800],[1024,680]]) {
    const p = await page.context().newPage(); p.setDefaultTimeout(10000);
    const errors = []; p.on('pageerror', error=>errors.push(error.message));
    await p.addInitScript(()=>{
      localStorage.setItem('yingqu.task-view','feed'); window.calls=[];
      const video={platform:'telegram',resolver:'grammers',kind:'video',video_id:'123_1',title:'统一卡片状态与间距',canonical_url:'https://t.me/c/123/1',author:'测试频道',author_profile:{id:'123',nickname:'测试频道',kind:'channel'},filesize:1000000,width:1920,height:1080,duration_ms:32000};
      const base={revision:1,video,bytes:300000,total_bytes:1000000,total_bytes_estimate:null,speed_bps:100000,eta_seconds:7,path:null,created_at:100,completed_at:null,canceling:false,file_missing:false,download_requested:true,cover_cached:false,files:[],error:null,cover_error:null,queue_order:null,telegram_link:'https://t.me/c/123/1'};
      window.tasks=[
        {...base,id:'parsing',status:'parsing',video:null},
        {...base,id:'ready',status:'ready'},
        {...base,id:'queued',status:'queued'},
        {...base,id:'deferred',status:'queued',video:null},
        {...base,id:'downloading',status:'downloading'},
        {...base,id:'unknown',status:'downloading',total_bytes:null,video:{...video,filesize:null},speed_bps:null,eta_seconds:null},
        {...base,id:'merging',status:'downloading',merging:true},
        {...base,id:'canceling',status:'downloading',canceling:true},
        {...base,id:'completed',status:'completed',bytes:1000000,path:'C:/isolated/video.mp4',completed_at:1700000000},
        {...base,id:'failed',status:'failed',error:'TELEGRAM_TIMEOUT'},
        {...base,id:'canceled',status:'canceled'},
        {...base,id:'missing',status:'completed',file_missing:true,path:'C:/isolated/missing.mp4',completed_at:1700000000},
        {...base,id:'cover-warning',status:'completed',cover_error:'COVER_UNAVAILABLE',path:'C:/isolated/cover.mp4',completed_at:1700000000},
      ];
      const asset=(id,local='missing')=>({asset_id:id,task_id:null,metadata:video,original_filename:'video.mp4',content_bytes:1000000,asset_state:'active',path:local==='present'?'C:/isolated/video.mp4':null,local_state:local,space_id:'space',remote_state:'backed_up',has_cover:false,has_transcript:false,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0,in_current_space:true});
      window.assets=[asset('remote-only'),asset('remote-local','present'),asset('remote-queued'),asset('remote-running','restoring'),asset('remote-failed'),asset('remote-upload','present'),asset('remote-delete','present')];
      const op=(id,kind,state,error=null)=>({operation_id:id+'-op',asset_id:id,space_id:'space',kind,state,phase:kind==='restore'?'downloading':'queued',attempt:1,payload:{completed_bytes:300000},last_error:error});
      window.ops=[op('remote-queued','restore','pending'),op('remote-running','restore','running'),op('remote-failed','restore','blocked','BACKUP_NETWORK_ERROR'),op('remote-upload','upload','blocked','BACKUP_SOURCE_CHANGED'),op('remote-delete','delete','blocked','BACKUP_ACCESS_DENIED')];
      const callbacks=new Map(),listeners=[];let next=1;
      window.patchTask=(id,changes)=>{const task=window.tasks.find(t=>t.id===id);Object.assign(task,changes);task.revision++;for(const l of listeners.filter(l=>l.event==='task_updated'))callbacks.get(l.handler)?.({event:'task_updated',id:l.id,payload:structuredClone(task)});};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:callback=>{const id=next++;callbacks.set(id,callback);return id;},unregisterCallback:id=>callbacks.delete(id),invoke:async(command,args)=>{
        window.calls.push({command,args});
        if(command==='get_settings')return {download_directory:'C:/isolated',filename_template:'{content}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return structuredClone(window.tasks);
        if(command==='get_backup_settings')return {space_id:'space',library_id:'library',provider_type:'s3',profile:'r2',enabled:true,credentials_configured:true,last_error:null};
        if(command==='list_backup_spaces')return [];
        if(command==='list_backup_assets')return structuredClone(window.assets);
        if(command==='list_backup_operations')return structuredClone(window.ops);
        if(command==='list_video_transcripts')return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/isolated',tools:[]};
        if(command==='plugin:event|listen'){const id=next++;listeners.push({...args,id});return id;}
        if(command==='cancel_task'){window.patchTask(args.taskId,{status:'canceled',canceling:false});return null;}
        if(command==='delete_asset_and_backup')throw 'BACKUP_ACCESS_DENIED';
        return null;
      }};
    });
    let step = 'load';
    try {
      await p.setViewportSize({width,height}); await p.goto(origin+'/',{waitUntil:'domcontentloaded'});
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      await p.locator('[data-asset-id="remote-failed"]').waitFor();
      step = 'state geometry';
      const grid=await p.locator('.task-list[data-view="feed"]').evaluate(el=>({width:el.clientWidth,columns:getComputedStyle(el).gridTemplateColumns.split(' ').length}));
      const cardWidth=(await p.locator('[data-task-id]').first().boundingBox()).width;
      check(grid.columns===Math.max(1,Math.floor((grid.width+16)/(260+16))) && cardWidth>=260,'grid did not adapt to its available width and minimum card width');
      check(!await p.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'responsive grid overflowed window');
      if(width===1920||width===2560)check(grid.columns>results[0].columns,'wide window retained an artificial column cap');
      const snapshots=await p.locator('[data-task-id], [data-asset-id]').evaluateAll(nodes=>nodes.map(el=>{
        const card=el.getBoundingClientRect(),cover=el.querySelector('.task-row-cover').getBoundingClientRect(),info=el.querySelector('.task-row-information').getBoundingClientRect();
        const badge=el.querySelector('.task-feed-status')?.getBoundingClientRect(),action=el.querySelector('.task-feed-play')?.getBoundingClientRect();
        return {id:el.dataset.taskId||el.dataset.assetId,height:card.height,info:info.height,infoOverflow:[...el.querySelector('.task-row-information').children].some(child=>child.getBoundingClientRect().bottom>info.bottom+1),cover:cover.height,badge:badge?{x:badge.x-cover.x,y:badge.y-cover.y,h:badge.height}:null,action:action?{x:cover.right-action.right,y:action.y-cover.y,w:action.width,h:action.height}:null,footer:!!el.querySelector('.task-row-footer')};
      }));
      for(const item of snapshots){
        check(Math.abs(item.height-snapshots[0].height)<1,'unequal state height '+JSON.stringify(item));
        check(Math.abs(item.info-92)<1,'information area grew '+JSON.stringify(item));
        check(!item.infoOverflow,'information clipped '+JSON.stringify(item));
        check(!item.footer,'duplicate footer '+item.id);
        check(item.badge && item.badge.x===8 && item.badge.y===12 && item.badge.h===24,'badge anchor '+JSON.stringify(item));
        if(item.action)check(item.action.x===8 && item.action.y===8 && item.action.w===32 && item.action.h===32,'action anchor '+JSON.stringify(item));
      }
      const row=id=>p.locator('[data-task-id="'+id+'"]');
      for(const [id,label] of [['parsing','解析中'],['ready','已解析，可下载'],['queued','等待中'],['deferred','等待解析'],['downloading','下载中'],['merging','合并中'],['canceling','正在取消…'],['completed','已完成'],['failed','下载失败'],['canceled','已取消']])check(await row(id).locator('.task-feed-status').innerText()===label,'status label '+id);
      const failed=row('failed');const original=await failed.boundingBox();
      await p.evaluate(()=>window.patchTask('failed',{error:'INTERRUPTED'}));
      await failed.locator('.task-feed-status').filter({hasText:'已中断'}).waitFor();
      await failed.getByRole('button',{name:'重试下载',exact:true}).hover();
      await p.getByRole('tooltip',{name:'任务已中断，点击重试下载',exact:true}).waitFor();
      await p.mouse.move(0,0);
      for(const error of [null,'TELEGRAM_TIMEOUT','TELEGRAM_FLOOD_WAIT:'+'1'.repeat(1200)]){
        step = 'error toggle '+String(error).slice(0,20);
        const expected=await p.evaluate(async error=>{const {errorText}=await import('/src/api.ts');return error?errorText(error):null;},error);
        await p.evaluate(error=>window.patchTask('failed',{error}),error);
        await p.waitForFunction(expected=>{const n=document.querySelector('[data-task-id="failed"] [role="alert"]');return expected?n?.textContent.includes(expected):!n;},expected);
        const box=await failed.boundingBox();check(Math.abs(box.height-original.height)<1,'error visibility resized card');
      }
      step = 'error tooltip';
      const notice=failed.getByRole('alert');await notice.scrollIntoViewIfNeeded();
      await p.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      await notice.evaluate(el=>el.focus({preventScroll:true}));
      const tooltip=p.getByRole('tooltip').filter({hasText:'Telegram 请求受限'});await tooltip.waitFor();
      check((await tooltip.innerText()).length>1000,'full error missing from keyboard tooltip');
      const hint=await tooltip.boundingBox();check(hint.x>=0&&hint.y>=0&&hint.x+hint.width<=width&&hint.y+hint.height<=height,'error tooltip overflow');
      await p.keyboard.press('Escape');
      const specs=await failed.locator('.task-row-metadata').boundingBox(),message=await notice.boundingBox();
      check(message.y>=specs.y+specs.height && message.y-specs.y-specs.height<=5,'error is not directly below resolution and size');
      check(await failed.locator('.task-row-cover [role="alert"]').count()===0,'error still covers the image');
      check(await notice.evaluate(el=>!!el.closest('.task-row-information')),'error escaped the information area');
      check(message.height===18,'error line grew');
      step = 'download feedback keyboard';
      await p.evaluate(()=>window.patchTask('failed',{status:'downloading',bytes:47.5*1024*1024,total_bytes:Math.round(100.1*1024*1024),speed_bps:2.9*1024*1024,eta_seconds:57}));
      await failed.locator('.task-feed-status').filter({hasText:'下载中'}).waitFor();
      const telemetry=failed.locator('.task-feed-telemetry');
      const items=telemetry.locator('> span');
      const telemetryBounds=await items.evaluateAll(nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return {top:r.top,height:r.height,width:r.width};}));
      check(telemetryBounds.length===3 && telemetryBounds.every(r=>r.height===18 && r.width>0 && Math.abs(r.top-telemetryBounds[0].top)<1),'bytes, speed and ETA wrapped or disappeared');
      for(const item of await items.all()) {
        if(await item.evaluate(n=>n.clientWidth<n.scrollWidth)) {
          const fullText=await item.innerText();
          await item.hover();
          await p.getByRole('tooltip',{name:fullText,exact:true}).waitFor();
        }
      }
      check(await items.nth(1).evaluate(n=>n.clientWidth>=n.scrollWidth),'speed was truncated');
      if(width===1440) {
        for(const resizeWidth of [1280,1440,1759,1760,1920,2560,1440]) {
          await p.setViewportSize({width:resizeWidth,height});
          await p.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
          const resized=await telemetry.evaluate(el=>{const rect=el.getBoundingClientRect();return {width:rect.width,items:[...el.children].map(n=>({top:n.getBoundingClientRect().top,width:n.getBoundingClientRect().width}))};});
          check(resized.items.every(n=>n.width>0 && Math.abs(n.top-resized.items[0].top)<1),'live resize hid or wrapped download info at '+resizeWidth);
          check(await items.nth(1).evaluate(n=>n.clientWidth>=n.scrollWidth),'live resize truncated speed at '+resizeWidth);
          check(await p.locator('.task-list[data-view="feed"]').evaluate(el=>getComputedStyle(el).gridTemplateColumns.split(' ').length===Math.max(1,Math.floor((el.clientWidth+16)/(260+16)))),'live resize failed to adapt its column count');
        }
      }
      const firstLine=await items.first().boundingBox();
      await items.first().focus();
      for(const next of [items.nth(1),items.nth(2),notice]) {
        await p.keyboard.press('Tab');
        check(await next.evaluate(el=>el===document.activeElement),'single-line info or error lost keyboard access');
      }
      check(Math.abs((await items.first().boundingBox()).y-firstLine.y)<1,'keyboard scrolled byte count away');
      check(Math.abs((await failed.boundingBox()).height-original.height)<1,'download error resized card');
      step = 'human readable remaining time';
      for (const [seconds, expected] of [[0,'0 秒'],[2.5,'3 秒'],[59,'59 秒'],[60,'60 秒'],[61,'61 秒'],[299,'299 秒'],[299.1,'300 秒'],[300,'300 秒'],[300.1,'5 分 1 秒'],[301,'5 分 1 秒'],[3599,'59 分 59 秒'],[3600,'1 小时'],[3661,'1 小时 1 分 1 秒'],[90061,'25 小时 1 分 1 秒']]) {
        await p.evaluate(seconds=>window.patchTask('failed',{eta_seconds:seconds}),seconds);
        await items.last().filter({hasText:'预计剩余 '+expected}).waitFor();
        check(await items.last().innerText()==='预计剩余 '+expected,'wrong ETA for '+seconds);
        check(Math.abs((await items.first().boundingBox()).y-(await items.last().boundingBox()).y)<1,'formatted ETA wrapped');
      }
      await items.last().hover();
      await p.getByRole('tooltip',{name:'预计剩余 25 小时 1 分 1 秒',exact:true}).waitFor();
      await p.mouse.move(0,0);
      await p.evaluate(()=>window.patchTask('failed',{status:'failed'}));
      await failed.locator('.task-feed-status').filter({hasText:'下载失败'}).waitFor();
      const remote=p.locator('[data-asset-id="remote-upload"]');const remoteHeight=(await remote.boundingBox()).height;
      for(const error of [null,'BACKUP_SOURCE_CHANGED']){
        await p.evaluate(error=>{window.ops.find(op=>op.asset_id==='remote-upload').last_error=error;},error);
        await p.waitForFunction(hasError=>!!document.querySelector('[data-asset-id="remote-upload"] [role="alert"]')===hasError,!!error);
        check(Math.abs((await remote.boundingBox()).height-remoteHeight)<1,'backup error resized card');
      }
      await p.screenshot({path:`.local/card-min-260/states-${width}.png`});
      check(await row('ready').getByRole('button',{name:'下载视频',exact:true}).count()===1,'ready download unreachable');
      check(await row('parsing').getByRole('button',{name:'取消解析',exact:true}).count()===1,'parse cancellation unreachable');
      check(await row('queued').getByRole('button',{name:'取消下载',exact:true}).count()===1,'queued cancellation lost');
      for(const id of ['remote-queued','remote-running'])check(await p.locator('[data-asset-id="'+id+'"]').getByRole('progressbar',{name:'恢复下载进度',exact:true}).count()===1,'restore progress missing '+id);
      check(await p.locator('[data-asset-id="remote-failed"]').getByRole('button',{name:'重试下载',exact:true}).count()===1,'remote retry missing');
      step = 'deletion intent error';
      const deletionTarget=p.locator('[data-asset-id="remote-local"]');const deletionHeight=(await deletionTarget.boundingBox()).height;
      await deletionTarget.locator('.task-row-title').click({button:'right'});
      await p.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).click();
      const deletionNotice=deletionTarget.getByRole('alert');await deletionNotice.waitFor();
      const deletionText=await p.evaluate(async()=>{const {errorText}=await import('/src/api.ts');return errorText('BACKUP_ACCESS_DENIED');});
      check(await deletionNotice.innerText()===deletionText,'intent failure cause was lost or formatted twice');
      check(await p.evaluate(()=>!window.ops.some(op=>op.asset_id==='remote-local')),'fixture accidentally created a delete operation');
      check(Math.abs((await deletionTarget.boundingBox()).height-deletionHeight)<1,'intent error resized card');
      await p.getByRole('button',{name:'切换为列表',exact:true}).click();
      check(await failed.locator('.task-row-footer').count()===1 && await failed.locator('.task-row-information [role="alert"]').count()===1,'list error/actions changed');
      await p.evaluate(()=>window.patchTask('failed',{status:'downloading',eta_seconds:3661}));
      await failed.getByText('预计剩余 1 小时 1 分 1 秒',{exact:true}).waitFor();
      check(errors.length===0,errors.join('\n'));
      results.push({width,height,cards:snapshots.length,columns:grid.columns,cardWidth,cardHeight:original.height,errorHeightStable:true});
    } catch (error) { const detail=await p.evaluate(()=>({active:document.activeElement?.outerHTML.slice(0,400),notice:document.querySelector('[data-task-id="failed"] [role="alert"]')?.outerHTML.slice(0,700),hints:[...document.querySelectorAll('[role="tooltip"]')].map(n=>n.textContent.slice(0,80))}));throw Error(step+': '+error.message+' '+JSON.stringify(detail)); } finally {await p.close();}
  }
  return {passed:true,results};
}
