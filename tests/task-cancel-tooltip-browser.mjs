// Isolated IPC: no user files, accounts or network transfers.
async page => {
  const results = [];
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p = await page.context().newPage(); p.setDefaultTimeout(7000);
    const errors=[]; p.on('pageerror',e=>errors.push(e.message));
    const check=(ok,message)=>{if(!ok)throw Error(message);};
    await p.addInitScript(()=>{
      localStorage.setItem('yingqu.task-view','list'); window.calls=[]; window.ops=[];
      const video={platform:'youtube',resolver:'yt-dlp',kind:'video',video_id:'12345678901',title:'测试视频',canonical_url:'https://www.youtube.com/watch?v=12345678901'};
      const base={revision:1,video,bytes:50,total_bytes:100,path:null,created_at:1,completed_at:null,canceling:false,file_missing:false,download_requested:true,cover_cached:false,files:[]};
      window.tasks=[{...base,id:'download',status:'downloading'},{...base,id:'upload',asset_id:'asset',status:'completed',path:'C:/fixture/video.mp4',completed_at:2,video:{...video,title:'待备份测试'}}];
      const space={space_id:'space',library_id:'library',provider_type:'s3',profile:'s3',enabled:true,credentials_configured:false,last_error:null};
      const asset={asset_id:'asset',task_id:'upload',metadata:video,original_filename:'video.mp4',content_bytes:100,asset_state:'active',path:'C:/fixture/video.mp4',local_state:'present',space_id:'space',remote_state:'not_backed_up',has_cover:false,has_transcript:false,remote_revision:0,remote_deleted:false,has_pending_local_changes:false,conflict_count:0};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
        window.calls.push({command,args});
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return structuredClone(window.tasks);
        if(command==='get_backup_settings')return space;
        if(command==='list_backup_spaces')return [space];
        if(command==='list_backup_assets')return [asset];
        if(command==='list_backup_operations')return structuredClone(window.ops);
        if(command==='list_video_transcripts')return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(command==='start_asset_backup'){window.ops=[{operation_id:'upload-op',space_id:'space',asset_id:'asset',kind:'upload',state:'pending',phase:'prepare',payload:{},attempt:0,last_error:null}];return null;}
        if(command==='cancel_backup_operation'){window.ops=[];return null;}
        if(command==='cancel_task'){window.tasks[0].canceling=true;window.tasks[0].revision++;return null;}
        return null;
      }};
    });
    try {
      await p.setViewportSize({width,height});await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      const upload=p.locator('[data-task-id="upload"]');
      await upload.getByRole('button',{name:'上传备份',exact:true}).click();
      await upload.getByRole('button',{name:'取消上传',exact:true}).waitFor();
      await p.mouse.move(0,0);await upload.locator('.backup-sync-tooltip').hover();
      const hint=p.getByRole('tooltip',{name:'待上传，点击取消上传',exact:true});await hint.waitFor();
      const hintBox=await hint.boundingBox();check(hintBox.x>=0&&hintBox.y>=0&&hintBox.x+hintBox.width<=width&&hintBox.y+hintBox.height<=height,'upload hint overflow');
      await p.keyboard.press('Escape');await upload.getByRole('button',{name:'取消上传',exact:true}).click();
      check(await p.evaluate(()=>window.calls.some(c=>c.command==='cancel_backup_operation'&&c.args.operationId==='upload-op')),'upload cancel lost target');
      const cancel=p.locator('[data-task-id="download"]').getByRole('button',{name:'取消',exact:true});
      check(await cancel.innerText()===''&&await cancel.locator('svg.lucide-x').count()===1,'cancel is not icon only');
      const box=await cancel.boundingBox();check(box.width===32&&box.height===32,'cancel is not 32px');
      check(await cancel.getAttribute('data-variant')==='ghost','cancel has a button border');
      await cancel.hover();await p.getByRole('tooltip',{name:'取消下载',exact:true}).waitFor();await p.keyboard.press('Escape');
      await cancel.focus();await p.keyboard.press('Enter');
      const dialog=p.getByRole('alertdialog',{name:'取消当前任务？',exact:true});await dialog.waitFor();
      check(await p.evaluate(()=>!window.calls.some(c=>c.command==='cancel_task')),'canceled before confirmation');
      await dialog.getByRole('button',{name:'返回',exact:true}).click();await dialog.waitFor({state:'hidden'});
      await cancel.click();await dialog.getByRole('button',{name:'确认取消',exact:true}).click();
      await p.waitForFunction(()=>window.calls.some(c=>c.command==='cancel_task'&&c.args.taskId==='download'));
      await p.evaluate(()=>window.dispatchEvent(new Event('focus')));
      const canceling=p.locator('[data-task-id="download"]').getByRole('button',{name:'正在取消…',exact:true});await canceling.waitFor();
      check(await canceling.isDisabled()&&await canceling.locator('[data-slot="spinner"]').count()===1,'canceling lacks disabled spinner feedback');
      check(errors.length===0,errors.join('\n'));results.push({width,passed:true});
    } catch(error) { throw new Error(error.message+" details="+JSON.stringify(await p.evaluate(()=>({ops:window.ops,hints:[...document.querySelectorAll('[data-slot=tooltip-content], [role=tooltip]')].map(n=>({text:n.textContent,state:n.dataset.state})),buttons:[...document.querySelectorAll('.backup-sync-button')].map(n=>({label:n.getAttribute('aria-label'),disabled:n.disabled}))})))); } finally {await p.close();}
  }
  return {passed:true,viewports:results,checks:['queued upload tooltip and cancel','icon size and tooltip','keyboard','confirmation and back','canceling feedback']};
}
