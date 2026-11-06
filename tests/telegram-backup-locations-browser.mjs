// Synthetic IPC fixtures. Switching providers never contacts a real service.
async page => {
  const results = [];
  for (const taskBacked of [true, false]) {
  const p = await page.context().newPage();
  const errors = []; p.on('pageerror', e => errors.push(e.message));
  await p.addInitScript(taskBacked => {
    window.taskBacked = taskBacked;
    localStorage.setItem('yingqu.task-view', 'feed');
    window.current = 'r2'; window.calls = []; window.uploaded = false;
    const base = { region:'auto', path_style:true, credentials_configured:false, connection_pending:false, last_error:null, can_delete_owned_media:true };
    window.spaces = [
      { ...base, space_id:'r2', library_id:'r2-library', provider_type:'s3', profile:'r2', display_name:'视频归档', endpoint:'https://fixture.r2.cloudflarestorage.com', bucket:'bucket', prefix:'yingqu/', enabled:true },
      { ...base, space_id:'tg', library_id:'tg-library', provider_type:'telegram', profile:'telegram', display_name:'Telegram 备份', endpoint:'', bucket:'', prefix:'', telegram:{account_id:'123',channel_id:'777'}, telegram_channel_title:'知识库', enabled:false },
    ];
    const metadata = { title:'测试视频标题', author:'测试作者', platform:'youtube', video_id:'12345678901', canonical_url:'https://www.youtube.com/watch?v=12345678901', kind:'video', resolver:'yt-dlp', duration_ms:2000, width:1920, height:1080 };
    window.task = { id:'local-task', asset_id:'asset', revision:1, status:'completed', video:metadata, bytes:100, total_bytes:100, speed_bps:null, error:null, path:'C:/fixture/video.mp4', created_at:1, completed_at:2, canceling:false, file_missing:false, download_requested:true, cover_cached:false, files:[] };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    window.__TAURI_INTERNALS__ = { convertFileSrc:x=>x, transformCallback:()=>1, unregisterCallback() {}, invoke:async (command,args) => {
      window.calls.push({command,args});
      if(command==='get_settings') return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
      if(command==='list_tasks') return window.taskBacked ? [structuredClone(window.task)] : [];
      if(command==='get_backup_settings') return structuredClone(window.spaces.find(s=>s.space_id===window.current));
      if(command==='list_backup_spaces') return structuredClone(window.spaces);
      if(command==='get_backup_sync_enabled') return true;
      if(command==='list_backup_assets') return [{asset_id:'asset',task_id:window.taskBacked ? 'local-task' : null,metadata,original_filename:'video.mp4',content_bytes:100,content_sha256:'a'.repeat(64),asset_state:'active',path:window.task.path,local_state:window.task.path?'present':'missing',space_id:args.spaceId,remote_state:args.spaceId==='r2'||window.uploaded?'backed_up':'not_backed_up',in_current_space:args.spaceId==='r2'||window.uploaded,has_cover:false,has_transcript:false,remote_revision:0,remote_deleted:false,has_pending_local_changes:false,conflict_count:0,backup_locations:[{space_id:'r2',profile:'r2',display_name:'视频归档',channel_title:null},...(window.uploaded?[{space_id:'tg',profile:'telegram',display_name:'Telegram 备份',channel_title:'知识库'}]:[])]}];
      if(command==='select_backup_space') {window.current=args.spaceId;window.spaces.forEach(s=>s.enabled=s.space_id===window.current);return structuredClone(window.spaces.find(s=>s.space_id===window.current));}
      if(command==='start_asset_backup') {window.uploaded=true;return;}
      if(command==='telegram_storage_status') return {configured:true,operation_id:'login',phase:'ready',account:{id:'123',name:'测试账号'},last_error:null};
      if(command==='list_backup_operations') return window.failedUpload ? [{operation_id:'failed-upload',asset_id:'asset',space_id:'tg',kind:'upload',state:'blocked',phase:'prepared',attempt:1,last_error:'TELEGRAM_CHANNEL_FORBIDDEN',payload:{}}] : [];
      if(['list_video_transcripts','list_backup_operations'].includes(command)) return [];
      if(command==='check_runtime_dependencies') return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};
  }, taskBacked);
  const button = name => p.getByRole('button',{name,exact:true});
  const assert = (value,message) => {if(!value) throw new Error(message);};
  try {
    await p.goto(page.url().split('/').slice(0,3).join('/')+'/'); await button('下载任务').click();
    const row = p.locator(taskBacked ? '[data-task-row="local-task"]' : '[data-asset-id="asset"]');
    await row.locator('[data-complete="true"]').waitFor();
    await p.waitForFunction(()=>window.calls.filter(c=>c.command==='list_backup_assets'&&c.args.verifyLocal===false).length>=2);
    await row.locator('.task-card-message').hover();
    await p.getByRole('tooltip').filter({hasText:/^已备份至：视频归档$/}).waitFor();
    await button('系统设置').click();
    assert((await p.getByRole('navigation',{name:'设置分类'}).getByRole('button').allTextContents()).join('|') === '下载设置|平台账号|远端备份|运行依赖','Settings sections are out of order');
    await button('远端备份').click();
    assert(await p.getByRole('combobox',{name:'备份提供商'}).count()===0,'Saved R2 provider remains editable');
    await p.getByText('Cloudflare R2',{exact:true}).waitFor();
    await p.getByRole('tab',{name:'Telegram 备份',exact:true}).click(); await button('设为默认').click();
    await p.waitForFunction(()=>window.current==='tg'); await p.getByText('默认备份空间已切换',{exact:true}).waitFor(); await button('下载任务').click();
    await row.locator('[data-complete="true"]').waitFor();
    await row.locator('.task-card-message').hover();
    await p.getByRole('tooltip').filter({hasText:/^已备份至：视频归档$/}).waitFor();
    await p.evaluate(()=>{window.failedUpload=true;});
    await row.locator('[data-complete="true"]').waitFor({state:'detached'});
    await row.locator('.task-card-notice').hover();
    await p.getByRole('tooltip').filter({hasText:'无法访问频道，请检查频道成员与管理权限'}).waitFor();
    await p.evaluate(()=>{window.failedUpload=false;});
    await row.locator('[data-complete="true"]').waitFor();
    await row.click({button:'right'});
    const item = name => p.getByRole('menuitem',{name,exact:true});
    assert(await item('上传云端').isEnabled(),'Unjoined local asset cannot upload to Telegram');
    assert(await item('从远端下载').isDisabled(),'Unbacked Telegram asset can download');
    assert(await item('从当前同步库删除').count()===0,'Virtual membership exposes remote deletion');
    assert(await item('释放本机空间').count()===0,'Old-space receipt enables release against new target');
    await item('上传云端').click();
    await p.waitForFunction(()=>window.calls.some(c=>c.command==='start_asset_backup'&&c.args.spaceId==='tg'&&c.args.assetId==='asset'));
    await row.locator('[data-complete="true"]:disabled').waitFor();
    assert(await p.evaluate(()=>window.calls.some(c=>c.command==='list_backup_assets'&&c.args.spaceId==='tg'&&c.args.verifyLocal===true)), 'Upload action did not refresh local file presence');
    await row.locator('.task-card-message').hover();
    await p.getByRole('tooltip').filter({hasText:/^已备份至：视频归档、Telegram 备份$/}).waitFor();
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.mouse.move(0, 0);
      await button('下载任务').focus();
      await p.setViewportSize({width,height});
      await row.locator('.task-card-message').scrollIntoViewIfNeeded();
      await p.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      await row.locator('.task-card-message').focus();
      await p.getByRole('tooltip').filter({hasText:/^已备份至：视频归档、Telegram 备份$/}).waitFor();
      const visibleTip = p.locator('[data-slot="tooltip-content"]').filter({hasText:'已备份至：视频归档、Telegram 备份'});
      await visibleTip.waitFor({state:'visible'});
      await p.waitForFunction(() => {
        const tip = [...document.querySelectorAll('[data-slot="tooltip-content"]')].find(el => el.textContent.includes('已备份至：视频归档、Telegram 备份'));
        const rect = tip?.getBoundingClientRect();
        return rect && rect.width > 0 && rect.x >= 0 && rect.y >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
      });
      const tip = await visibleTip.boundingBox();
      assert(tip && tip.x>=0 && tip.y>=0 && tip.x+tip.width<=width && tip.y+tip.height<=height,'Backup tooltip escaped viewport');
      await p.keyboard.press('Escape');
      assert(await row.locator('[data-complete="true"]').isVisible(),'Card mode loses backup status');
      assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Horizontal overflow');
    }
    assert(errors.length===0,errors.join('\n'));
    results.push({taskBacked,viewports:3,cardBackupIcon:true,crossProviderReceipts:true,currentTargetUpload:true,noFalseDeletionOrRelease:true});
  } finally {await p.close();}
  }
  return {passed:true,results};
}
