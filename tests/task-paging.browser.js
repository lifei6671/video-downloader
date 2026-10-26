// Browser-only IPC/clipboard fixtures. No platform, player or filesystem actions.
async page => {
  page.setDefaultTimeout(8000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const row=id=>page.locator('[data-task-id="'+id+'"]');
  const rows=()=>page.locator('[data-task-id]');
  const panel=()=>page.getByRole('tabpanel');
  const count=async n=>page.waitForFunction(n=>document.querySelectorAll('[data-task-id]').length===n,n);
  const load=async(feed=false)=>{
    await page.evaluate(()=>localStorage.removeItem('yingqu.task-view'));
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list-paging');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await count(30);
    if(feed)await page.getByRole('button',{name:'切换为信息流',exact:true}).click();
  };
  const approach=async()=>panel().evaluate(node=>{
    node.scrollTop=node.scrollHeight-node.clientHeight-350;
    return {top:node.scrollTop,gap:node.scrollHeight-node.clientHeight-node.scrollTop};
  });
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]])for(const feed of [false,true]){
    await page.setViewportSize({width,height});await load(feed);
    check(await rows().count()===30,'initial page is not 30');
    check(await row('visual-0').count()===0,'unsubmitted preview leaked into task list');
    check(await page.getByRole('radio',{name:'全部',exact:true}).locator('.task-filter-count').textContent()==='74','status count was paginated');
    await page.evaluate(()=>window.firstPagingRow=document.querySelector('[data-task-id]'));
    const start=await approach();
    check(start.gap>=349,'prefetch test already reached the bottom');
    await page.locator('.task-load-more[aria-busy="true"]').waitFor({state:'attached'});
    check(await rows().count()===30,'loading feedback skipped');
    check(await page.locator('.task-load-more [data-slot="spinner"]').evaluate(n=>getComputedStyle(n).animationName!=='none'),'loading feedback is not animated');
    await count(40);
    check(Math.abs(await panel().evaluate(n=>n.scrollTop)-start.top)<2,'append jumped scroll position');
    check(await page.evaluate(()=>window.firstPagingRow===document.querySelector('[data-task-id]')),'append remounted old rows');
    await page.evaluate(()=>window.backgroundTest.progress({bytes:90000000}));
    await count(40);
    await approach();await count(50);
    check(await rows().count()===50,'second page did not append exactly ten');
    await page.getByRole('radio',{name:'已完成',exact:true}).click();await count(30);
    check(await panel().evaluate(n=>n.scrollTop)===0,'status filter did not reset scroll');
    check(await rows().evaluateAll(nodes=>nodes.every(n=>n.dataset.status==='completed')),'filter leaked other statuses');
    await page.getByRole('tab',{name:'TikTok',exact:true}).click();
    await page.getByText('没有匹配的下载任务',{exact:true}).waitFor();
    check(await rows().count()===0 && await page.locator('.task-load-more').count()===0,'empty filter has stale pages/loading');
    await load(feed);
    for(const n of [40,50,60,70,74]){await approach();await count(n)}
    check(await page.locator('.task-load-more').count()===0,'terminal page still offers loading');
    check(new Set(await rows().evaluateAll(nodes=>nodes.map(n=>n.dataset.taskId))).size===74,'duplicate tasks across pages');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    results.push(`${width} ${feed?'feed':'list'}: 30/+10, 350px ahead, animated feedback, stable identity/scroll, live update, filter reset, final partial page`);
  }
  // Changing filters while a prefetch is pending cancels the old batch.
  await load();await approach();
  await page.locator('.task-load-more[aria-busy="true"]').waitFor({state:'attached'});
  await page.getByRole('radio',{name:'失败',exact:true}).click();await count(1);
  await page.waitForTimeout(250);
  check(await rows().count()===1 && await row('visual-4').count()===1,'old prefetch contaminated new filter');
  await page.getByRole('radio',{name:'全部',exact:true}).click();await count(30);
  await page.emulateMedia({reducedMotion:'reduce'});await approach();
  await page.locator('.task-load-more[aria-busy="true"]').waitFor({state:'attached'});
  check(await page.locator('.task-load-more [data-slot="spinner"]').evaluate(n=>getComputedStyle(n).animationName==='none'),'reduced motion spinner still animates');
  await count(40);await page.emulateMedia({reducedMotion:'no-preference'});
  results.push('pending batch cleanup, new query starts at 30, reduced motion keeps textual loading feedback');
  return {passed:results.length,results};
}
