// Production UI with synthetic IPC. Never installs dependencies or changes accounts.
async page => {
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);await p.setViewportSize({width,height});
    const errors=[];p.on('pageerror',e=>errors.push(e.message));
    await p.addInitScript(()=>{
      window.errorQA={fail:null,calls:[],
        status:{configured:true,phase:'ready',operation_id:'login',account:{id:'123',name:'测试账号'},last_error:'CANCELED',qr:null,qr_expires_at:null},
        space:{space_id:'space',library_id:'library',display_name:'Telegram',provider_type:'telegram',profile:'telegram',endpoint:'',bucket:'',prefix:'',region:'auto',path_style:true,enabled:true,credentials_configured:false,connection_pending:false,telegram:{account_id:'123',channel_id:'777'},telegram_channel_title:'测试频道',last_error:null},
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async command=>{
        const qa=window.errorQA;qa.calls.push(command);
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='get_backup_settings')return structuredClone(qa.space);
        if(command==='list_backup_spaces')return [structuredClone(qa.space)];
        if(command==='get_backup_sync_enabled')return true;
        if(command==='telegram_storage_status')return structuredClone(qa.status);
        if(command==='save_backup_settings'){if(qa.fail)throw qa.fail;return structuredClone(qa.space);}
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[{id:'ffmpeg',available:false,can_install:true}]};
        if(command==='install_runtime_dependency')throw 'CANCELED';
        if(['list_tasks','list_backup_assets','list_backup_operations','list_video_transcripts'].includes(command))return [];
        return null;
      }};
    });
    const button=name=>p.getByRole('button',{name,exact:true});
    const openBackup=async()=>{await button('远端备份').click();await p.getByText('测试账号',{exact:true}).waitFor();};
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await button('系统设置').click();await openBackup();
      if(await p.getByRole('alert').count())throw Error('Background cancellation leaked into backup settings');
      for(const code of ['TELEGRAM_MEDIA_MISSING','TELEGRAM_CHANNEL_FORBIDDEN','TELEGRAM_INDEX_INVALID']) {
        await button('下载设置').click();await p.evaluate(code=>window.errorQA.status.last_error=code,code);await openBackup();
        if(await p.getByRole('alert').count())throw Error(`Background task error leaked: ${code}`);
      }
      for(const [code,message] of [['CANCELED','操作已取消'],['TELEGRAM_CHANNEL_FORBIDDEN','无法访问频道，请检查频道成员与管理权限']]) {
        await p.evaluate(code=>window.errorQA.fail=code,code);await button('重新连接').click();
        await p.getByRole('alert').filter({hasText:message}).waitFor();
      }
      await p.evaluate(()=>{window.errorQA.fail=null;window.errorQA.status.last_error=null;});
      await button('重新连接').click();await p.getByRole('alert').waitFor({state:'hidden'});
      await button('运行依赖').click();await button('下载 FFmpeg').click();
      await p.getByRole('alert').filter({hasText:'依赖操作已取消'}).waitFor();
      await openBackup();
      if(await p.getByRole('alert').count())throw Error('Dependency error visible on backup page');
      if(errors.length)throw Error(errors.join('\n'));
      results.push({width,height,backgroundTaskErrorsHidden:true,foregroundErrorsVisible:true,dependencyErrorScoped:true});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
