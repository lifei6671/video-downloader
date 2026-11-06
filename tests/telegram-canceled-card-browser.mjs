// Isolated IPC fixtures; no real session, download, upload, or user file access.
async page => {
  const origin = page.url().startsWith('http') ? page.url().split('/').slice(0,3).join('/') : 'http://127.0.0.1:1432';
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  for (const [width, height] of [[1440,900], [1280,800], [1024,680]]) {
    const p = await page.context().newPage();
    p.setDefaultTimeout(8000);
    const errors = [];
    p.on('pageerror', error => errors.push(error.message));
    await p.addInitScript(() => {
      localStorage.setItem('yingqu.task-view', 'feed');
      window.calls = [];
      const video = { platform:'telegram', resolver:'grammers', kind:'video', video_id:'12345_9', canonical_url:'https://t.me/c/12345/9', title:'保留真实视频标题', author:'知识分享频道', author_profile:{id:'12345',nickname:'知识分享频道',handle:'channel_name',kind:'channel',avatar_cached:false}, filesize:100, width:1920, height:1080 };
      const base = { revision:1, video, bytes:50, total_bytes:100, total_bytes_estimate:null, speed_bps:null, eta_seconds:null, path:null, created_at:1, completed_at:null, canceling:false, file_missing:false, download_requested:true, cover_cached:false, files:[], error:null, queue_order:null };
      window.tasks = [
        {...base, id:'canceled', status:'canceled'},
        {...base, id:'unknown', status:'canceled', video:{...video,video_id:'67890_1',author:'67890',author_profile:{id:'67890',nickname:null,handle:null,kind:'channel'}}},
        {...base, id:'failed', status:'failed', error:'NETWORK_ERROR', video:{...video,video_id:'99999_1',author_profile:{...video.author_profile,id:'99999'}}},
        {...base, id:'unparsed', status:'canceled', video:null, download_requested:false},
      ];
      const callbacks = new Map();
      const listeners = [];
      let next = 1;
      const emit = task => {
        for (const listener of listeners.filter(item => item.event === 'task_updated')) {
          callbacks.get(listener.handler)?.({event:'task_updated',id:listener.id,payload:structuredClone(task)});
        }
      };
      window.updateTask = (id, changes) => {
        const task = window.tasks.find(item => item.id === id);
        Object.assign(task, changes); task.revision++; emit(task);
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = {
        convertFileSrc:value=>value,
        transformCallback:callback=>{const id=next++; callbacks.set(id,callback); return id;},
        unregisterCallback:id=>callbacks.delete(id),
        invoke:async(command,args)=>{
          window.calls.push({command,args});
          if (command==='get_settings') return {download_directory:'C:/isolated',filename_template:'{content}',max_downloads:2,profiles:[]};
          if (command==='list_tasks') return structuredClone(window.tasks);
          if (command==='get_backup_settings') return null;
          if (['list_backup_spaces','list_backup_assets','list_backup_operations','list_video_transcripts'].includes(command)) return [];
          if (command==='check_runtime_dependencies') return {platform:'windows',directory:'C:/isolated',tools:[]};
          if (command==='plugin:event|listen') {const id=next++;listeners.push({...args,id});return id;}
          if (command==='retry_task' || command==='cancel_task') {
            const task=window.tasks.find(item=>item.id===args.taskId);
            task.status=command==='retry_task'?'queued':'canceled'; task.revision++; task.canceling=false;
            if (command==='retry_task') task.error=null;
            queueMicrotask(()=>emit(task));
            return command==='retry_task'?structuredClone(task):null;
          }
          return null;
        },
      };
    });
    try {
      await p.setViewportSize({width,height});
      await p.goto(origin+'/');
      await p.getByRole('button',{name:'下载任务',exact:true}).click();
      const canceled=p.locator('[data-task-id="canceled"]');
      const unknown=p.locator('[data-task-id="unknown"]');
      await canceled.waitFor();
      check(await canceled.locator('.task-feed-status').innerText()==='已取消','missing top-left canceled badge');
      check(await canceled.locator('.task-row-footer').count()===0,'canceled footer remains');
      check(await canceled.locator('.task-row-title').innerText()==='保留真实视频标题','video title was replaced with status');
      check(await canceled.locator('[data-author-info]').innerText()==='知识分享频道','channel title is not primary');
      check(await unknown.locator('[data-author-info]').innerText()==='Telegram 频道','legacy numeric ID is displayed as a title');
      const retry=canceled.getByRole('button',{name:'重试下载',exact:true});
      check(await retry.innerText()==='' && await retry.locator('svg.lucide-circle-x').count()===1,'canceled retry is not a cancellation icon');
      const box=await retry.boundingBox(),cover=await canceled.locator('.task-row-cover').boundingBox();
      check(box.y>=cover.y && box.y<cover.y+12 && box.x+box.width>cover.x+cover.width-12,'retry is not at cover top-right');
      await retry.hover();
      const tooltip=p.getByRole('tooltip',{name:'已取消，点击重试下载',exact:true});await tooltip.waitFor();
      const hint=await tooltip.boundingBox();
      check(hint.x>=0 && hint.y>=0 && hint.x+hint.width<=width && hint.y+hint.height<=height,'tooltip overflow');
      await p.screenshot({path:`.local/card-layout-unification/cards-${width}.png`});
      await p.mouse.move(0,0);await retry.focus();await p.keyboard.press('Enter');
      await p.waitForFunction(()=>window.tasks[0].status==='queued');
      await canceled.locator('.task-feed-status').filter({hasText:'等待中'}).waitFor();
      check(await p.evaluate(()=>window.calls.filter(call=>call.command==='retry_task').length)===1,'retry was duplicated');
      const cancel=canceled.getByRole('button',{name:'取消下载',exact:true});await cancel.click();
      const dialog=p.getByRole('alertdialog',{name:'取消当前任务？',exact:true});await dialog.waitFor();
      await dialog.getByRole('button',{name:'确认取消',exact:true}).click();
      await canceled.getByRole('button',{name:'重试下载',exact:true}).waitFor();
      check(await canceled.locator('.task-row-footer').count()===0,'manual cancel restored redundant footer');
      await canceled.getByRole('button',{name:'重试下载',exact:true}).click();
      await canceled.locator('.task-feed-status').filter({hasText:'等待中'}).waitFor();
      check(await p.evaluate(()=>window.calls.filter(call=>call.command==='retry_task').length)===2,'mouse retry must send exactly one additional request');
      check(await p.evaluate(()=>window.calls.filter(call=>call.command==='retry_task').every(call=>call.args.taskId==='canceled')),'retry changed task identity');
      await canceled.getByRole('button',{name:'取消下载',exact:true}).click();
      await dialog.getByRole('button',{name:'确认取消',exact:true}).click();
      await canceled.getByRole('button',{name:'重试下载',exact:true}).waitFor();
      const failed=p.locator('[data-task-id="failed"]');
      check(await failed.locator('.task-feed-status').innerText()==='下载失败','failed badge is not in the completed-badge position');
      check(await failed.locator('.task-row-footer').count()===0,'failed footer remains');
      check(await failed.getByRole('alert').evaluate(el=>el.parentElement.classList.contains('task-card-feedback')),'failure reason is not inside the height-independent feedback area');
      const failedRetry=failed.getByRole('button',{name:'重试下载',exact:true});
      check(await failedRetry.innerText()==='' && await failedRetry.locator('svg.lucide-rotate-ccw').count()===1,'failed retry is not an icon');
      const retryBox=await failedRetry.boundingBox(), failedCover=await failed.locator('.task-row-cover').boundingBox();
      check(retryBox.y>=failedCover.y && retryBox.y<failedCover.y+12 && retryBox.x+retryBox.width>failedCover.x+failedCover.width-12,'failed retry is not top-right');
      await failedRetry.hover();
      const failedHint=p.getByRole('tooltip',{name:'下载失败，点击重试下载',exact:true});await failedHint.waitFor();
      const hintBounds=await failedHint.boundingBox();
      check(hintBounds.x>=0 && hintBounds.y>=0 && hintBounds.x+hintBounds.width<=width && hintBounds.y+hintBounds.height<=height,'failed tooltip overflow');
      await p.screenshot({path:`.local/card-layout-unification/failed-${width}.png`});
      await p.mouse.move(0,0);await failedRetry.focus();await p.keyboard.press('Enter');
      await failed.locator('.task-feed-status').filter({hasText:'等待中'}).waitFor();
      check(await p.evaluate(()=>window.calls.filter(call=>call.command==='retry_task'&&call.args.taskId==='failed').length)===1,'failed retry lost its identity or was duplicated');
      const retryHeight = (await failed.boundingBox()).height;
      for (const [changes, label] of [
        [{status:'queued'}, '等待中'],
        [{status:'parsing'}, '解析中'],
        [{status:'downloading',merging:false}, '下载中'],
        [{status:'downloading',merging:true}, '合并中'],
        [{status:'canceled',merging:false}, '已取消'],
      ]) {
        await p.evaluate(({changes})=>window.updateTask('failed',changes),{changes});
        await failed.locator('.task-feed-status').filter({hasText:label}).waitFor();
        check(await failed.locator('.task-row-footer, .task-actions, .task-row-controls').count()===0,'retry restored bottom actions in '+label);
        check(Math.abs((await failed.boundingBox()).height-retryHeight)<1,'retry changed card height in '+label);
        for (const button of await failed.getByRole('button').all()) {
          if (!await button.isVisible()) continue;
          const bounds=await button.boundingBox(),coverBounds=await failed.locator('.task-row-cover').boundingBox();
          if (['取消','取消下载','取消解析','正在取消…'].includes(await button.getAttribute('aria-label'))) check(bounds.y+bounds.height<=coverBounds.y+coverBounds.height+1,'cancel button escaped cover in '+label);
        }
      }
      await p.getByRole('button',{name:'切换为列表',exact:true}).click();
      await p.evaluate(()=>window.updateTask('failed',{status:'queued'}));
      check(await failed.getByRole('button',{name:'取消',exact:true}).isVisible(),'list cancellation lost');
      check(await canceled.getByRole('button',{name:'重试',exact:true}).isVisible(),'list task lost original retry');
      await canceled.getByRole('button',{name:'查看知识分享频道的作品',exact:true}).click();
      check(await p.locator('[data-task-id="failed"]').count()===0,'same title merged different channel IDs');
      check(await canceled.isVisible(),'channel search lost matching ID');
      check(!await p.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'page overflow');
      check(errors.length===0,errors.join('\n'));
      results.push({width,height,passed:true});
    } finally { await p.close(); }
  }
  return {passed:true,results,checks:['real channel title','legacy fallback','canceled and failed corner controls','persistent failure reason','tooltip bounds','keyboard retry once','manual cancellation','list controls','stable channel search identity']};
}
