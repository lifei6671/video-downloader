// Production renderer, delayed synthetic IPC; no account or storage mutations.
async page => {
  const results = [];
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    const p = await page.context().newPage();
    p.setDefaultTimeout(7000);
    await p.setViewportSize({width, height});
    const errors = [];
    p.on('pageerror', error => errors.push(error.message));
    await p.addInitScript(() => {
      window.navigationQA = {calls: [], account: '导航测试账号'};
      const space = {space_id:'space',library_id:'library',display_name:'Telegram',provider_type:'telegram',profile:'telegram',endpoint:'',bucket:'',prefix:'',region:'auto',path_style:true,enabled:true,credentials_configured:false,connection_pending:false,telegram:{account_id:'123',channel_id:'777'},telegram_channel_title:'测试频道',last_error:null};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
      window.__TAURI_INTERNALS__ = {convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async command=>{
        const qa = window.navigationQA;
        qa.calls.push(command);
        if (['get_backup_settings','list_backup_spaces','get_backup_sync_enabled','telegram_storage_status'].includes(command)) await new Promise(resolve=>setTimeout(resolve,250));
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='get_backup_settings')return structuredClone(space);
        if(command==='list_backup_spaces')return [structuredClone(space)];
        if(command==='get_backup_sync_enabled')return true;
        if(command==='telegram_storage_status')return {configured:true,phase:'ready',operation_id:'login',account:{id:'123',name:qa.account},last_error:null,qr:null,qr_expires_at:null};
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(['list_tasks','list_backup_assets','list_backup_operations','list_video_transcripts'].includes(command))return [];
        return null;
      }};
    });
    const button = name => p.getByRole('button', {name,exact:true});
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await button('系统设置').click();
      await button('远端备份').click();
      await p.getByText('导航测试账号',{exact:true}).waitFor();
      await p.evaluate(()=>{
        window.navigationQA.node=document.querySelector('.backup-settings-actions');
      });
      for (const destination of ['下载作品','下载任务','关于软件','下载设置','运行依赖']) {
        await button(destination).click();
        if(await button('重新连接').isVisible())throw Error('Hidden settings remain interactive');
        await p.waitForTimeout(300); // Let any request issued before leaving settle.
        const polls = await p.evaluate(()=>window.navigationQA.calls.filter(c=>c==='telegram_storage_status').length);
        await p.waitForTimeout(1150);
        if(await p.evaluate(()=>window.navigationQA.calls.filter(c=>c==='telegram_storage_status').length)!==polls)throw Error('Hidden Telegram settings keep polling');
        await p.evaluate(()=>{
          const qa=window.navigationQA;
          qa.visibleFrames=0;qa.incompleteFrames=0;qa.recording=true;
          function sample(){
            const node=document.querySelector('.backup-settings-actions');
            if(node?.getBoundingClientRect().height){
              qa.visibleFrames++;
              if(node!==qa.node || !node.textContent.includes('导航测试账号') || node.querySelector('#backup-space-name'))qa.incompleteFrames++;
            }
            if(qa.recording)requestAnimationFrame(sample);
          }
          requestAnimationFrame(sample);
        });
        const returnButton = button(['下载设置','运行依赖'].includes(destination)?'远端备份':'系统设置');
        await returnButton.focus();await p.keyboard.press('Enter');
        await p.waitForTimeout(400);
        const sample = await p.evaluate(()=>{
          const qa=window.navigationQA;qa.recording=false;
          return {visible:qa.visibleFrames,incomplete:qa.incompleteFrames,same:document.querySelector('.backup-settings-actions')===qa.node};
        });
        if(!sample.same || !sample.visible || sample.incomplete)throw Error(`Page reset on return from ${destination}: ${JSON.stringify(sample)}`);
      }
      // Resume refresh updates the retained account without a placeholder page.
      await button('下载作品').click();
      await p.evaluate(()=>window.navigationQA.account='刷新后的账号');
      await button('系统设置').click();
      await p.getByText('刷新后的账号',{exact:true}).waitFor();
      // New-space draft and selected tab survive navigation too.
      await p.getByRole('tab',{name:'新建空间',exact:true}).click();
      await p.getByLabel('空间名称',{exact:true}).fill('未保存草稿');
      await button('下载作品').click();await button('系统设置').click();
      if(await p.getByLabel('空间名称',{exact:true}).inputValue()!=='未保存草稿')throw Error('Draft lost across page navigation');
      await button('下载设置').click();await button('远端备份').click();
      if(await p.getByLabel('空间名称',{exact:true}).inputValue()!=='未保存草稿')throw Error('Draft lost across settings navigation');
      if(errors.length)throw Error(errors.join('\n'));
      results.push({width,height,retained:true,incompleteReturnFrames:0,hiddenPolling:false,refreshOnReturn:true,draftPreserved:true,keyboardReturn:true});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
