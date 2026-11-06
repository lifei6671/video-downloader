// Production renderer with synthetic IPC. No real downloads or credentials.
async page => {
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) for(const view of ['list','feed']) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);
    await p.setViewportSize({width,height});
    const errors=[];p.on('pageerror',e=>errors.push(e.message));
    await p.addInitScript(view=>{
      localStorage.setItem('yingqu.task-view',view);
      const callbacks=new Map(),listeners=new Map();let sequence=0;
      window.navTasks=Array.from({length:200},(_,i)=>({id:`task-${i}`,revision:1,status:'completed',video:{platform:'youtube',resolver:'yt-dlp',kind:'video',video_id:String(i),title:`性能测试 ${i}`,author:'测试作者',width:1920,height:1080,duration_ms:2000},download_requested:true,bytes:100,path:`C:/fixture/${i}.mp4`,file_missing:false,created_at:200-i,completed_at:200-i,cover_cached:false,files:[]}));
      window.navTasks.push({...structuredClone(window.navTasks[0]),id:'preview',status:'ready',created_at:300,completed_at:null,path:null,download_requested:false,video:{...window.navTasks[0].video,video_id:'preview',canonical_url:'https://www.youtube.com/watch?v=fixture',title:'待下载预览',formats:[{format_id:'hd',container:'mp4',width:1920,height:1080}]}});
      window.navEmit=(event,payload)=>listeners.get(event)?.({payload});
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:fn=>{callbacks.set(++sequence,fn);return sequence;},unregisterCallback(){},invoke:async(command,args)=>{
        if(command==='plugin:event|listen'){listeners.set(args.event,callbacks.get(args.handler));return sequence;}
        if(command==='list_tasks')return structuredClone(window.navTasks);
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(['list_backup_assets','list_backup_operations','list_video_transcripts','list_backup_spaces'].includes(command))return [];
        if(command==='get_backup_sync_enabled')return false;
        return null;
      }};
    },view);
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      const nav=name=>p.locator('.app-navigation-item').filter({hasText:name});
      if(await p.locator('[data-task-row]').count())throw Error('Task page mounted before first visit');
      await nav('下载任务').click();
      const first=p.locator('[data-task-row="task-0"]');await first.waitFor();
      await p.evaluate(()=>{
        window.originalRow=document.querySelector('[data-task-row="task-0"]');
        const panel=window.originalRow.closest('[role="tabpanel"]');
        panel.scrollTop=200;window.taskScroll=panel.scrollTop;
      });
      await nav('关于软件').focus();await p.keyboard.press('Enter');await first.waitFor({state:'hidden'});
      await p.evaluate(()=>{
        const task=window.navTasks.find(task=>task.id==='task-0');
        task.video.title='后台更新后的标题';task.revision++;
        window.navEmit('task_updated',structuredClone(task));
      });
      await p.waitForTimeout(100);
      if(await first.textContent().then(text=>text.includes('后台更新后的标题')))throw Error('Hidden task page reconciled background updates');
      await nav('下载任务').click();await first.getByText('后台更新后的标题',{exact:true}).waitFor();
      if(!await p.evaluate(()=>window.originalRow===document.querySelector('[data-task-row="task-0"]')))throw Error('Navigation remounted task cards');
      if(!await p.evaluate(()=>window.taskScroll>0&&window.originalRow.closest('[role="tabpanel"]').scrollTop===window.taskScroll))throw Error('Task page scroll was not retained');
      await first.click({button:'right'});await p.getByRole('menu').waitFor();await p.keyboard.press('Escape');
      await nav('系统设置').click();await first.waitFor({state:'hidden'});
      await p.evaluate(()=>{
        window.navTasks=window.navTasks.filter(task=>task.id!=='task-0');
        window.navEmit('task_removed','task-0');
      });
      await nav('下载任务').click();await first.waitFor({state:'detached'});
      await p.locator('[data-task-row="task-1"]').waitFor();
      await nav('关于软件').click();
      const scrolled=await p.locator('[data-slot="page-content"]').evaluate(el=>{el.scrollTop=el.scrollHeight;return el.scrollTop;});
      await nav('下载作品').click();await p.getByText('待下载预览',{exact:true}).waitFor();
      if(await p.locator('[data-slot="page-content"]').evaluate(el=>el.scrollTop)!==0)throw Error(`Previous page scroll leaked into download preview (${scrolled})`);
      if(!await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth))throw Error('Horizontal overflow');
      if(errors.length)throw Error(errors.join('\n'));
      results.push({width,view,passed:true});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
