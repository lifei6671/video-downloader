// Production settings components with isolated IPC; no native account or cloud writes.
async page => {
  const base = page.url().split('/').slice(0,3).join('/');
  const p = await page.context().newPage();
  const results = [];
  await p.addInitScript(() => {
    window.connectionQA = {
      calls: [], failConnect: false,
      status: {configured:true,phase:'ready',operation_id:'fixture-login',account:{id:'123',name:'隔离测试账号'},last_error:'TELEGRAM_RECONNECTING',qr:null,qr_expires_at:null},
      space: {space_id:'fixture-space',library_id:'fixture-library',display_name:'Telegram',provider_type:'telegram',profile:'telegram',endpoint:'',bucket:'',prefix:'',region:'auto',path_style:true,enabled:true,credentials_configured:false,connection_pending:false,cleanup_deleted_records:false,deleted_record_retention_days:30,can_delete_owned_media:true,telegram:{account_id:'123',channel_id:'777'},telegram_channel_title:'隔离频道',last_error:'TELEGRAM_RECONNECTING'},
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener:()=>{}};
    window.__TAURI_INTERNALS__ = {convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async command=>{
      const qa = window.connectionQA; qa.calls.push(command);
      if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
      if(command==='get_backup_settings')return structuredClone(qa.space);
      if(command==='list_backup_spaces')return [structuredClone(qa.space)];
      if(command==='get_backup_sync_enabled')return true;
      if(command==='telegram_storage_status')return structuredClone(qa.status);
      if(command==='save_backup_settings') {
        if(qa.failFastRecovered) throw 'TELEGRAM_TRANSFER_TIMEOUT';
        if(qa.failConnect) {qa.space.last_error='TELEGRAM_RECONNECTING';qa.status.last_error='TELEGRAM_RECONNECTING';throw 'TELEGRAM_TRANSFER_TIMEOUT';}
        return structuredClone(qa.space);
      }
      if(['list_tasks','list_video_transcripts','list_backup_assets','list_backup_operations'].includes(command))return [];
      if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
      return null;
    }};
  });
  const note = () => p.getByRole('status').filter({hasText:'正在重连 Telegram，连接恢复后自动继续'});
  const clear = async () => {
    await p.evaluate(()=>{window.connectionQA.status.last_error=null;window.connectionQA.space.last_error=null;});
    await p.waitForFunction(()=>!document.body.textContent.includes('正在重连 Telegram，连接恢复后自动继续'));
  };
  try {
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      await p.goto(base+'/');
      await p.getByRole('button',{name:'系统设置',exact:true}).click();
      await p.getByRole('button',{name:'远端备份',exact:true}).click();
      await note().first().waitFor();
      await p.getByText('隔离测试账号',{exact:true}).waitFor();
      if(await note().count()!==1)throw Error('account and space repeated the same reconnect message');
      if(await p.getByRole('alert').count())throw Error('automatic reconnect shown as terminal failure');
      if(await p.getByRole('button',{name:'重新连接',exact:true}).isDisabled())throw Error('existing connection action unexpectedly locked');
      await clear();
      if(await p.getByRole('alert').count())throw Error('stale timeout remained after recovery');
      if(await p.evaluate(()=>window.connectionQA.calls.some(command=>['telegram_storage_resume','save_backup_settings','set_backup_sync_enabled'].includes(command))))throw Error('status polling performed a remote action');
      await p.evaluate(()=>{window.connectionQA.failConnect=true;});
      await p.getByRole('button',{name:'重新连接',exact:true}).click();
      await note().first().waitFor();
      if(await p.getByRole('alert').count())throw Error('foreground timeout masked active reconnect');
      await clear();
      if(await p.getByRole('alert').count())throw Error('foreground network error stayed after successful recovery');
      await p.evaluate(()=>{window.connectionQA.failConnect=false;window.connectionQA.failFastRecovered=true;});
      await p.getByRole('button',{name:'重新连接',exact:true}).click();
      await p.waitForFunction(()=>Array.from(document.querySelectorAll('button')).some(button=>button.textContent.includes('重新连接')&&!button.disabled));
      await p.waitForFunction(()=>!document.body.textContent.includes('Telegram 传输超时，请重试'));
      if(await p.getByRole('alert').count())throw Error('fast recovery missed between polls left a stale foreground error');
      await p.evaluate(()=>{window.connectionQA.status.last_error='TELEGRAM_AUTH_REQUIRED';window.connectionQA.status.phase='signed_out';window.connectionQA.status.account=null;window.connectionQA.space.last_error='TELEGRAM_AUTH_REQUIRED';});
      await p.getByRole('alert').first().waitFor();
      if(await note().count())throw Error('authorization failure falsely kept reconnecting');
      if(await p.evaluate(()=>window.connectionQA.calls.filter(command=>command==='save_backup_settings').length)!==2)throw Error('automatic reconnect replayed configuration writes');
      results.push({width,height,reconnectIsStatus:true,recoveryClearsBothSources:true,foregroundTimeoutClears:true,fastRecoveryWithoutIntermediatePoll:true,authorizationVisible:true});
    }
    return {passed:results.length,results};
  } finally {await p.close();}
}
