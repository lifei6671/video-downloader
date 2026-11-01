// Synthetic IPC progression. No account, local media or real transfer is used.
async page => {
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);
    const errors=[];p.on('pageerror',e=>errors.push(e.message));
    const check=(ok,message)=>{if(!ok)throw Error(width+': '+message);};
    await p.addInitScript(()=>{
      localStorage.setItem('yingqu.task-view','list');
      const video={title:'测试备份视频',platform:'telegram',resolver:'grammers',kind:'video',video_id:'100_9',canonical_url:'https://t.me/c/100/9',width:1920,height:1080,duration_ms:2000};
      const task={id:'local',asset_id:'asset',revision:1,status:'completed',video,path:'C:/fixture/video.mov',bytes:10000000,total_bytes:10000000,created_at:1,completed_at:2,canceling:false,file_missing:false,download_requested:true,cover_cached:false,files:[]};
      const space={space_id:'space',library_id:'library',display_name:'测试备份',provider_type:'telegram',profile:'telegram',enabled:true,credentials_configured:false,last_error:null,telegram:{account_id:'test',channel_id:'100'}};
      window.asset={asset_id:'asset',task_id:'local',metadata:video,original_filename:'video.mov',content_bytes:10000000,asset_state:'active',path:task.path,local_state:'present',space_id:'space',remote_state:'pending',has_cover:false,has_transcript:false,remote_revision:0,remote_deleted:false,has_pending_local_changes:false,conflict_count:0};
      window.operation={operation_id:'upload',space_id:'space',asset_id:'asset',kind:'upload',state:'pending',phase:'queued',payload:{completed_bytes:0},attempt:0,last_error:null};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async command=>{
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return [task];
        if(command==='get_backup_settings')return space;
        if(command==='list_backup_spaces')return [space];
        if(command==='get_backup_sync_enabled')return true;
        if(command==='list_backup_assets')return [structuredClone(window.asset)];
        if(command==='list_backup_operations')return [structuredClone(window.operation)];
        if(command==='list_video_transcripts')return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        return null;
      }};
    });
    try {
      await p.setViewportSize({width,height});await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      const row=p.locator('[data-task-id="local"]');
      const hint=async expected=>{
        await row.scrollIntoViewIfNeeded();await p.mouse.move(0,0);await row.locator('.backup-sync-tooltip').hover();
        const tooltip=p.getByRole('tooltip',{name:expected,exact:true});await tooltip.waitFor();
        const box=await tooltip.boundingBox();check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'tooltip overflow');
        await p.keyboard.press('Escape');await tooltip.waitFor({state:'hidden'});
      };
      for(const view of ['list','feed']) {
        if(view==='feed')await p.getByRole('button',{name:'切换为信息流',exact:true}).click();
        await p.evaluate(()=>{window.asset.remote_state='pending';window.operation.state='pending';window.operation.payload={completed_bytes:0};});
        await hint('待上传，点击取消上传');
        await p.evaluate(()=>window.operation.state='running');
        await hint('待上传，点击取消上传'); // validation/preparation has not transferred bytes
        await p.evaluate(()=>{window.operation.payload={completed_bytes:1,transfer_started:true};});
        await hint('备份中 0%，点击取消上传');
        check(await row.locator('.backup-sync-button [data-slot="spinner"]').count()===0,'first byte still shows queued spinner');
        await p.evaluate(()=>window.operation.payload.completed_bytes=6700000);
        await hint('备份中 67%，点击取消上传');
        await p.evaluate(()=>window.operation.payload.completed_bytes=10000000);
        await hint('备份中 99%，点击取消上传');
        check(await row.getByRole('img',{name:'已备份',exact:true}).count()===0,'upload bytes alone falsely completed backup');
        await p.evaluate(()=>window.operation.payload={transfer_started:true});
        await hint('备份中，点击取消上传'); // metadata-only upload has no video-byte denominator
        check(await row.locator('.backup-sync-button [data-slot="spinner"]').count()===1,'unknown progress should remain indeterminate');
        await p.evaluate(()=>window.operation.state='retry_wait');
        await hint('待上传，点击取消上传');
        await p.evaluate(()=>{window.operation.state='completed';window.asset.remote_state='backed_up';});
        await row.getByRole('img',{name:'已备份',exact:true}).waitFor();
        await hint('已备份');results.push({width,view,passed:true});
      }
      check(errors.length===0,errors.join('\n'));
    } finally {await p.close();}
  }
  return {passed:true,results};
}
