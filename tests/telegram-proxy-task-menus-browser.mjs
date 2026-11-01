// Synthetic IPC only: no real credentials, sessions, files or remote requests.
async page => {
  const p = await page.context().newPage(); p.setDefaultTimeout(7000);
  const errors = []; p.on('pageerror', e => errors.push(e.message));
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await p.addInitScript(() => {
    localStorage.setItem('yingqu.task-view', 'list');
    window.proxy = null; window.calls = []; window.proxyReadFails = true;
    const profile = () => ({ platform:'telegram', user_agent:'fixture', proxy:window.proxy, cookie_configured:false });
    const settings = () => ({ download_directory:'C:/fixture', filename_template:'{title}', max_downloads:2, profiles:[profile()] });
    const metadata = { platform:'telegram', resolver:'grammers', kind:'video', title:'本机测试视频', author:'测试作者', video_id:'100_9', canonical_url:'https://t.me/c/100/9' };
    const task = { id:'local', revision:1, status:'completed', video:metadata, path:'C:/fixture/video.mp4', bytes:100, total_bytes:100, created_at:1, completed_at:2, canceling:false, file_missing:false, download_requested:true, cover_cached:false, files:[] };
    const space = { space_id:'space', library_id:'library', display_name:'测试备份', provider_type:'telegram', profile:'telegram', telegram:{account_id:'test',channel_id:'100'}, enabled:true, credentials_configured:false, can_delete_owned_media:false, last_error:null };
    const asset = { asset_id:'remote', task_id:null, metadata:{...metadata,title:'远端测试视频'}, original_filename:'video.mp4', content_bytes:100, content_sha256:'a'.repeat(64), asset_state:'active', path:null, local_state:'missing', space_id:'space', remote_state:'backed_up', has_cover:false, has_transcript:true, remote_revision:1, remote_deleted:false, has_pending_local_changes:false, conflict_count:0 };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
    window.__TAURI_INTERNALS__ = {convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
      window.calls.push({command,args});
      if(command==='get_settings') return settings();
      if(command==='get_profile_proxy') { if(window.proxyReadFails) throw 'CREDENTIAL_ERROR'; return window.proxy; }
      if(command==='save_profile') { if(window.saveFails) throw 'DATABASE_ERROR'; window.proxy=args.update.proxy||null; return settings(); }
      if(command==='list_tasks') return [task];
      if(command==='get_backup_settings') return space;
      if(command==='list_backup_spaces') return [space];
      if(command==='get_backup_sync_enabled') return true;
      if(command==='list_backup_assets') return [asset];
      if(command==='list_video_transcripts'||command==='list_backup_operations') return [];
      if(command==='check_runtime_dependencies') return {platform:'windows',directory:'C:/fixture',tools:[]};
      if(command==='telegram_storage_status') return {configured:true,phase:'ready',operation_id:'session',account:{id:'test',name:'测试账号'}};
      return null;
    }};
  });
  try {
    await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
    await p.getByRole('button',{name:'系统设置',exact:true}).click();
    await p.getByRole('button',{name:'平台账号',exact:true}).click();
    await p.getByRole('tab',{name:'Telegram',exact:true}).click();
    const form = p.getByRole('form',{name:'Telegram 网络代理'});
    await form.getByRole('alert').waitFor();
    check(await form.getByRole('button',{name:'保存代理',exact:true}).isDisabled(),'failed read allowed proxy overwrite');
    await p.evaluate(()=>window.proxyReadFails=false);
    await form.getByRole('button',{name:'重新读取代理',exact:true}).click();
    await form.getByRole('radio',{name:'自定义代理',exact:true}).click();
    await form.getByRole('textbox',{name:'SOCKS5 地址',exact:true}).fill('socks5://127.0.0.1:1080');
    await p.evaluate(()=>window.saveFails=true); await form.getByRole('button',{name:'保存代理',exact:true}).click();
    await form.getByRole('alert').waitFor(); check(await p.evaluate(()=>window.proxy)===null,'failed save mutated configuration');
    await p.evaluate(()=>window.saveFails=false); await form.getByRole('button',{name:'保存代理',exact:true}).click();
    await form.getByRole('status').waitFor(); check(await p.evaluate(()=>window.proxy)==='socks5://127.0.0.1:1080','proxy not saved');
    await form.getByRole('radio',{name:'直连',exact:true}).click(); await form.getByRole('button',{name:'保存代理',exact:true}).click();
    await p.waitForFunction(()=>window.proxy===null);
    await p.getByRole('button',{name:'下载任务',exact:true}).click();
    const menu = () => p.getByRole('menu',{name:'任务操作',exact:true});
    const local = () => p.locator('[data-task-id="local"]'); const remote = () => p.locator('[data-asset-id="remote"]');
    const entries = async target => { await target.click({button:'right',force:true}); const labels = await menu().getByRole('menuitem').allTextContents(); await p.keyboard.press('Escape'); return labels; };
    const cases = [];
    for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      const listItems = await entries(local().locator('.task-row-title'));
      check(JSON.stringify(await entries(local().locator('[data-media-cover]')))===JSON.stringify(listItems),'list cover menu differs');
      const edge = await remote().evaluate(row=>({right:row.getBoundingClientRect().right,controls:row.querySelector('.task-row-controls').getBoundingClientRect().right}));
      check(edge.right-edge.controls<=20,'remote icons too far from right edge');
      check(await remote().getByRole('button',{name:'更多操作',exact:true}).count()===0,'remote ellipsis remains');
      await remote().click({button:'right'}); const remoteList = await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem').allTextContents(); await p.keyboard.press('Escape');
      await p.getByRole('button',{name:'切换为信息流',exact:true}).click();
      check(JSON.stringify(await entries(local().locator('.task-row-title')))===JSON.stringify(listItems),'list/card menu differs');
      await remote().click({button:'right'}); check(JSON.stringify(await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem').allTextContents())===JSON.stringify(remoteList),'remote list/card menu differs'); await p.keyboard.press('Escape');
      await p.getByRole('button',{name:'切换为列表',exact:true}).click();
      const trigger = local().locator('..'); await trigger.focus(); await p.keyboard.press('Shift+F10'); await menu().waitFor(); await p.keyboard.press('Escape');
      cases.push(width);
    }
    check(errors.length===0, errors.join('\n'));
    return {passed:true,viewports:cases,flows:['proxy read/save failure','SOCKS5 save','direct','local/remote menu parity','no ellipsis','right alignment','keyboard']};
  } finally { await p.close(); }
}
