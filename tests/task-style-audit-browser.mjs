// Production renderer with generated image fixtures and synthetic IPC. Never touches real tasks, media or accounts.
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
      const wanted=new Set(['completed','failed','downloading','queued','backup-pending','backup-running','backup-blocked','backup-retry_wait','backup-canceled','backup-completed']);
      window.tasks=window.tasks.filter(task=>wanted.has(task.id));
      window.assets=window.assets.filter(asset=>asset.asset_id==='remote'||wanted.has(asset.task_id));
      window.assets.push({...window.assets.find(asset=>asset.asset_id==='remote'),asset_id:'remote2',local_state:'missing',metadata:{...video,title:'   ',author:'long-author-'.repeat(80)}});
      window.assets.find(asset=>asset.asset_id==='remote2').original_filename='无标题原视频.mov';
      window.tasks.push({...base,id:'cloud-history',asset_id:'cloud-history',file_missing:true,video:{...video,title:'保留历史的云端视频'}});
      window.assets.push({...window.assets.find(asset=>asset.asset_id==='remote'),asset_id:'cloud-history',task_id:'cloud-history',local_state:'missing'});
      window.ops.push({...window.ops.find(operation=>operation.operation_id==='restore-op'),operation_id:'history-restore',asset_id:'cloud-history'});
      window.tasks.find(task=>task.id==='completed').video.title='很长的原视频标题'.repeat(30);
      window.tasks.forEach(task=>task.cover_cached=!!task.video);
      window.assets.forEach(asset=>asset.has_cover=true);
      const canvas=document.createElement('canvas');canvas.width=160;canvas.height=90;
      const context=canvas.getContext('2d');context.fillStyle='#edf2fe';context.fillRect(0,0,160,90);context.fillStyle='#3e63dd';context.fillRect(20,20,80,50);
      window.mockCover=Uint8Array.from(atob(canvas.toDataURL('image/png').split(',')[1]),c=>c.charCodeAt(0));
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
        window.calls.push({command,args});
        if(command==='get_task_cover')return Array.from(window.mockCover);
        if(command==='get_asset_cover')return window.mockCover.buffer;
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return structuredClone(window.tasks);
        if(command==='get_backup_settings')return space;
        if(command==='list_backup_spaces')return [space];
        if(command==='get_backup_sync_enabled')return true;
        if(command==='list_backup_assets')return structuredClone(window.assets);
        if(command==='list_backup_operations')return structuredClone(window.ops);
        if(command==='list_video_transcripts')return [];
        if(command==='get_asset_transcript')return {text:'测试云端文案'};
        if(command==='prepare_asset_playback')throw 'FIXTURE_PLAYBACK_UNAVAILABLE';
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
      const remote=p.locator('[data-asset-id="remote"]'), other=p.locator('[data-asset-id="remote2"]');
      const filters=p.getByRole('radiogroup',{name:'按状态筛选下载任务'});
      const filter=label=>filters.getByRole('radio').filter({hasText:label}).click();
      const coverMetrics=async target=>{
        await target.scrollIntoViewIfNeeded();const image=target.locator('.task-row-cover img');await image.waitFor();await image.evaluate(img=>img.decode());
        return target.evaluate(el=>{
          const rect=node=>{if(!node)return null;const b=node.getBoundingClientRect();return {width:b.width,height:b.height,right:b.right,bottom:b.bottom};};
          const cover=el.querySelector('.media-thumbnail'),controls=el.querySelector('.task-row-controls'),container=el.closest('[role="listitem"]');
          return {row:rect(el),cover:rect(cover),radius:getComputedStyle(cover).borderRadius,fit:getComputedStyle(cover.querySelector('img')).objectFit,
            play:rect(el.querySelector('button[aria-label="播放视频"]')),controls:controls?rect(controls):null,gap:controls?getComputedStyle(controls).gap:null,wrapperPadding:parseFloat(getComputedStyle(container).paddingBottom),
            frame:getComputedStyle(el).borderWidth,shadow:getComputedStyle(el).boxShadow};
        });
      };
      const menuAudit=async target=>{
        await target.click({button:'right',force:true});const menu=p.getByRole('menu',{name:'远端视频操作',exact:true});await menu.waitFor();
        const items=await menu.getByRole('menuitem').evaluateAll(nodes=>nodes.map(node=>{
          const text=[...node.childNodes].find(child=>child.nodeType===Node.TEXT_NODE&&child.textContent.trim());
          const range=document.createRange();range.selectNodeContents(text);
          return {label:node.textContent.trim(),icon:node.querySelector('svg')?.getAttribute('class'),left:range.getBoundingClientRect().left,disabled:node.getAttribute('aria-disabled')==='true'};
        }));
        check(items.every(item=>item.icon),'remote menu items have missing icons');
        check(items.every(item=>Math.abs(item.left-items[0].left)<1),'remote menu labels are misaligned');
        check(items.findIndex(item=>item.label==='从列表移除')<items.findIndex(item=>item.label==='从当前同步库删除'),'remote menu removal order differs from local task menu');
        check(items[0].label==='查看文案','remote transcript is missing from the shared menu');
        check(await menu.getByRole('separator').count()===2,'remote transcript/transfer/removal groups are not separated');
        const box=await menu.boundingBox();check(box.x>=0&&box.y>=0&&box.x+box.width<=width&&box.y+box.height<=height,'remote menu overflows');
        await menu.screenshot({path:'.local/cloud-only-badge/menu-'+width+'-'+(await p.locator('.task-list').getAttribute('data-view'))+'.png'});
        await p.keyboard.press('Escape');await menu.waitFor({state:'hidden'});return items.map(({label,icon,disabled})=>({label,icon,disabled}));
      };
      const hints=async(name,target)=>{
        await p.mouse.move(0,0);await target.hover();const tooltip=p.getByRole('tooltip',{name,exact:true});await tooltip.waitFor();
        const b=await tooltip.boundingBox();check(b.x>=0&&b.y>=0&&b.x+b.width<=width&&b.y+b.height<=height,'tooltip overflows');await p.keyboard.press('Escape');
      };
      const setRestore=async(state,bytes,error=null)=>{
        await filter('全部');
        await p.evaluate(({state,bytes,error})=>{
          for(const id of ['remote','cloud-history']) {
            const asset=window.assets.find(asset=>asset.asset_id===id),operation=window.ops.find(op=>op.asset_id===id);
            operation.state=state;operation.payload=bytes==null?{}:{completed_bytes:bytes};operation.last_error=error;
            asset.path=state==='completed'?'C:/fixture/restored.mp4':null;asset.local_state=state==='completed'?'present':['pending','running'].includes(state)?'restoring':'missing';asset.remote_state='backed_up';asset.remote_deleted=false;asset.asset_state='active';
          }
        },{state,bytes,error});
      };
      const coverBadge=(target)=>target.locator('.task-row-cover [data-slot=badge]');
      const checkBadge=async(view,label,variant)=>{
        for(const target of [remote,row('cloud-history')]) {
          if(view==='list')check(await coverBadge(target).count()===0,'list view gained a cover badge');
          else {
            await coverBadge(target).filter({hasText:new RegExp('^'+label+'$')}).waitFor();
            check(await coverBadge(target).count()===1,'cloud card has duplicate badges');
            check(await coverBadge(target).getAttribute('data-variant')===variant,'cloud badge has incorrect status color');
            const position=await target.evaluate(el=>{const cover=el.querySelector('.task-row-cover').getBoundingClientRect(),badge=el.querySelector('.task-row-cover [data-slot=badge]').getBoundingClientRect();return {top:badge.top-cover.top,left:badge.left-cover.left};});
            check(Math.abs(position.top-8)<1&&Math.abs(position.left-8)<1,'cloud badge is not in the cover top-left');
          }
        }
      };
      const cases=[];
      let listMenu;
      for(const view of ['list','feed']) {
        if(view==='feed')await p.getByRole('button',{name:'切换为信息流',exact:true}).click();
        await setRestore('pending',0);await remote.getByRole('progressbar',{name:'待下载',exact:true}).waitFor();await checkBadge(view,'待下载','warning');
        const local=await coverMetrics(row('completed')),queued=await coverMetrics(row('queued')),cloud=await coverMetrics(remote),long=await coverMetrics(other);
        for(const metrics of [local,queued,cloud,long]) {
          check(metrics.frame==='0px'&&metrics.shadow==='none','task row gained a default card frame');
          check(metrics.radius===local.radius&&metrics.fit===local.fit,'remote/local cover appearance differs');
          if(view==='list')check(Math.abs(metrics.cover.width-56)<1&&Math.abs(metrics.cover.height-56)<1,'list thumbnail is stretched or incorrectly sized');
          else check(Math.abs(metrics.cover.width/metrics.cover.height-0.75)<0.01,'feed cover aspect ratio differs');
        }
        if(view==='list') {
          check(local.wrapperPadding===4&&cloud.wrapperPadding===4&&long.wrapperPadding===0,'remote/local row gaps differ');
          check(long.row.height===local.row.height,'long remote author stretched the row');
          check(cloud.gap==='8px','remote list action icons are packed together');
          check(await remote.getByRole('button',{name:'查看文案',exact:true}).count()===1,'list transcript shortcut is missing');
        } else {
          check(await remote.locator('.task-row-footer').count()===0,'remote card has a redundant footer');
          check(await remote.getByRole('button',{name:'查看文案',exact:true}).count()===0,'remote card repeats the transcript button');
          check(await remote.getByRole('button',{name:'播放视频',exact:true}).count()===1,'remote card duplicates the play button');
          check(await remote.locator('.task-row-cover').getByRole('button',{name:'播放视频',exact:true}).count()===1,'remote play action is outside the cover');
          check(Math.abs((cloud.cover.right-cloud.play.right)-(local.cover.right-local.play.right))<1&&Math.abs((cloud.cover.bottom-cloud.cover.height+8)-(cloud.play.bottom-cloud.play.height))<1,'remote/local play overlays differ');
        }
        check(await other.locator('.task-row-title').innerText()==='无标题原视频.mov','blank title did not fall back to filename');
        const author=other.locator('.task-row-author');
        check(await author.evaluate(node=>node.scrollWidth>node.clientWidth&&node.getBoundingClientRect().height<=20),'long remote author is not truncated');
        await hints('long-author-'.repeat(80),author);
        for(const state of ['blocked','retry_wait'])check((await row('backup-'+state).getByRole('alert').innerText()).length>0,'backup error is only visible in a tooltip');
        for(const state of ['pending','running','canceled','completed'])check(await row('backup-'+state).getByRole('alert').count()===0,'backup row shows a stale failure');
        await remote.scrollIntoViewIfNeeded();await p.mouse.move(0,0);
        await remote.screenshot({path:'.local/cloud-only-badge/after-'+view+'-'+width+'.png'});
        const items=await menuAudit(remote);if(view==='list')listMenu=items;else check(JSON.stringify(listMenu)===JSON.stringify(items),'remote list/feed menus differ');
        check(await remote.getByRole('progressbar',{name:'待下载',exact:true}).getAttribute('aria-valuenow')===null,'queued restore invents numeric progress');
        check((await remote.locator('.task-row-byline').innerText()).includes('待下载'),'queued restore row says downloading');
        await hints('待下载，点击取消下载',remote.locator('.backup-sync-tooltip'));
        await filter('等待中');await remote.waitFor();await filter('下载中');check(await remote.count()===0,'queued restore is classified as downloading');await filter('全部');
        await setRestore('running',null);await remote.getByRole('progressbar',{name:'正在下载',exact:true}).waitFor();await checkBadge(view,'下载中','info');
        check(await remote.getByRole('progressbar',{name:'正在下载',exact:true}).getAttribute('aria-valuenow')===null,'unknown restore invents zero percent');
        await filter('下载中');await remote.waitFor();await filter('等待中');check(await remote.count()===0,'running restore is classified as queued');await filter('全部');
        await setRestore('running',50);await remote.getByRole('progressbar',{name:'正在下载 50%',exact:true}).waitFor();
        await setRestore('running',100);await remote.getByRole('progressbar',{name:'正在下载 99%',exact:true}).waitFor();
        await setRestore('blocked',0,'BACKUP_SERVICE_UNAVAILABLE');await remote.getByRole('alert').waitFor();await checkBadge(view,'下载失败','destructive');
        check((await remote.locator('.task-row-byline').innerText()).includes('下载失败'),'failed restore has no persistent state');
        await filter('失败');await remote.waitFor();await filter('下载中');check(await remote.count()===0,'failed restore remains downloading');await filter('全部');
        await setRestore('completed',100);await remote.getByRole('img',{name:'已备份',exact:true}).waitFor();await checkBadge(view,'已完成','success');check(await remote.getByRole('alert').count()===0,'completed restore retains failure text');
        await filter('已完成');await remote.waitFor();await filter('失败');check(await remote.count()===0,'completed restore remains failed');await filter('全部');
        await setRestore('canceled',null);await remote.getByRole('img',{name:'待下载',exact:true}).waitFor();check(await remote.getByRole('progressbar').count()===0,'idle cloud video is displayed as an active transfer');
        await checkBadge(view,'待同步','warning');
        if(view==='feed') {
          await coverBadge(remote).evaluate(async badge=>{await Promise.all(badge.getAnimations().map(animation=>animation.finished.catch(()=>{})));});
          const colors=await coverBadge(remote).evaluate(badge=>({background:getComputedStyle(badge).backgroundColor,color:getComputedStyle(badge).color}));
          const waitingColors=await row('queued').locator('[data-slot=badge]').evaluate(badge=>({background:getComputedStyle(badge).backgroundColor,color:getComputedStyle(badge).color}));
          check(JSON.stringify(colors)===JSON.stringify(waitingColors),'pending sync badge does not use the existing pale yellow warning colors');
          await remote.screenshot({path:'.local/cloud-only-badge/pending-sync-'+width+'.png'});
          await row('cloud-history').screenshot({path:'.local/cloud-only-badge/history-pending-sync-'+width+'.png'});
          for(const state of ['pending_delete','deleted']) {
            await p.evaluate(state=>{for(const id of ['remote','cloud-history']){const asset=window.assets.find(a=>a.asset_id===id);asset.remote_state=state==='pending_delete'?'pending_delete':'backed_up';asset.remote_deleted=state==='deleted';}},state);
            for(const target of [remote,row('cloud-history')])await coverBadge(target).waitFor({state:'hidden'});
          }
          await setRestore('canceled',null);await checkBadge(view,'待同步','warning');
        }
        // The moved actions keep their asset identity and restore keyboard focus.
        await remote.focus();await p.keyboard.press('Shift+F10');
        await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem',{name:'查看文案',exact:true}).click();
        const transcript=p.getByRole('dialog',{name:'视频文案',exact:true});await transcript.getByText('测试云端文案',{exact:true}).waitFor();
        check(await p.evaluate(()=>window.calls.some(c=>c.command==='get_asset_transcript'&&c.args.assetId==='remote'&&c.args.spaceId==='space')),'menu opens the wrong transcript');
        await p.keyboard.press('Escape');await transcript.waitFor({state:'hidden'});await p.waitForFunction(()=>document.activeElement?.getAttribute('data-asset-id')==='remote');
        await remote.getByRole('button',{name:'播放视频',exact:true}).click();
        const player=p.getByRole('dialog',{name:'远端视频',exact:true});await player.waitFor();
        await p.waitForFunction(()=>window.calls.some(c=>c.command==='prepare_asset_playback'&&c.args.assetId==='remote'));
        await p.keyboard.press('Escape');await player.waitFor({state:'hidden'});
        await p.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='播放视频');
        cases.push({view,passed:true});
      }
      check(errors.length===0,errors.join('\n'));checks.push({width,height,cases});
    } catch(error) {throw new Error(error.message+" details="+JSON.stringify(await p.evaluate(()=>({view:document.querySelector('.task-list')?.dataset.view,active:document.activeElement?.outerHTML.slice(0,800),hints:[...document.querySelectorAll('[role=tooltip]')].map(n=>n.textContent)}))));} finally {await p.close();}
  }
  return {passed:true,checks};
}
