// Existing Playwright CLI run-code --filename, with npm run dev.
async page=>{
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const tabs=()=>page.getByRole('tablist',{name:'按平台筛选下载任务'});
  const panel=()=>page.getByRole('tabpanel');
  const titles=()=>panel().locator('[data-slot="card-title"]').allTextContents();
  const expected={全部:['douyin-live','tiktok-wait','youtube-done','youtube-failed','Telegram https://t.me/c/100/9','下载任务 unknown-failed'],抖音:['douyin-live'],TikTok:['tiktok-wait'],YouTube:['youtube-done','youtube-failed'],Telegram:['Telegram https://t.me/c/100/9']};
  const load=async(query='')=>{await page.goto(base+'/tests/fixtures/task-platforms.html'+query);await page.getByRole('button',{name:'下载任务',exact:true}).click();await tabs().waitFor()};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load();
    check(await tabs().getByRole('tab').allTextContents().then(x=>x.join(','))==='全部,抖音,TikTok,YouTube,Telegram','tab order incorrect');
    check(await tabs().getByRole('tab',{name:'全部',exact:true}).getAttribute('aria-selected')==='true','default is not all');
    const before=await page.evaluate(()=>window.taskPlatformsTest.snapshot());
    for(const [name,wanted] of Object.entries(expected)) {
      await tabs().getByRole('tab',{name,exact:true}).click();
      const actual=await titles();check(actual.length===wanted.length && wanted.every(t=>actual.includes(t)),name+' leaked or lost task');
      check(await panel().getAttribute('aria-labelledby')===await tabs().getByRole('tab',{name,exact:true}).getAttribute('id'),'tab/panel accessibility linkage wrong');
    }
    check((await panel().textContent()).includes('队列第 2 位'),'filtered queue position changed');
    check(JSON.stringify(before)===await page.evaluate(()=>JSON.stringify(window.taskPlatformsTest.snapshot())),'filter mutated tasks');
    const positions=await tabs().getByRole('tab').evaluateAll(items=>items.map(i=>i.getBoundingClientRect().y));
    check(Math.max(...positions)-Math.min(...positions)<2,'tabs are not a horizontal row');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal viewport overflow');
    await tabs().getByRole('tab',{name:'全部',exact:true}).focus();await page.keyboard.press('ArrowRight');
    await tabs().getByRole('tab',{name:'抖音',exact:true,selected:true}).waitFor();
    await page.screenshot({path:'.local/task-platforms-'+width+'.png',animations:'disabled'});
    results.push(width+': default/order/platform isolation/unknown/Telegram queue/keyboard/layout');
  }
  await load();await tabs().getByRole('tab',{name:'YouTube',exact:true}).click();
  const search=page.getByRole('textbox',{name:'搜索作品标题或作者'});
  await search.fill('done');check((await titles()).join(',')==='youtube-done','search not intersected with platform');
  await search.fill('douyin');await page.getByText('没有匹配的下载任务',{exact:true}).waitFor();
  await tabs().getByRole('tab',{name:'抖音',exact:true}).click();check((await titles()).join(',')==='douyin-live','search lost across tabs');
  await search.fill('');await tabs().getByRole('tab',{name:'YouTube',exact:true}).click();
  await page.evaluate(()=>{window.taskPlatformsTest.push('douyin-new','douyin');window.taskPlatformsTest.push('youtube-new','youtube')});
  await panel().getByText('youtube-new',{exact:true}).waitFor();
  check(!(await titles()).includes('douyin-new'),'event bypassed filter');
  const completed=panel().locator('[data-slot="card"]').filter({has:page.getByText('youtube-done',{exact:true})});
  await completed.click({button:'right',position:{x:200,y:30}});
  await page.getByRole('menuitem',{name:'打开文件夹',exact:true}).click();
  check(await page.evaluate(()=>window.taskPlatformsTest.calls.some(c=>c.type==='reveal' && c.id==='youtube-done')),'filtered action targeted wrong task');
  await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'下载任务',exact:true}).click();
  check(await tabs().getByRole('tab',{name:'YouTube',exact:true}).getAttribute('aria-selected')==='true','navigation lost filter');
  results.push('search intersection/live events/filtered action/navigation persistence');
  await load('?empty=telegram');await tabs().getByRole('tab',{name:'Telegram',exact:true}).click();
  await page.getByText('该平台暂无下载任务',{exact:true}).waitFor();
  await tabs().getByRole('tab',{name:'全部',exact:true}).click();check((await titles()).length===5,'all did not restore tasks');
  results.push('platform empty state and return to all');
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await load('?long=1');
    const content=page.locator('[data-slot="page-content"]');
    const heading=page.getByRole('heading',{name:'下载任务',exact:true});
    const beforeTabs=await tabs().boundingBox();const beforeHeading=await heading.boundingBox();
    const cards=panel().locator('[data-slot="card"]');
    const firstHeight=(await cards.first().boundingBox()).height;
    check(firstHeight>=70,'long-list cards collapsed');
    check(await panel().evaluate(node=>node.scrollHeight>node.clientHeight+500),'task list does not scroll');
    const box=await panel().boundingBox();
    await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.wheel(0,600);
    await page.waitForFunction(()=>document.querySelector('[role="tabpanel"]').scrollTop>200);
    const afterTabs=await tabs().boundingBox();const afterHeading=await heading.boundingBox();
    check(Math.abs(afterTabs.y-beforeTabs.y)<1 && Math.abs(afterHeading.y-beforeHeading.y)<1,'tab/header moved with list');
    check(await content.evaluate(node=>node.scrollTop===0 && node.scrollHeight<=node.clientHeight+1),'outer content also scrolls');
    check(await tabs().getByRole('tab',{name:'全部',exact:true}).evaluate(node=>{
      const box=node.getBoundingClientRect();return node.contains(document.elementFromPoint(box.x+box.width/2,box.y+box.height/2));
    }),'list painted over tabs');
    await panel().focus();await page.keyboard.press('Control+End');
    await page.waitForFunction(()=>{const n=document.querySelector('[role="tabpanel"]');return n.scrollHeight-n.clientHeight-n.scrollTop<2});
    const last=await cards.last().boundingBox();const viewport=await panel().boundingBox();
    check(last.y>=viewport.y && last.y+last.height<=viewport.y+viewport.height+1,'last task is unreachable');
    check(Math.abs((await cards.first().boundingBox()).height-firstHeight)<1,'scroll changes card height');
    check(Math.abs((await tabs().boundingBox()).y-beforeTabs.y)<1,'keyboard scroll moves tabs');
    await page.screenshot({path:'.local/task-tabs-fixed-'+width+'.png',animations:'disabled'});
    await tabs().getByRole('tab',{name:'抖音',exact:true}).click();
    check((await titles()).length===51,'fixed tab cannot filter long list');
    await tabs().getByRole('tab',{name:'TikTok',exact:true}).click();
    check((await titles()).join(',')==='tiktok-wait','short filter after long list failed');
    check(await panel().evaluate(node=>node.scrollTop===0),'short filter retained invalid scroll offset');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'fixed layout overflows viewport');
    results.push(width+': long-list wheel/keyboard scroll keeps header and tabs fixed; filtering and last row accessible');
  }
  return {passed:results.length,cases:results};
}
