// Production renderer and synthetic IPC; deletion touches no real data.
async page => {
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) for(const view of ['list','feed']) for(const delayed of [false,true]) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);
    await p.setViewportSize({width,height});
    const errors=[];p.on('pageerror',error=>errors.push(error.message));
    await p.addInitScript(delayed => {
    window.taskBacked = true;
    localStorage.setItem('yingqu.task-view', 'feed');
    window.current = 'r2'; window.calls = []; window.uploaded = true;
    const base = { region:'auto', path_style:true, credentials_configured:false, connection_pending:false, last_error:null, can_delete_owned_media:true };
    window.spaces = [
      { ...base, space_id:'r2', library_id:'r2-library', provider_type:'s3', profile:'r2', display_name:'视频归档', endpoint:'https://fixture.r2.cloudflarestorage.com', bucket:'bucket', prefix:'yingqu/', enabled:true },
      { ...base, space_id:'tg', library_id:'tg-library', provider_type:'telegram', profile:'telegram', display_name:'Telegram 备份', endpoint:'', bucket:'', prefix:'', telegram:{account_id:'123',channel_id:'777'}, telegram_channel_title:'知识库', enabled:false },
    ];
    const metadata = { title:'测试视频标题', author:'测试作者', platform:'youtube', video_id:'12345678901', canonical_url:'https://www.youtube.com/watch?v=12345678901', kind:'video', resolver:'yt-dlp', duration_ms:2000, width:1920, height:1080 };
    window.task = { id:'local-task', asset_id:'asset', revision:1, status:'completed', video:metadata, bytes:100, total_bytes:100, speed_bps:null, error:null, path:'C:/fixture/video.mp4', created_at:1, completed_at:2, canceling:false, file_missing:false, download_requested:true, cover_cached:false, files:[] };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    const callbacks=new Map(),listeners=new Map();let next=0;
    window.deliverRemoval=()=>{
      window.taskBacked=false;window.task.path=null;
      listeners.get('task_removed')?.({payload:{id:'local-task',space_id:window.deletedSpace}});
    };
    window.removeTaskFromCurrentSpace=()=>{
      window.deletedSpace=window.current;
      if(!delayed)window.deliverRemoval();
    };
    window.__TAURI_INTERNALS__ = { convertFileSrc:x=>x, transformCallback:callback=>{callbacks.set(++next,callback);return next;}, unregisterCallback() {}, invoke:async (command,args) => {
      window.calls.push({command,args});
      if(command==='plugin:event|listen'){listeners.set(args.event,callbacks.get(args.handler));return next;}
      if(command==='get_settings') return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
      if(command==='list_tasks') return window.taskBacked ? [structuredClone(window.task)] : [];
      if(command==='get_backup_settings') return structuredClone(window.spaces.find(s=>s.space_id===window.current));
      if(command==='list_backup_spaces') return structuredClone(window.spaces);
      if(command==='get_backup_sync_enabled') return true;
      if(command==='list_backup_assets') return args.spaceId===window.deletedSpace ? [] : [{asset_id:'asset',task_id:window.taskBacked ? 'local-task' : null,metadata,original_filename:'video.mp4',content_bytes:100,content_sha256:'a'.repeat(64),asset_state:'active',path:window.task.path,local_state:window.task.path?'present':'missing',space_id:args.spaceId,remote_state:args.spaceId==='r2'||window.uploaded?'backed_up':'not_backed_up',in_current_space:args.spaceId==='r2'||window.uploaded,has_cover:false,has_transcript:false,remote_revision:0,remote_deleted:false,has_pending_local_changes:false,conflict_count:0,backup_locations:[{space_id:'r2',profile:'r2',display_name:'视频归档',channel_title:null},...(window.uploaded?[{space_id:'tg',profile:'telegram',display_name:'Telegram 备份',channel_title:'知识库'}]:[])]}];
      if(command==='select_backup_space') {window.current=args.spaceId;window.spaces.forEach(s=>s.enabled=s.space_id===window.current);return structuredClone(window.spaces.find(s=>s.space_id===window.current));}
      if(command==='delete_asset_and_backup') { window.removeTaskFromCurrentSpace(); return 'delete-operation'; }
      if(command==='start_asset_backup') {window.uploaded=true;return;}
      if(command==='telegram_storage_status') return {configured:true,operation_id:'login',phase:'ready',account:{id:'123',name:'测试账号'},last_error:null};
      if(command==='list_backup_operations') return window.failedUpload ? [{operation_id:'failed-upload',asset_id:'asset',space_id:'tg',kind:'upload',state:'blocked',phase:'prepared',attempt:1,last_error:'TELEGRAM_CHANNEL_FORBIDDEN',payload:{}}] : [];
      if(['list_video_transcripts','list_backup_operations'].includes(command)) return [];
      if(command==='check_runtime_dependencies') return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};

    },delayed);
    await p.addInitScript(view=>localStorage.setItem('yingqu.task-view',view),view);
    const button=name=>p.getByRole('button',{name,exact:true});
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');await button('下载任务').click();
      const original=p.locator('[data-task-row="local-task"]');await original.waitFor();
      await original.click({button:'right'});
      await p.getByRole('menuitem',{name:'从当前同步库删除',exact:true}).click();
      if(!delayed){
        await original.waitFor({state:'detached'});
        if(await p.locator('[data-asset-id="asset"]').count())throw Error('Deleted membership reappeared from a stale asset snapshot');
      }
      await button('系统设置').click();await button('远端备份').click();
      await p.getByRole('tab',{name:'Telegram 备份',exact:true}).click();
      await button('设为默认').focus();await p.keyboard.press('Enter');
      await p.waitForFunction(()=>window.current==='tg');await button('下载任务').click();
      if(delayed){await p.evaluate(()=>window.deliverRemoval());await original.waitFor({state:'detached'});}
      const remaining=p.locator('[data-asset-id="asset"]');await remaining.waitFor();
      await remaining.click({button:'right'});
      if(!await p.getByRole('menuitem',{name:'从远端下载',exact:true}).isEnabled())throw Error('Other space lost its restore action');
      await p.keyboard.press('Escape');
      if(!await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth))throw Error('Horizontal overflow');
      if(errors.length)throw Error(errors.join('\n'));
      results.push({width,view,delayed,deletedSpaceHidden:true,otherSpaceRestorable:true});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
