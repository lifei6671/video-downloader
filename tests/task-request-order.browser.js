// Production App with isolated IPC; no files, sessions or network transfers.
async page => {
  const base=page.url().split('/').slice(0,3).join('/'),results=[];
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const ids=()=>page.locator('[data-task-id]').evaluateAll(rows=>rows.map(row=>row.dataset.taskId));
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) for(const view of ['feed','list']){
    await page.setViewportSize({width,height});
    await page.evaluate(view=>localStorage.setItem('yingqu.task-view',view),view);
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=task-request-order');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.locator('[data-task-id="interrupted"]').waitFor();
    const initial=['parsed-first','parsed-last','middle','legacy','interrupted'];
    check(JSON.stringify(await ids())===JSON.stringify(initial),view+': not sorted by first database insertion ID');
    check(await page.locator('[data-task-id="canceled"]').count()===0,view+': canceled history remained visible');
    check((await page.getByRole('radio',{name:'全部',exact:true}).innerText()).replace(/\s/g,'')==='全部5',view+': canceled history inflated counters');
    await page.evaluate(()=>window.backgroundTest.progress('parsed-first',{status:'downloading',bytes:10}));
    await page.waitForTimeout(150);
    check(JSON.stringify(await ids())===JSON.stringify(initial),view+': leaving the queue jumped the row');
    await page.evaluate(()=>window.backgroundTest.complete('parsed-first'));
    await page.waitForTimeout(150);
    check(JSON.stringify(await ids())===JSON.stringify(initial),view+': completion jumped the row');
    // A retry updates request time and queue position without changing insertion order.
    const legacy=page.locator('[data-task-id="legacy"]');
    await legacy.getByRole('button',{name:view==='feed'?'重试下载':'重试',exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.snapshot().find(task=>task.id==='legacy')?.download_requested_at===9000);
    check(JSON.stringify(await ids())===JSON.stringify(initial),view+': retry moved the original database row');
    await page.reload();
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.locator('[data-task-id="interrupted"]').waitFor();
    check(JSON.stringify(await ids())===JSON.stringify(initial),view+': reopening changed insertion order');
    await page.evaluate(()=>window.backgroundTest.insert());
    await page.locator('[data-task-id="new-insertion"]').waitFor();
    check(JSON.stringify(await ids())===JSON.stringify(['new-insertion',...initial]),view+': new database row did not appear first');
    await page.evaluate(()=>window.backgroundTest.progress('parsed-last',{status:'canceled',canceling:false}));
    await page.locator('[data-task-id="parsed-last"]').waitFor({state:'detached'});
    check(await page.locator('[data-task-id="interrupted"]').count()===1,view+': application interruption was treated as explicit cancellation');
    check(await page.locator('[data-sonner-toast][data-type="error"]').count()===0,view+': cancellation displayed an error Toast');
    results.push(view+'/'+width+': insertion IDs override creation/request time; queue/progress/completion/retry/reopen preserve order; canceled rows and counts removed; interruption retained');
  }
  return {passed:results.length,results};
}
