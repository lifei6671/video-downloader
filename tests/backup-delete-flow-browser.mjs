// Production renderer, synthetic IPC only. Never deletes real files or Telegram messages.
async page => {
  const results=[];
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    for (const view of ['list','feed']) for (const taskBacked of [true,false]) {
      const p=await page.context().newPage();p.setDefaultTimeout(7000);
      const errors=[];p.on('pageerror',error=>errors.push(error.message));
      const check=(ok,message)=>{if(!ok)throw Error(`${width}/${view}/${taskBacked}: ${message}`);};
      await p.addInitScript(({view,taskBacked})=>{
        localStorage.setItem('yingqu.task-view',view);
        const metadata={platform:'telegram',resolver:'grammers',kind:'video',video_id:'100_9',canonical_url:'https://t.me/c/100/9',title:'删除流程验证',author:'测试作者'};
        window.task={id:'task',asset_id:'asset',revision:1,status:'completed',video:metadata,path:'C:/fixture/video.mp4',bytes:100,total_bytes:100,created_at:1,completed_at:2,canceling:false,file_missing:false,download_requested:true,cover_cached:true,files:[]};
        window.asset={asset_id:'asset',task_id:taskBacked?'task':null,metadata,original_filename:'video.mp4',content_bytes:100,content_sha256:'a'.repeat(64),asset_state:'active',path:'C:/fixture/video.mp4',local_state:'present',space_id:'space',remote_state:'backed_up',has_cover:false,has_transcript:true,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0};
        window.operation=null;window.calls=[];window.taskRemoved=false;window.assetHidden=false;window.pauseDelete=true;
        const canvas=document.createElement('canvas');canvas.width=16;canvas.height=16;canvas.getContext('2d').fillRect(0,0,16,16);const cover=Array.from(atob(canvas.toDataURL('image/png').split(',')[1]),c=>c.charCodeAt(0));
        const callbacks=new Map(),listeners=new Map();let next=0;
        window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
        window.activity=null;
        window.emitActivity=value=>{
          window.activity=value;
          listeners.get('backup_delete_activity')?.({event:'backup_delete_activity',id:2,payload:value});
        };
        window.emitRemoved=()=>listeners.get('task_removed')?.({event:'task_removed',id:1,payload:'task'});
        window.emitBackupDeleted=()=>listeners.get('backup_asset_deleted')?.({event:'backup_asset_deleted',id:3,payload:'delete-operation'});
        window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:callback=>{callbacks.set(++next,callback);return next;},unregisterCallback(){},invoke:async(command,args)=>{
          window.calls.push({command,args});
          if(command==='plugin:event|listen'){listeners.set(args.event,callbacks.get(args.handler));return next;}
          if(command==='get_task_cover')return cover;
          if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
          if(command==='list_tasks')return taskBacked&&!window.taskRemoved?[structuredClone(window.task)]:[];
          if(command==='get_backup_settings')return {space_id:'space',library_id:'library',display_name:'测试备份',provider_type:'telegram',profile:'telegram',enabled:true,credentials_configured:false,can_delete_owned_media:true,last_error:null};
          if(command==='list_backup_assets')return window.assetHidden?[]:[structuredClone(window.asset)];
          if(command==='get_backup_delete_activity')return window.activity;
          if(command==='list_backup_operations')return window.operation?[structuredClone(window.operation)]:[];
          if(command==='get_backup_sync_enabled')return true;
          if(command==='list_video_transcripts')return [];
          if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
          if(command==='delete_asset_and_backup'){
            if(window.pauseDelete){await new Promise(resolve=>window.releaseDelete=resolve);window.pauseDelete=false;}
            window.operation??={operation_id:'delete-operation',space_id:'space',asset_id:'asset',kind:'delete',state:'pending',phase:'tombstone',payload:{},attempt:0,last_error:null};
            window.operation.state='pending';window.operation.last_error=null;window.asset.remote_state='pending_delete';return window.operation.operation_id;
          }
          return null;
        }};
      },{view,taskBacked});
      try {
        await p.setViewportSize({width,height});await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
        await p.getByRole('button',{name:'下载任务',exact:true}).click();
        const row=p.locator(taskBacked?'[data-task-id="task"]':'[data-asset-id="asset"]');await row.waitFor();if(taskBacked)await row.locator('img').waitFor();
        const original=await row.elementHandle();
        const sameRow=async()=>{
          check(await row.evaluate((node,original)=>node===original,original),'the original card was replaced');
          check(await p.locator('.task-list > [role=listitem]').count()===1,'deletion creates a second task/asset row');
          if(taskBacked)check(await p.locator('[data-asset-id="asset"]').count()===0,'task is duplicated as a remote asset');
        };
        const headingIcon=row.locator('.task-title-backup .backup-sync-tooltip');
        const listLayout=()=>row.evaluate(node=>[node,node.querySelector('.task-row-heading'),node.querySelector('.task-row-byline')].map(el=>{
          const {x,y,width,height}=el.getBoundingClientRect();return [x,y,width,height];
        }));
        const layoutBefore=view==='list'?JSON.stringify(await listLayout()):null;
        const assertStage=async label=>{
          if(view==='list') {
            if(label.includes('失败'))await row.getByText(label,{exact:true}).waitFor();
            else {
              await headingIcon.getByRole('button',{name:label,exact:true}).waitFor();
              check(await row.locator('.task-row-information > [role=status]').count()===0,'list displays duplicate deletion status');
              check(JSON.stringify(await listLayout())===layoutBefore,'list layout shifted during '+label);
            }
          }
          else {
            const progress=row.locator('.task-row-cover').getByRole('progressbar',{name:'删除进度',exact:true});
            await progress.waitFor();await p.waitForFunction(({label,taskBacked})=>document.querySelector((taskBacked?'[data-task-id="task"]':'[data-asset-id="asset"]')+' .task-row-cover [role=progressbar]')?.getAttribute('aria-valuetext')===label,{label,taskBacked});
            check(await progress.getAttribute('aria-valuenow')===null,'deletion has a made-up percentage');
            const metrics=await progress.evaluate(el=>{const probe=document.createElement('span');probe.style.color='var(--destructive)';el.append(probe);const red=getComputedStyle(probe).color;probe.remove();const ring=el.getBoundingClientRect(),cover=el.closest('.task-row-cover').getBoundingClientRect();return {color:getComputedStyle(el).color,red,ring:{x:ring.x,y:ring.y,width:ring.width,height:ring.height},cover:{x:cover.x,y:cover.y,width:cover.width,height:cover.height}};});
            check(metrics.color===metrics.red,'deletion ring does not use destructive red');
            const background=await progress.evaluate(el=>getComputedStyle(el).backgroundColor);
            const alpha=background.includes('/')?parseFloat(background.split('/').pop()):background.startsWith('rgba')?parseFloat(background.split(',').pop()):1;
            check(Math.abs(alpha-0.5)<0.01,'deletion center is not half translucent: '+background);
            check(metrics.ring.width===64&&metrics.ring.height===64&&Math.abs(metrics.ring.x+32-metrics.cover.x-metrics.cover.width/2)<1&&Math.abs(metrics.ring.y+32-metrics.cover.y-metrics.cover.height/2)<1,'deletion ring is not centered');
            if(!label.includes('失败'))check(await row.locator('.task-row-information > [role=status]').count()===0,'deletion retained its bottom message');
          }
        };
        await row.click({button:'right'});
        await p.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).click();
        await assertStage('正在提交删除');await sameRow();
        await p.evaluate(()=>window.releaseDelete());
        await assertStage('等待删除远端备份');await sameRow();
        await row.click({button:'right'});
        const activeMenu=p.getByRole('menu');
        if(taskBacked)for(const name of ['保存封面','复制封面'])check(await activeMenu.getByRole('menuitem',{name,exact:true}).isDisabled(),'cover export remains available during deletion');
        await p.keyboard.press('Escape');
        if(!taskBacked&&view==='list')check(await row.getByRole('button',{name:'查看文案',exact:true}).isDisabled(),'transcript remains clickable during deletion');
        const stage=async(phase,state='running',error=null)=>{
          await p.evaluate(({phase,state,error})=>{window.operation.phase=phase;window.operation.state=state;window.operation.last_error=error;},{phase,state,error});
        };
        await stage('objects');await assertStage('正在删除远端备份');await sameRow();
        await p.evaluate(()=>window.emitActivity({operation_id:'delete-operation',companion:null,step:'archive_identity_check'}));
        const statusLine=row.locator('.task-card-feedback .task-card-message');
        await statusLine.filter({hasText:'正在按 ID 核对归档附件'}).waitFor();
        await p.evaluate(()=>window.emitActivity({operation_id:'unrelated-operation',companion:null,step:'delete_message'}));
        await statusLine.filter({hasText:'正在清理远端备份'}).waitFor();
        await p.evaluate(()=>window.emitActivity({operation_id:'delete-operation',companion:null,step:'archive_identity_check'}));
        await statusLine.filter({hasText:'正在按 ID 核对归档附件'}).waitFor();
        check(await p.getByRole('tooltip').count()===0,'progress requires opening an icon tooltip');
        const icon=view==='feed'?row.locator('.backup-delete-progress'):headingIcon;
        if(taskBacked)await icon.hover();else await icon.focus();
        const tip=p.getByRole('tooltip');
        await tip.filter({hasText:'正在按 ID 核对归档附件'}).waitFor();
        await p.evaluate(()=>window.emitActivity({operation_id:'peer',companion:'delete-operation',step:'confirm_message_gone'}));
        await tip.filter({hasText:'正在确认同批视频消息均已删除'}).waitFor();
        await statusLine.filter({hasText:'正在确认同批视频消息均已删除'}).waitFor();
        const bounds=await p.locator('[data-slot=tooltip-content]').boundingBox();
        check(bounds && bounds.x>=0 && bounds.y>=0 && bounds.x+bounds.width<=width && bounds.y+bounds.height<=height,'delete tooltip outside viewport');
        await p.evaluate(()=>{window.emitActivity(null);window.operation.phase='archive_ready';window.operation.state='pending';window.operation.payload.archive_next='peer';});
        await tip.filter({hasText:'归档已确认，等待同批清理完成'}).waitFor();
        await statusLine.filter({hasText:'归档已确认，等待同批清理完成'}).waitFor();
        await p.keyboard.press('Escape');await p.mouse.move(0,0);
        await stage('cleanup_verify','blocked','BACKUP_ACCESS_DENIED');await assertStage('远端备份删除失败');await row.getByRole('alert').waitFor();await sameRow();
        await row.locator('.backup-sync-button[aria-label="重试删除"]').click();
        await assertStage('等待删除远端备份');
        await stage('local_cleanup');await p.evaluate(()=>window.asset.remote_deleted=true);
        await assertStage('正在删除本地文件');await sameRow();
        await stage('local_cleanup','blocked','IO_ERROR');await assertStage('本地文件删除失败');await sameRow();
        const context=taskBacked?row.locator('..'):row;await context.focus();await p.keyboard.press('Shift+F10');
        const menu=p.getByRole('menu');await menu.getByRole('menuitem',{name:'重试删除',exact:true}).click();
        await assertStage('正在删除本地文件');
        await stage('finalizing');await p.evaluate(()=>{window.asset.path=null;window.asset.local_state='missing';});
        await assertStage('正在移除记录');await sameRow();
        await row.screenshot({path:`.local/backup-retry-download-stability/delete-${width}-${view}-${taskBacked?'task':'asset'}.png`});
        const keepOutsideFocus=!taskBacked&&view==='feed';
        if(keepOutsideFocus)await p.getByRole('button',{name:'下载任务',exact:true}).focus();else await context.focus();
        // Simulate the authoritative remove event before a stale asset snapshot refreshes.
        await p.evaluate(taskBacked=>{
          window.operation.payload.local_cleanup_done=true;window.operation.phase='cleanup_verify';window.operation.state='retry_wait';
          if(taskBacked){window.taskRemoved=true;window.emitRemoved();}
        },taskBacked);
        if(!taskBacked) {
          await row.getByText('删除完成',{exact:true}).first().waitFor();await sameRow();
          check(await row.getByText('远端待删除',{exact:true}).count()===0,'mixed task/asset polls revert a completed deletion to pending');
          await p.evaluate(()=>{
            window.assetHidden=true;
            const before=window.calls.filter(call=>call.command==='list_backup_assets').length;
            window.emitBackupDeleted();
            if(window.calls.filter(call=>call.command==='list_backup_assets').length===before)throw Error('post-commit event did not refresh visible assets');
          });
        }
        await p.locator('.task-list > [role=listitem]').waitFor({state:'hidden'});
        check(await p.locator('[data-asset-id="asset"]').count()===0,'a stale backup poll recreates the deleted task');
        if(keepOutsideFocus)check(await p.getByRole('button',{name:'下载任务',exact:true}).evaluate(button=>button===document.activeElement),'successful deletion steals focus that moved outside the row');
        else await p.waitForFunction(()=>document.activeElement?.getAttribute('role')==='tabpanel');
        await p.evaluate(()=>window.assetHidden=true);
        const calls=await p.evaluate(()=>window.calls.filter(call=>call.command==='delete_asset_and_backup'));
        check(calls.length===3&&calls.every(call=>call.args.spaceId==='space'&&call.args.assetId==='asset'),'delete/retry targets or operation count changed');
        check(await p.evaluate(()=>window.calls.every(call=>call.command!=='delete_local_video'&&call.command!=='remove_task')),'the UI removes local history before remote confirmation');
        check(errors.length===0,errors.join('\n'));results.push({width,view,taskBacked,passed:true});
      } catch(error) {throw Error(`${width}/${view}/${taskBacked}: ${error.message} `+JSON.stringify(await p.evaluate(()=>({active:document.activeElement?.outerHTML.slice(0,700),rows:document.querySelectorAll('.task-list > [role=listitem]').length,panels:[...document.querySelectorAll('[role=tabpanel]')].map(panel=>({tabindex:panel.getAttribute('tabindex'),hidden:panel.hidden}))}))));} finally {await p.close();}
    }
  }
  return {passed:true,results};
}
