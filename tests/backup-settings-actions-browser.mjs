// Production preview; isolated IPC fixtures never touch a real backup library.
async (page) => {
  const p = await page.context().newPage();
  const errors = []; p.on('pageerror', error => errors.push(error.message));
  await p.addInitScript(() => {
    const base = { provider_type:'s3',profile:'s3',region:'us-east-1',path_style:true,endpoint:'https://s3.example.test',prefix:'test/',credentials_configured:true,protocol:2,last_error:null };
    window.spaces = [{...base,space_id:'a',library_id:'lib-a',display_name:'主空间',bucket:'space-a',enabled:true},{...base,space_id:'b',library_id:'lib-b',display_name:'备用空间',bucket:'space-b',enabled:false},{...base,space_id:'empty',library_id:'lib-empty',display_name:'空配置',bucket:'space-empty',enabled:false}];
    window.current='a'; window.syncEnabled=true; window.calls=[]; window.failDelete=false; window.raceReference=false;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
    window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args)=>{
      window.calls.push({command,args});
      if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[],tdl:{enabled:false,path:null}};
      if(command==='get_backup_settings')return structuredClone(window.spaces.find(s=>s.space_id===window.current)??null);
      if(command==='list_backup_spaces'){
        if(window.failSpaceRefresh)throw 'DATABASE_ERROR';
        if(window.holdSpaceRefresh)await new Promise(resolve=>{window.finishSpaceRefresh=()=>{window.holdSpaceRefresh=false;resolve();};});
        return structuredClone(window.spaces);
      }
      if(command==='get_backup_credentials')return {access_key_id:'fixture-key',secret_access_key:'fixture-secret'};
      if(command==='get_backup_sync_enabled')return window.syncEnabled;
      if(command==='set_backup_sync_enabled'){window.syncEnabled=args.enabled;window.spaces.forEach(s=>s.enabled=s.space_id===window.current&&!s.connection_pending&&args.enabled);if(args.enabled){const current=window.spaces.find(s=>s.space_id===window.current);if(current)current.last_error=null;}return args.enabled;}
      if(command==='get_backup_space_usage')return {assets:args.spaceId==='empty'?0:2,operations:0};
      if(command==='delete_backup_space'){
        if(window.failDelete)throw 'DATABASE_ERROR';
        if(window.raceReference&&!args.confirmed)throw 'BACKUP_SPACE_REFERENCED';
        window.spaces=window.spaces.filter(s=>s.space_id!==args.spaceId);if(window.current===args.spaceId)window.current=null;return;
      }
      if(command==='select_backup_space'){if(!window.syncEnabled)throw 'BACKUP_SYNC_DISABLED';window.current=args.spaceId;window.spaces.forEach(s=>s.enabled=s.space_id===args.spaceId);return window.spaces.find(s=>s.enabled);}
      if(['list_tasks','list_video_transcripts','list_backup_assets','list_backup_operations'].includes(command))return [];
      if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};
  });
  const button = name=>p.getByRole('button',{name,exact:true});
  const tab = name=>p.getByRole('tab',{name,exact:true});
  const ready = ()=>p.locator('#backup-key:not(:disabled)').waitFor();
  const clickFeedback = async (name, heldText) => {
    await p.emulateMedia({reducedMotion:'no-preference'});
    await p.evaluate(()=>document.fonts.ready);
    if(heldText)await p.evaluate(()=>{window.holdSpaceRefresh=true;window.finishSpaceRefresh=null;});
    const control=button(name);await control.scrollIntoViewIfNeeded();
    const before=await control.boundingBox(),started=Date.now();
    await control.click();
    if(await control.getAttribute('aria-busy')!=='true'||!await control.isDisabled()||!await control.locator('[data-slot="spinner"]').count())throw new Error('Missing busy feedback: '+name);
    const during=await control.boundingBox();if(Math.abs(before.width-during.width)>1||Math.abs(before.height-during.height)>1)throw new Error('Feedback resized button: '+name+' '+JSON.stringify({before,during}));
    const visible=await control.locator('.backup-save-progress').evaluate(async el=>{
      let intermediate=false;const start=performance.now();
      while(performance.now()-start<180){await new Promise(requestAnimationFrame);const opacity=Number(getComputedStyle(el).opacity);if(opacity>0&&opacity<1)intermediate=true;}
      return intermediate;
    });
    if(!visible)throw new Error('No observed fade frames: '+name);
    if(heldText){
      await p.waitForFunction(()=>typeof window.finishSpaceRefresh==='function');
      if(await p.locator('.backup-feedback-button[aria-busy="true"] .backup-save-progress').textContent()!==heldText)throw new Error('Toggle feedback reversed during refresh');
      await p.evaluate(()=>window.finishSpaceRefresh());
    }
    await p.waitForFunction(label=>!document.querySelector(`button[aria-label="${label}"][aria-busy="true"]`),name);
    if(Date.now()-started<280)throw new Error('Feedback ended before visible duration: '+name);
  };
  try {
    await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
    await button('系统设置').click(); await button('远端备份').click(); await ready();
    await p.getByText('备份视频与文案，视频按需下载。',{exact:true}).waitFor();
    if(await p.locator('#backup-enabled').count())throw new Error('Old enable checkbox remains');
    if(await button('设为默认').count())throw new Error('Default space still has set-default action');
    await button('保存配置').waitFor();
    if(await button('新建配置').count())throw new Error('Saved configuration exposes create action');
    await clickFeedback('停用远端备份','正在停用…');await button('启用远端备份').waitFor();
    if(await button('设为默认').count())throw new Error('Disabled default still has set-default action');
    await tab('备用空间').click();await ready();
    if(!await button('设为默认').isDisabled())throw new Error('Disabled backup can set default');
    if(await p.evaluate(()=>window.current)!=='a')throw new Error('Toggle/browse changed default');
    await clickFeedback('启用远端备份','正在启用…');await ready();await clickFeedback('设为默认');await ready();
    if(await button('设为默认').count())throw new Error('New default action did not disappear');
    if(await p.evaluate(()=>window.current)!=='b')throw new Error('Default not set');
    if((await p.evaluate(()=>window.calls)).some(c=>['start_asset_backup','delete_asset_and_backup','save_backup_settings'].includes(c.command)))throw new Error('Setting default migrates assets or submits drafts');
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
      await p.setViewportSize({width,height});await tab('新建空间').click();await ready();
      if(await button('删除配置').count())throw new Error('New draft exposes delete configuration');
      await button('新建配置').waitFor();
      if(await button('保存配置').count())throw new Error('New draft exposes save label');
      if(await button('设为默认').count())throw new Error('New draft exposes set-default configuration');
      const header = p.locator('.backup-settings-actions > header');
      const titleBox = await header.getByRole('heading',{name:'远端备份',exact:true}).boundingBox();
      const toggleBox = await button('停用远端备份').boundingBox();
      const headerBox = await header.boundingBox();
      if(Math.abs(titleBox.y+titleBox.height/2-toggleBox.y-toggleBox.height/2)>1||Math.abs(toggleBox.x+toggleBox.width-headerBox.x-headerBox.width)>1)throw new Error('Enable toggle not aligned to title at right edge');
      if((await p.getByRole('combobox',{name:'备份提供商'}).textContent())!=='通用 S3'){
        await p.getByRole('combobox',{name:'备份提供商'}).click();await p.getByRole('option',{name:'通用 S3',exact:true}).click();
      }
      const form=p.getByRole('form',{name:'远端备份设置'});
      if(!await form.evaluate(e=>e.noValidate))throw new Error('Native validation bubble still enabled');
      await button('新建配置').click();
      const endpoint=p.locator('#backup-endpoint');
      const headerAfter = await header.boundingBox();
      if(Math.abs(headerAfter.y-headerBox.y)>1)throw new Error('Scrolling form moved title action');
      if(await endpoint.getAttribute('aria-invalid')!=='true'||!await endpoint.evaluate(e=>e===document.activeElement))throw new Error('First invalid field not focused');
      if(!await p.locator('#backup-endpoint-error').isVisible())throw new Error('Inline required message missing');
      for(const id of ['backup-endpoint','backup-bucket','backup-prefix','backup-region','backup-key','backup-secret']){
        if(!await p.locator(`label[for="${id}"] .text-destructive`).count())throw new Error('Required star missing: '+id);
      }
      await endpoint.fill('bad-url');await button('新建配置').click();
      if(await p.locator('#backup-endpoint-error').textContent()!=='请输入有效的地址')throw new Error('URL error missing');
      if(width===1440)await p.screenshot({path:'output/playwright/backup-settings-validation.png'});
      await p.emulateMedia({reducedMotion:'no-preference'});
      for(const control of [button('新建配置'),button('测试连接'),button('停用远端备份'),tab('主空间')]){
        if(!await control.evaluate(e=>getComputedStyle(e).transitionDuration.split(',').every(x=>x.trim()==='0.25s')))throw new Error('Button transition missing');
      }
      await p.emulateMedia({reducedMotion:'reduce'});
      if(await button('新建配置').evaluate(e=>getComputedStyle(e).transitionDuration)!=='0s')throw new Error('Reduced motion ignored');
      if(await p.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw new Error('Viewport overflow');
      await tab('备用空间').click();await ready();
    }
    await clickFeedback('删除配置');await p.getByRole('alertdialog').waitFor();
    await p.getByText('此配置仍被视频或同步任务使用。删除后，关联视频将无法通过此配置同步。本机视频、远端文件和未同步文案会保留。',{exact:true}).waitFor();
    await button('取消').click();if(await p.evaluate(()=>window.spaces.length)!==3)throw new Error('Cancel deleted configuration');
    await button('删除配置').click();await p.evaluate(()=>window.failDelete=true);await button('确认删除').click();
    await p.getByRole('alertdialog').getByRole('alert').waitFor();
    if(await p.evaluate(()=>window.current)!=='b')throw new Error('Failed deletion lost default');
    await p.evaluate(()=>window.failDelete=false);await button('确认删除').click();await ready();
    if(await p.evaluate(()=>window.current)!==null||await tab('备用空间').count())throw new Error('Default deletion failed');
    await tab('空配置').click();await ready();await p.evaluate(()=>window.raceReference=true);
    await button('删除配置').click();await p.getByRole('alertdialog').waitFor();
    await button('取消').click();await p.evaluate(()=>window.raceReference=false);
    await clickFeedback('删除配置');await ready();
    if(await tab('空配置').count()||await p.getByRole('alertdialog').count())throw new Error('Unused deletion did not complete directly');
    await p.evaluate(()=>window.failSpaceRefresh=true);
    await clickFeedback('停用远端备份');
    if(await p.evaluate(()=>window.syncEnabled)!==false||!await button('启用远端备份').count())throw new Error('Confirmed disable lost after refresh failure');
    await clickFeedback('启用远端备份');
    if(await p.evaluate(()=>window.syncEnabled)!==true||!await button('停用远端备份').count())throw new Error('Confirmed enable lost after refresh failure');
    await p.evaluate(()=>window.failSpaceRefresh=false);
    await p.evaluate(()=>{window.current='a';window.syncEnabled=true;window.spaces[0].enabled=true;});
    await button('关于软件').click();await button('系统设置').click();await button('远端备份').click();await ready();
    await clickFeedback('停用远端备份');await clickFeedback('启用远端备份');
    if(await p.getByRole('alertdialog').count()||await button('查看升级要求').count())throw new Error('Toggle exposed a protocol upgrade flow');
    if((await p.evaluate(()=>window.calls)).some(c=>c.command==='upgrade_backup_space'))throw new Error('Unexpected upgrade IPC');
    if(await p.evaluate(()=>window.current)!=='a')throw new Error('Toggle changed default storage');
    // The local toggle must finish even while the worker has not connected.
    await clickFeedback('停用远端备份');
    await p.evaluate(()=>{window.spaces[0].connection_pending=true;window.spaces[0].enabled=false;});
    await button('关于软件').click();await button('系统设置').click();await button('远端备份').click();await ready();
    await clickFeedback('启用远端备份');await ready();
    await p.getByRole('status').filter({hasText:'正在连接备份存储…'}).waitFor();
    if(!await button('停用远端备份').isEnabled())throw new Error('Pending connection locked stop');
    await p.locator('#backup-key').fill('unsaved-key');
    await p.evaluate(()=>window.spaces[0].last_error='BACKUP_TIMEOUT');
    await p.getByRole('alert').filter({hasText:'连接备份存储超时，请检查网络后重试'}).waitFor();
    if(await p.locator('#backup-key').inputValue()!=='unsaved-key')throw new Error('Worker state refresh replaced draft');
    await clickFeedback('重新连接');
    await p.getByRole('status').filter({hasText:'正在连接备份存储…'}).waitFor();
    if(await p.evaluate(()=>window.syncEnabled)!==true||await p.evaluate(()=>window.current)!=='a')throw new Error('Retry disabled backup or changed default');
    await p.evaluate(()=>{window.spaces[0].connection_pending=false;window.spaces[0].enabled=true;});
    await p.getByRole('status').filter({hasText:'正在连接备份存储…'}).waitFor({state:'hidden'});
    if(await p.locator('#backup-key').inputValue()!=='unsaved-key')throw new Error('Ready state refresh replaced draft');
    // Pending work can be stopped, and local polling stops with it.
    await clickFeedback('停用远端备份');
    await p.evaluate(()=>{window.spaces[0].connection_pending=true;window.spaces[0].enabled=false;});
    await clickFeedback('启用远端备份');
    await p.getByRole('status').filter({hasText:'正在连接备份存储…'}).waitFor();
    await clickFeedback('停用远端备份');
    await p.getByRole('status').filter({hasText:'正在连接备份存储…'}).waitFor({state:'hidden'});
    const refreshes=await p.evaluate(()=>window.calls.filter(c=>c.command==='list_backup_spaces').length);
    await p.waitForTimeout(1200);
    if(await p.evaluate(()=>window.calls.filter(c=>c.command==='list_backup_spaces').length)!==refreshes)throw new Error('Disabled backup still polls connection');
    await p.evaluate(()=>{window.spaces[0].connection_pending=false;window.spaces[0].cleanup_deleted_records=true;window.spaces[0].last_error=null;});
    await clickFeedback('启用远端备份');
    await p.evaluate(()=>window.spaces[0].last_error='BACKUP_CONDITIONAL_DELETE_UNSUPPORTED');
    await p.getByRole('alert').filter({hasText:'该存储不支持按版本清理记录，已停止清理'}).waitFor();
    if(await p.locator('#backup-key').inputValue()!=='unsaved-key')throw new Error('Cleanup error refresh replaced draft');
    await clickFeedback('停用远端备份');
    if(errors.length)throw new Error(errors.join('\n'));
    return {result:'PASS',viewports:3,toggle:true,pendingConnection:true,connectionErrorRetry:true,preserveDraft:true,cancelPending:true,defaultGate:true,noMigration:true,inlineValidation:true,requiredStars:true,transitions:true,referencedDeleteConfirm:true,deleteFailureRetry:true,referenceRaceGuard:true,emptyDelete:true,pageErrors:0};
  } finally {await p.close();}
}
