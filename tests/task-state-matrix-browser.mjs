// Production renderer, synthetic IPC only. Never touches user tasks or media.
async page => {
  const checks=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);
    const errors=[];p.on('pageerror',e=>errors.push(e.message));
    const check=(ok,message)=>{if(!ok)throw Error(width+': '+message);};
    await p.addInitScript(()=>{
      localStorage.setItem('yingqu.task-view','list');window.calls=[];
      const video={platform:'youtube',resolver:'yt-dlp',kind:'video',video_id:'12345678901',canonical_url:'https://www.youtube.com/watch?v=12345678901',title:'测试视频',duration_ms:2500,width:1920,height:1080,filesize:100};
      const base={revision:1,status:'completed',video,bytes:50,total_bytes:100,speed_bps:10,path:'C:/fixture/video.mp4',created_at:1,completed_at:2,canceling:false,file_missing:false,download_requested:true,cover_cached:false,files:[]};
      window.ids=['parsing','ready','queued','deferred','downloading','unknown','merging','canceling','completed','missing','album','failed','canceled','failed-empty'];
      window.tasks=window.ids.map((id,i)=>{
        const task={...base,id,created_at:100-i,video:{...video,title:id},path:null,completed_at:null};
        task.status=['deferred'].includes(id)?'queued':['unknown','merging','canceling'].includes(id)?'downloading':['missing','album'].includes(id)?'completed':id==='failed-empty'?'failed':id;
        if(['parsing','deferred','failed-empty'].includes(id))task.video=null;
        if(id==='deferred')task.telegram_link='https://t.me/c/100/9';
        if(['completed','album','missing'].includes(id)){task.path='C:/fixture/'+id;task.completed_at=2;}
        if(id==='album'){task.video.kind='image_album';task.video.images=[];}
        if(id==='missing')task.file_missing=true;
        if(id==='unknown')task.total_bytes=null;
        if(id==='merging')task.merging=true;
        if(id==='canceling')task.canceling=true;
        if(id==='failed')task.error='DOWNLOAD_FAILED';
        return task;
      });
      const space={space_id:'space',library_id:'library',display_name:'测试备份',provider_type:'s3',profile:'s3',enabled:true,credentials_configured:false,last_error:null};
      window.assets=[];window.ops=[];
      for(const state of ['pending','running','retry_wait','blocked','canceled','completed']){
        const id='backup-'+state;
        window.tasks.push({...base,id,asset_id:id,video:{...video,title:id}});
        window.assets.push({asset_id:id,task_id:id,metadata:video,original_filename:'video.mp4',content_bytes:100,asset_state:'active',path:'C:/fixture/video.mp4',local_state:'present',space_id:'space',remote_state:['canceled','completed'].includes(state)?'backed_up':'pending',has_cover:false,has_transcript:false,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0});
        window.ops.push({operation_id:'op-'+state,space_id:'space',asset_id:id,kind:'upload',state,phase:'upload',payload:{completed_bytes:state==='running'?50:0},attempt:0,last_error:state==='blocked'||state==='retry_wait'?'BACKUP_SERVICE_UNAVAILABLE':null});
      }
      window.assets.push({asset_id:'remote',task_id:null,metadata:{...video,title:'远端视频'},original_filename:'remote.mp4',content_bytes:100,asset_state:'active',path:null,local_state:'restoring',space_id:'space',remote_state:'backed_up',has_cover:false,has_transcript:true,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0});
      window.ops.push({operation_id:'restore-op',space_id:'space',asset_id:'remote',kind:'restore',state:'pending',phase:'download',payload:{completed_bytes:0},attempt:0,last_error:null});
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
        window.calls.push({command,args});
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return structuredClone(window.tasks);
        if(command==='get_backup_settings')return space;
        if(command==='list_backup_spaces')return [space];
        if(command==='get_backup_sync_enabled')return true;
        if(command==='list_backup_assets')return structuredClone(window.assets);
        if(command==='list_backup_operations')return structuredClone(window.ops);
        if(command==='list_video_transcripts')return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(command==='cancel_backup_operation'){window.ops.find(o=>o.operation_id===args.operationId).state='canceled';return null;}
        if(command==='cancel_asset_restore'){window.ops.find(o=>o.asset_id===args.assetId).state='canceled';return null;}
        return null;
      }};
    });
    try {
      await p.setViewportSize({width,height});await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      const row=id=>p.locator('[data-task-id="'+id+'"]');
      const menu=()=>p.getByRole('menu',{name:'任务操作',exact:true});
      const menuSnapshot=async(target,remote=false)=>{
        await target.click({button:'right',force:true});
        const current=remote?p.getByRole('menu',{name:'远端视频操作',exact:true}):menu();await current.waitFor();
        check(await p.getByRole('menu').count()===1,'nested or duplicate menus');
        const values=await current.getByRole('menuitem').evaluateAll(items=>items.map(n=>({label:n.textContent.trim(),disabled:n.getAttribute('aria-disabled')==='true',icon:n.querySelector('svg')?.getAttribute('class'),variant:n.dataset.variant})));
        const box=await current.boundingBox();check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'menu overflow '+JSON.stringify(box));
        await p.keyboard.press('Escape');await current.waitFor({state:'hidden'});return values;
      };
      const inline=async target=>target.locator('button').evaluateAll(buttons=>buttons.filter(b=>!b.matches('[data-media-cover], .backup-sync-button')).map(b=>({label:b.getAttribute('aria-label')??b.textContent.trim(),disabled:b.disabled,icon:b.querySelector('svg')?.getAttribute('class'),spinner:!!b.querySelector('[data-slot="spinner"]')})));
      const signatures=new Map();
      for(const view of ['list','feed']){
        if(view==='feed')await p.getByRole('button',{name:'切换为信息流',exact:true}).click();
        for(const id of await p.evaluate(()=>window.ids)){
          const target=row(id);await target.waitFor();
          const entries=await menuSnapshot(target.locator('.task-row-title'));
          const coverEntries=await menuSnapshot(target.locator('[data-media-cover]'));
          check(JSON.stringify(entries)===JSON.stringify(coverEntries),id+' cover/body menu mismatch '+JSON.stringify({entries,coverEntries}));
          const controls=await inline(target);
          if(view==='list')signatures.set(id,{entries,controls});
          else {
            check(JSON.stringify(signatures.get(id).entries)===JSON.stringify(entries),id+' list/card menus differ');
            if(!['queued','deferred','downloading','unknown','merging','canceling'].includes(id))check(JSON.stringify(signatures.get(id).controls)===JSON.stringify(controls),id+' list/card actions, icons or disabled state differ');
          }
          const labels=entries.map(e=>e.label);
          if(['downloading','unknown','merging'].includes(id))check(labels.includes('取消下载')&&controls.some(c=>view==='list'?c.label==='取消'&&c.icon?.includes('lucide-x'):c.label==='取消下载'),id+' lost cancel');
          if(id==='canceling')check(entries.some(e=>e.label==='正在取消…'&&e.disabled)&&controls.some(c=>c.label==='正在取消…'&&c.disabled&&c.spinner),'canceling state inconsistent');
          if(view==='feed'&&['queued','deferred'].includes(id)){
            check(await target.locator('.task-row-footer,.task-queue-position').count()===0,id+' has queued footer or position');
            check(await target.locator('.task-row-cover .task-feed-status').innerText()===(id==='deferred'?'等待解析':'等待中'),id+' lost corner status');
            check(controls.length===1&&controls[0].label==='取消下载'&&!controls[0].disabled,id+' missing progress cancel');
            check(await target.getByRole('progressbar',{name:'等待下载',exact:true}).count()===1,id+' missing waiting progress');
          }
          if(['completed','missing','album','failed','canceled','failed-empty'].includes(id))check(labels.includes('从列表移除'),id+' cannot remove');
          if(id==='album')check(controls.some(c=>c.label==='查看图集'&&c.icon?.includes('lucide-images')),'album icon mismatch');
          if(id==='missing')check(!controls.some(c=>c.label==='播放视频')&&!labels.includes('打开文件夹'),'missing file has local playback/folder action');
          check(await target.locator('[title]').count()===0,id+' retains native title tooltip');
        }
        for(const state of ['pending','running','retry_wait','blocked','canceled','completed']){
          const target=row('backup-'+state);const active=['pending','running','retry_wait'].includes(state);
          await target.getByRole('button',{name:active?'取消上传':['canceled','completed'].includes(state)?'已备份':'上传备份',exact:true}).waitFor();
          const entries=await menuSnapshot(target.locator('.task-row-title'));
          if(active)check(entries.some(e=>e.label==='取消上传'&&!e.disabled)&&entries.some(e=>e.label==='上传云端'&&e.disabled),'queued/running transfer allows duplicate upload');
          if(['canceled','completed'].includes(state))check(await target.getByRole('img',{name:'已备份',exact:true}).count()===1,'terminal history hides completed receipt');
          if(view==='list')signatures.set('backup-'+state,entries);
          else check(JSON.stringify(signatures.get('backup-'+state))===JSON.stringify(entries),'backup '+state+' menus differ');
          await p.mouse.move(0,0);await target.locator('.backup-sync-tooltip').hover();
          const tooltip=p.getByRole('tooltip');await tooltip.waitFor();
          const text=await tooltip.innerText();if(state==='pending')check(text==='待上传，点击取消上传','persisted pending tooltip incorrect');
          if(active)check(text.includes('点击取消上传'),'transfer tooltip lacks cancel action');
          const box=await tooltip.boundingBox();check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'tooltip overflow');
          await p.keyboard.press('Escape');
        }
        const remote=p.locator('[data-asset-id="remote"]');
        await remote.getByRole('button',{name:'取消下载',exact:true}).waitFor();
        const remoteEntries=await menuSnapshot(remote,true);
        check(remoteEntries.some(e=>e.label==='取消下载'&&!e.disabled)&&remoteEntries.some(e=>e.label==='从远端下载'&&e.disabled),'pending restore missing cancel or allows duplicate restore');
        if(view==='list')signatures.set('remote',remoteEntries);else check(JSON.stringify(signatures.get('remote'))===JSON.stringify(remoteEntries),'remote menus differ');
        for(const [name,hint] of view==='list' ? [['播放视频','在应用内播放'],['查看文案','查看文案']] : [['播放视频','在应用内播放']]){
          await p.mouse.move(0,0);await remote.getByRole('button',{name,exact:true}).hover();await p.getByRole('tooltip',{name:hint,exact:true}).waitFor();await p.keyboard.press('Escape');
        }
        const quality=row('completed').getByLabel('清晰度：1080P',{exact:true});await quality.scrollIntoViewIfNeeded();await p.mouse.move(0,0);await quality.focus();await p.getByRole('tooltip',{name:'清晰度：1920 × 1080',exact:true}).waitFor();await p.keyboard.press('Escape');
        const context=row('downloading').locator('..');await context.focus();await p.keyboard.press('Shift+F10');await menu().getByRole('menuitem',{name:'取消下载',exact:true}).waitFor();await p.keyboard.press('Escape');
      }
      // Exercise queued cancel actions by their actual operation/asset identifiers.
      await row('backup-pending').locator('.task-row-title').click({button:'right'});await menu().getByRole('menuitem',{name:'取消上传',exact:true}).click();
      await p.waitForFunction(()=>window.calls.some(c=>c.command==='cancel_backup_operation'&&c.args.operationId==='op-pending'));
      await p.locator('[data-asset-id="remote"]').click({button:'right'});await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem',{name:'取消下载',exact:true}).click();
      await p.waitForFunction(()=>window.calls.some(c=>c.command==='cancel_asset_restore'&&c.args.assetId==='remote'));
      check(errors.length===0,errors.join('\n'));checks.push({width,downloadStates:14,backupStates:7,layouts:2,passed:true});
    } catch(error) {throw new Error(error.message+" details="+JSON.stringify(await p.evaluate(()=>({view:document.querySelector('.task-list')?.dataset.view,active:document.activeElement?.outerHTML.slice(0,800),hints:[...document.querySelectorAll('[role=tooltip]')].map(n=>n.textContent)}))));} finally {await p.close();}
  }
  return {passed:true,checks};
}
