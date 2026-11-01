// Isolated IPC: only synthetic tasks, no accounts or real transfers.
async page => {
  const results = [];
  for (const [width, height] of [[1440,900],[1280,800],[1024,680]]) {
    const p = await page.context().newPage(); p.setDefaultTimeout(7000);
    const errors = []; p.on('pageerror', error => errors.push(error.message));
    const check = (ok, message) => { if (!ok) throw Error(width + ': ' + message); };
    await p.addInitScript(() => {
      localStorage.setItem('yingqu.task-view', 'feed'); window.calls = [];
      const video = {platform:'telegram',kind:'video',video_id:'123',title:'等待下载测试',canonical_url:'https://t.me/example/123'};
      window.tasks = ['queued','deferred','downloading','canceling'].map((id, index) => ({id,revision:1,status:id==='deferred'?'queued':id==='canceling'?'downloading':id,video:id==='deferred'?null:video,telegram_link:'https://t.me/example/123',bytes:30,total_bytes:100,path:null,created_at:10-index,completed_at:null,canceling:id==='canceling',file_missing:false,download_requested:true,cover_cached:false,files:[]}));
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
      window.__TAURI_INTERNALS__ = {convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
        window.calls.push({command,args});
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='list_tasks')return structuredClone(window.tasks);
        if(['list_backup_spaces','list_backup_assets','list_backup_operations','list_video_transcripts'].includes(command))return [];
        if(command==='get_backup_settings')return {enabled:false};
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(command==='cancel_task'){const task=window.tasks.find(task=>task.id===args.taskId);task.canceling=true;task.revision++;return null;}
        return null;
      }};
    });
    try {
      await p.setViewportSize({width,height}); await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      const row = id => p.locator('[data-task-id="'+id+'"]');
      for(const id of ['queued','deferred','downloading']) {
        const target=row(id), cancel=target.getByRole('button',{name:'取消下载',exact:true});
        await cancel.scrollIntoViewIfNeeded();
        check(await target.locator('.task-row-footer,.task-queue-position,.task-feed-cancel').count()===0,id+' retains footer, queue position or extra cancel');
        check(await target.locator('.task-row-cover button.task-feed-progress-cancel').count()===1,id+' center action missing');
        const cover=await target.locator('.task-row-cover').boundingBox(), button=await cancel.boundingBox();
        check(button.width===64&&button.height===64&&Math.abs(button.x+32-cover.x-cover.width/2)<1&&Math.abs(button.y+32-cover.y-cover.height/2)<1,id+' progress is not centered '+JSON.stringify({cover,button}));
        if(id!=='downloading') {
          const badge=target.locator('.task-feed-status');
          check(await badge.innerText()===(id==='queued'?'等待中':'等待解析'),id+' corner status wrong');
          const badgeBox=await badge.boundingBox();check(Math.abs(badgeBox.x-cover.x-8)<1&&Math.abs(badgeBox.y-cover.y-8)<1,id+' status not at top left');
          const progress=target.getByRole('progressbar',{name:'等待下载',exact:true});
          check(await progress.getAttribute('aria-valuenow')===null&&await progress.innerText()==='等待',id+' has a fake percentage');
        } else check(await target.getByRole('progressbar',{name:'下载进度',exact:true}).getAttribute('aria-valuenow')==='30','download progress changed');
        await cancel.hover();const hint=p.getByRole('tooltip',{name:'点击取消下载',exact:true});await hint.waitFor();
        const hintBox=await hint.boundingBox();check(hintBox.x>=0&&hintBox.y>=0&&hintBox.x+hintBox.width<=width&&hintBox.y+hintBox.height<=height,'tooltip outside viewport');
        await p.keyboard.press('Escape'); await cancel.focus(); await p.keyboard.press('Enter');
        const dialog=p.getByRole('alertdialog',{name:'取消当前任务？',exact:true});await dialog.waitFor();
        check(!await p.evaluate(id=>window.calls.some(c=>c.command==='cancel_task'&&c.args.taskId===id),id),'canceled without confirmation');
        await dialog.getByRole('button',{name:'返回',exact:true}).click();await dialog.waitFor({state:'hidden'});
        await p.waitForFunction(id=>document.activeElement?.closest('[data-task-id]')?.dataset.taskId===id,id);
        await cancel.click(); await dialog.getByRole('button',{name:'确认取消',exact:true}).click();
        await p.waitForFunction(id=>window.calls.some(c=>c.command==='cancel_task'&&c.args.taskId===id),id);
        await p.evaluate(()=>window.dispatchEvent(new Event('focus')));
        const canceling=target.getByRole('button',{name:'正在取消…',exact:true});await canceling.waitFor();
        check(await canceling.isDisabled()&&await canceling.locator('[data-slot=spinner]').count()===1,id+' canceling feedback missing');
      }
      check(await row('canceling').getByRole('button',{name:'正在取消…',exact:true}).isDisabled(),'initial canceling enabled');
      await p.getByRole('button',{name:'切换为列表',exact:true}).click();
      check(await row('queued').locator('.task-queue-position').count()===1,'list queue position changed');
      check(errors.length===0,errors.join('\n'));
      results.push({width,passed:true});
    } finally { await p.close(); }
  }
  return {passed:true,results};
}
