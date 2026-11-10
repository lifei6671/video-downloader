// Production UI with isolated IPC fixtures; no account, credentials or network media.
async (page) => {
  const p = await page.context().newPage();
  p.setDefaultTimeout(7000);
  let stage='initial';
  const errors=[]; p.on('pageerror',e=>errors.push(e.message));
  await p.addInitScript(() => {
    window.calls=[]; window.tasks=[];
    const signedOut=()=>({configured:true,operation_id:null,phase:'signed_out',account:null,last_error:null,qr:null,qr_expires_at:null});
    const shared=signedOut(); window.accounts={download:shared,backup:shared}; window.pending={};
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
    window.__TAURI_INTERNALS__={convertFileSrc:value=>value,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args={})=>{
      window.calls.push({command,purpose:args.purpose,otherAccount:args.otherAccount});
      const purpose=args.purpose??'backup', other=purpose==='download'?'backup':'download';
      const shown=()=>window.pending[purpose]??window.accounts[purpose];
      if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
      if(command==='list_tasks')return structuredClone(window.tasks);
      if(['list_video_transcripts','list_backup_spaces','list_backup_assets','list_backup_operations'].includes(command))return [];
      if(command==='get_backup_settings')return null;
      if(command==='get_backup_sync_enabled')return false;
      if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
      if(command==='telegram_storage_channels')return [{id:'777',title:'当前账号私有频道',can_post:true,can_delete:false}];
      if(command==='telegram_storage_status')return structuredClone(shown());
      if(command==='telegram_storage_resume') {
        if(window.accounts[purpose].phase==='signed_out' && window.accounts[other].phase==='ready')window.accounts[purpose]=window.accounts[other];
        if(shown().phase==='signed_out')shown().operation_id='old-restore-operation';
        return structuredClone(shown());
      }
      if(command==='telegram_storage_login_qr') {
        if(!args.otherAccount && window.accounts[purpose].phase==='ready')return structuredClone(window.accounts[purpose]);
        if(args.otherAccount)window.pending[purpose]=signedOut();
        Object.assign(shown(),{operation_id:'login-'+purpose,phase:'qr',qr:[[true,false],[false,true]]});
        return await new Promise(resolve=>{window.finishQr=(name='账号 A')=>{
          Object.assign(shown(),{phase:'ready',qr:null,account:{id:name,name}});
          if(window.pending[purpose]){window.accounts[purpose]=window.pending[purpose];delete window.pending[purpose];}
          resolve(structuredClone(shown()));
        };});
      }
      if(command==='telegram_storage_cancel') {
        if(window.pending[purpose])delete window.pending[purpose];else Object.assign(window.accounts[purpose],signedOut());
        return structuredClone(shown());
      }
      if(command==='telegram_storage_logout') {
        window.accounts[purpose]=signedOut();return structuredClone(shown());
      }
      if(command==='resolve_download_link') {
        if(args.resolver!=='grammers')throw 'Wrong resolver';
        const video={platform:'telegram',resolver:'grammers',kind:'video',video_id:'100_9',canonical_url:'https://t.me/c/100/9',title:'测试原始视频',original_filename:'original.mp4',mime_type:'video/mp4',filesize:4096,selected_format_id:'original',formats:[{format_id:'original',container:'mp4',filesize:4096}],author_profile:{id:'100',nickname:null,handle:null,kind:'channel',avatar_cached:false}};
        const task={id:'tg-1',revision:1,status:'ready',video,bytes:0,total_bytes:null,speed_bps:null,error:null,path:null,created_at:1,completed_at:null,canceling:false,file_missing:false,queue_order:null,download_requested:false,cover_path:null,cover_cached:false,files:[],telegram_link:'https://t.me/c/100/9'};
        window.tasks=[task];return structuredClone(task);
      }
      if(command==='start_download')return null;
      return null;
    }};
  });
  const assert=(v,m)=>{if(!v)throw new Error(m);};
  const button=name=>p.getByRole('button',{name,exact:true});
  try {
    await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
    await button('系统设置').click(); await button('平台账号').click();
    await p.getByRole('tab',{name:'Telegram',exact:true}).click();
    stage='initial account login'; await button('扫码登录').waitFor();
    assert(await p.getByRole('combobox',{name:'私有频道'}).count()===0,'Platform login requires backup channel');
    await button('恢复登录').click();await p.getByText('没有可恢复的登录，请扫码登录。',{exact:true}).waitFor();
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      assert(await p.evaluate(()=>window.accounts.download.phase==='signed_out' && !!window.accounts.download.operation_id),'Missing stale download restore fixture');
      assert(await button('退出账号').count()===0,'Signed-out download account exposes logout');
    }

    await button('扫码登录').click();await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();
    await p.evaluate(()=>window.finishQr());await p.getByText('账号 A',{exact:true}).filter({visible:true}).waitFor();
    assert(!(await p.evaluate(()=>window.calls)).some(call=>call.command==='telegram_storage_channels'),'Account-only UI enumerated backup channels');
    const backupPage=async()=>{
      await button('远端备份').click();
      await p.getByRole('combobox',{name:'备份提供商'}).click();await p.getByRole('option',{name:'Telegram',exact:true}).click();
    };
    const downloadPage=async()=>{await button('平台账号').click();await p.getByRole('tab',{name:'Telegram',exact:true}).click();};
    stage='backup reuses download account'; await backupPage();
    await p.getByText('账号 A',{exact:true}).filter({visible:true}).waitFor();await button('切换其他账号').waitFor();
    assert((await p.evaluate(()=>window.calls)).filter(call=>call.command==='telegram_storage_login_qr').length===1,'Backup asked for a second QR login');
    stage='canceled replacement preserves shared account';await button('切换其他账号').click();
    await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();await p.keyboard.press('Escape');
    await p.getByText('账号 A',{exact:true}).filter({visible:true}).waitFor();
    stage='backup switches independently';await button('切换其他账号').click();
    await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();await p.evaluate(()=>window.finishQr('账号 B'));
    await p.getByText('账号 B',{exact:true}).filter({visible:true}).waitFor();
    await downloadPage();await p.getByText('账号 A',{exact:true}).filter({visible:true}).waitFor();
    stage='download switches independently';await button('切换其他账号').click();
    await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();await p.evaluate(()=>window.finishQr('账号 C'));
    await p.getByText('账号 C',{exact:true}).filter({visible:true}).waitFor();
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Horizontal overflow');
      await button('切换其他账号').focus();assert(await button('切换其他账号').evaluate(el=>el===document.activeElement),'Switch action cannot be focused');
      await p.screenshot({path:'.local/telegram-accounts/download-'+width+'.png'});
    }
    await backupPage();await p.getByText('账号 B',{exact:true}).filter({visible:true}).waitFor();
    await p.screenshot({path:'.local/telegram-accounts/backup-1024.png'});
    stage='logout preserves other purpose';await button('退出存储账号').click();await button('扫码登录').waitFor();
    await downloadPage();await p.getByText('账号 C',{exact:true}).filter({visible:true}).waitFor();
    stage='default login reuses other account';await backupPage();await button('恢复登录').click();
    await p.getByText('账号 C',{exact:true}).filter({visible:true}).waitFor();
    assert((await p.evaluate(()=>window.calls)).filter(call=>call.command==='telegram_storage_login_qr'&&call.otherAccount).every(call=>['backup','download'].includes(call.purpose)),'Switch lost purpose');
    await button('运行依赖').click();
    assert(await p.getByText(/tdl/i).count()===0,'Removed external dependency remains visible');
    await button('下载作品').click();
    await p.getByRole('combobox',{name:'解析引擎'}).click();await p.getByRole('option',{name:'Telegram',exact:true}).click();
    await p.getByRole('textbox',{name:'作品分享链接'}).fill('https://t.me/c/100/9');await button('解析链接').click();
    await p.getByText('测试原始视频',{exact:true}).first().waitFor();
    assert(await p.getByText('原始文件',{exact:true}).isVisible(),'Original format missing');
    assert(await p.getByText(/实验模式|tdl 服务/).count()===0,'Old HTTP flow text remains visible');
    await p.screenshot({path:'.local/telegram-download-browser.png'});
    assert(!(await p.evaluate(()=>window.calls)).some(call=>call.command.includes('tdl')),'UI invoked removed tdl command');
    assert(errors.length===0,errors.join('\n'));
    return {passed:true,viewports:3,flows:['account-only','resume','shared-qr','escape-retains-account','backup-switch-isolation','download-switch-isolation','logout-isolation','reuse-other-account','dependency-removal','grammers-resolver','original-format']};
  } catch(error) { await p.screenshot({path:'.local/telegram-download-browser-failure.png'}); throw new Error(stage+': '+error.message); } finally {await p.close();}
}
