// playwright-cli --session p5-spaces run-code --filename tests/multi-device-sync-browser.mjs
async (page) => {
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(()=>{
    localStorage.setItem('yingqu.task-view','list');
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
    const target={provider_type:'s3',profile:'r2',region:'auto',path_style:true,endpoint:'https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com',bucket:'space-a',prefix:'test/',enabled:true,credentials_configured:true,protocol:2,last_error:null};
    window.spaces=[{...target,space_id:'a',library_id:'lib-a'},{...target,space_id:'b',library_id:'lib-b',profile:'s3',region:'us-east-1',endpoint:'https://s3.example.com',bucket:'space-b',enabled:false},{...target,space_id:'old',library_id:'lib-old',bucket:'old-bucket',protocol:2,enabled:false}];
    window.current='a';window.calls=[];
    const asset=(id,extra={})=>({asset_id:id,task_id:null,metadata:{title:id,author:'测试作者',platform:'youtube'},original_filename:'video.mp4',content_sha256:'a'.repeat(64),content_bytes:100,asset_state:'active',path:null,local_state:'missing',space_id:'a',remote_state:'backed_up',has_cover:false,has_transcript:true,remote_revision:1,remote_deleted:false,has_pending_local_changes:false,conflict_count:0,...extra});
    window.lists={a:[asset('cloud-conflict',{conflict_count:1}),asset('pending-local',{remote_deleted:true,remote_state:'deleted',has_pending_local_changes:true}),asset('local-retained',{remote_deleted:true,remote_state:'deleted',path:'C:/fixture/keep.mp4',local_state:'present'}),asset('released-task',{task_id:'local-task'}),asset('upload',{path:'C:/fixture/video.mp4',local_state:'present',remote_state:'pending'})],b:[asset('other-library',{space_id:'b'})],old:[]};
    window.ops=[{operation_id:'upload-a',space_id:'a',asset_id:'upload',kind:'upload',state:'running',phase:'uploading',payload:{completed_bytes:42},attempt:0,last_error:null}];
    window.tasks=[{id:'local-task',asset_id:'released-task',revision:1,status:'completed',video:{title:'释放过的本机任务',platform:'youtube',video_id:'12345678901',kind:'video',canonical_url:'https://www.youtube.com/watch?v=12345678901'},bytes:100,total_bytes:100,path:null,created_at:1,completed_at:2,file_missing:true,canceling:false,files:[],download_requested:true}];
    window.__TAURI_INTERNALS__={convertFileSrc:path=>`http://asset.localhost/${encodeURIComponent(path)}`,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args)=>{
      window.calls.push({command,args});
      if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[],tdl:{enabled:false,path:null}};
      if(command==='list_tasks')return window.tasks;
      if(command==='list_video_transcripts')return [];
      if(command==='get_video_transcript')return null;
      if(command==='get_backup_sync_enabled')return true;
      if(command==='get_backup_settings')return window.spaces.find(s=>s.space_id===window.current);
      if(command==='list_backup_spaces')return window.spaces;
      if(command==='select_backup_space'){window.current=args.spaceId;window.spaces.forEach(s=>s.enabled=s.space_id===window.current);return window.spaces.find(s=>s.space_id===window.current);}
      if(command==='get_backup_credentials')return {access_key_id:`key-${args.spaceId}`,secret_access_key:`secret-${args.spaceId}`};
      if(command==='list_backup_assets')return window.lists[args.spaceId];
      if(command==='list_backup_operations')return window.ops;
      if(command==='refresh_backup_assets')return {imported:0,unchanged:0,rejected:[]};
      if(command==='test_backup_connection')throw 'BACKUP_CONDITIONAL_WRITE_UNSUPPORTED';
      if(command==='get_asset_transcript')return {asset_id:args.assetId,text:'云端文案，无需恢复视频',content_sha256:'a'.repeat(64),model_revision:'fixture',model_name:'fixture',pipeline_revision:'fixture',completed_at:1};
      if(command==='get_backup_local_data')return {title:args.assetId,transcript:args.assetId==='pending-local'?'尚未同步的本机文案':null,pending:args.assetId==='pending-local',remote_deleted:args.assetId==='pending-local',conflicts:args.assetId==='cloud-conflict'?[{id:'version-1',field:'title',value:'另一版本',device_id:'b-device',can_adopt:true,image:null}]:[]};
      if(command==='get_backup_conflict')return {id:'version-1',field:'title',value:'另一设备保留的版本',device_id:'b-device',can_adopt:true,image:null};
      if(command==='resolve_backup_conflict'){window.resolved=args;window.lists.a.find(a=>a.asset_id===args.assetId).conflict_count=0;return;}
      if(command==='export_backup_local_data'){window.exported=args;return 'C:/fixture/local.json';}
      if(command==='prepare_asset_playback'){window.played=args.assetId;throw 'FIXTURE_PLAYBACK';}
      if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};
  });
  await page.reload();
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.locator('[data-asset-id="cloud-conflict"]').getByRole('button',{name:'冲突 1'}).click();
  await page.getByRole('button',{name:'标题冲突版本'}).click();
  await page.getByRole('textbox',{name:'冲突版本内容'}).waitFor();
  if(await page.getByRole('textbox',{name:'冲突版本内容'}).inputValue()!=='另一设备保留的版本')throw new Error('conflict body missing');
  await page.getByRole('button',{name:'采用此版本'}).click();
  await page.waitForFunction(()=>window.resolved?.adopt===true);
  await page.getByRole('dialog').waitFor({state:'hidden'});
  const pending=page.locator('[data-asset-id="pending-local"]');
  if(await pending.getByRole('button',{name:'播放视频'}).isEnabled())throw new Error('tombstone restores video');
  await pending.getByRole('button',{name:'本机待处理',exact:true}).click();
  if(await page.getByRole('textbox',{name:'本机保留文案'}).inputValue()!=='尚未同步的本机文案')throw new Error('pending text lost');
  await page.getByRole('button',{name:'导出本机数据'}).click();await page.waitForFunction(()=>window.exported?.assetId==='pending-local');
  await page.keyboard.press('Escape');
  if(await page.evaluate(()=>window.calls.some(c=>['ensure_local_video','prepare_asset_playback'].includes(c.command))))throw new Error('metadata browsing fetched video');
  const task=page.locator('[data-task-id="local-task"]');
  await task.click({button:'right'});await page.getByRole('menuitem',{name:'查看文案',exact:true}).click();
  await page.getByRole('textbox',{name:'识别文案'}).waitFor();await page.keyboard.press('Escape');
  await task.getByRole('button',{name:'播放视频',exact:true}).click();await page.waitForFunction(()=>window.played==='released-task');await page.keyboard.press('Escape');
  await page.locator('[data-asset-id="local-retained"]').getByRole('button',{name:'播放视频',exact:true}).click();await page.waitForFunction(()=>window.played==='local-retained');await page.keyboard.press('Escape');
  const retained=page.locator('[data-asset-id="local-retained"]');
  await retained.click({button:'right'});
  const reupload=page.getByRole('menuitem',{name:'重新上传',exact:true});
  if(!await reupload.isEnabled())throw new Error('Explicit upload is permanently blocked by a deletion marker');
  await reupload.click();await page.waitForFunction(()=>window.calls.some(c=>c.command==='start_asset_backup'&&c.args.assetId==='local-retained'));
  await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'远端备份',exact:true}).click();
  for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});
    await page.getByRole('tab',{name:'S3 / space-b',exact:true}).click();
    await page.getByLabel('Region',{exact:true}).waitFor();
    if(await page.getByLabel('Access Key ID',{exact:true}).inputValue()!=='key-b')throw new Error('space credentials leaked');
    if(await page.evaluate(()=>window.current)!=='a')throw new Error('tab changed default space');
    await page.getByRole('button',{name:'设为默认',exact:true}).click();
    await page.waitForFunction(()=>window.current==='b');
    await page.getByRole('combobox',{name:'备份提供商'}).click();
    if(await page.getByRole('option',{name:'Telegram（计划支持）'}).getAttribute('aria-disabled')!=='true')throw new Error('Telegram enabled');
    await page.keyboard.press('Escape');
    if(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw new Error('settings overflow');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.getByText('other-library',{exact:true}).waitFor();
    if(await page.locator('[data-asset-id="upload"]').count())throw new Error('old-space upload displayed in new library');
    await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'远端备份',exact:true}).click();
    await page.getByRole('tab',{name:'R2 / space-a',exact:true}).click();
    await page.getByRole('button',{name:'设为默认',exact:true}).click();
    await page.waitForFunction(()=>window.current==='a');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.locator('[data-asset-id="upload"] [role="progressbar"]').waitFor();
    if(await page.locator('[data-asset-id="upload"] [role="progressbar"]').getAttribute('aria-valuenow')!=='42')throw new Error('back-switch lost original progress');
    await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'远端备份',exact:true}).click();
  }
  await page.getByRole('button',{name:'测试连接',exact:true}).click();
  await page.locator('[data-sonner-toast]').filter({hasText:'不支持原子条件写入'}).waitFor();
  if(await page.locator('[data-sonner-toast]').filter({hasText:'连接正常'}).count())throw new Error('unsupported service reported ready');
  await page.getByRole('tab',{name:'R2 / old-bucket',exact:true}).click();
  await page.getByRole('button',{name:'设为默认',exact:true}).click();
  await page.waitForFunction(()=>window.current==='old');
  if(await page.getByRole('alertdialog').count())throw new Error('Set default opened an upgrade dialog');
  await page.getByLabel('Bucket',{exact:true}).fill('new-target');
  if(await page.getByRole('button',{name:'设为默认',exact:true}).count())throw new Error('new draft exposes set default');
  if(await page.getByLabel('Access Key ID',{exact:true}).inputValue()||await page.getByLabel('Secret Access Key',{exact:true}).inputValue())throw new Error('changed target retained old credentials');
  await page.getByRole('combobox',{name:'备份提供商'}).focus();await page.keyboard.press('Space');await page.getByRole('option',{name:'通用 S3',exact:true}).click();await page.getByLabel('Region',{exact:true}).focus();
  await page.screenshot({path:'output/playwright/multi-device-settings-1024.png'});
  if(await page.evaluate(()=>window.calls.some(c=>c.command==='upgrade_backup_space')))throw new Error('implicit upgrade IPC');
  return {result:'PASS',viewports:3,spaceSwitches:6,conflictAdopted:true,pendingExported:true,lazyPlayback:true,telegramDisabled:true,wrongUpgradeTargetBlocked:true};
}
