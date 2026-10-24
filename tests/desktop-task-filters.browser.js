// Isolated IPC fixtures only; filters and selection must never mutate task data.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const rows = () => page.locator('.task-row');
  const ids = () => rows().evaluateAll(nodes => nodes.map(node => node.dataset.taskId).sort());
  const states = () => page.getByRole('radiogroup', {name:'按状态筛选下载任务'});
  const platform = name => page.getByRole('tablist', {name:'按平台筛选下载任务'}).getByRole('tab', {name, exact:true});
  const choose = name => states().getByRole('radio', {name, exact:true}).click();
  const results = [];
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/task-platforms.html');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await rows().first().waitFor();
    const counts = () => states().locator('.task-filter-count').allTextContents();
    check((await counts()).join(',')==='6,1,2,1,2', 'counts include preview or omit submitted tasks');
    const before = await page.evaluate(()=>JSON.stringify(window.taskPlatformsTest.snapshot()));
    for (const [name,expected] of [['下载中',['douyin-live']],['等待中',['telegram-wait','tiktok-wait']],['已完成',['youtube-done']],['失败',['unknown-failed','youtube-failed']]]) {
      await choose(name);
      check(JSON.stringify(await ids())===JSON.stringify(expected), 'status filter '+name);
    }
    await platform('YouTube').click();
    check((await ids()).join(',')==='youtube-failed','status/platform intersection');
    await page.getByRole('textbox',{name:'搜索作品标题或作者'}).fill('done');
    await page.getByText('没有匹配的下载任务',{exact:true}).waitFor();
    await choose('已完成');
    check((await ids()).join(',')==='youtube-done','status/platform/search intersection');
    await page.getByRole('textbox',{name:'搜索作品标题或作者'}).fill('');
    await choose('下载中');
    await page.evaluate(()=>window.taskPlatformsTest.push('youtube-arrived','youtube'));
    await page.locator('[data-task-id="youtube-arrived"]').waitFor();
    check((await counts()).join(',')==='3,1,0,1,1', 'counts did not update with platform and task event');
    check((await ids()).join(',')==='youtube-arrived','live event bypassed filter');
    await choose('全部'); await platform('全部').click();
    const first = rows().first();
    await first.locator('.task-row-status').click();
    check(await first.getAttribute('data-selected')==='true','click does not select row');
    const second = rows().nth(1);
    await second.getByRole('button',{name:'查看封面',exact:true}).focus();
    check(await second.getAttribute('data-selected')==='true' && await page.locator('.task-row[data-selected="true"]').count()===1,'keyboard selection not exclusive');
    await page.waitForFunction(()=>{const n=document.querySelector('.task-row[data-selected="true"]'); return n && getComputedStyle(n).backgroundColor==='rgb(237, 242, 254)';});
    check(await second.evaluate(n=>getComputedStyle(n).backgroundColor==='rgb(237, 242, 254)'), 'selected background is not Radix indigo-3');
    check(await page.evaluate(()=>window.taskPlatformsTest.calls.length)===0,'selection caused IPC');
    check(await page.evaluate(()=>JSON.stringify(window.taskPlatformsTest.snapshot().filter(t=>t.id!=='youtube-arrived')))===before,'filters mutated task data');
    await states().getByRole('radio',{name:'全部',exact:true}).focus();
    await page.keyboard.press('ArrowRight');
    await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='下载中');
    await page.keyboard.press('Space');
    check(await states().getByRole('radio',{name:'下载中',exact:true}).getAttribute('aria-checked')==='true','state keyboard navigation');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    results.push(width+': status/platform/search, live events, selection, keyboard, no mutation');

    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await choose('已完成');
    const video = page.locator('[data-task-id="visual-3"]');
    await video.locator('.media-cover-image').evaluate(n=>n.decode());
    await video.locator('.task-row-status').click();
    await page.screenshot({path:`output/task-ui-v4/completed-${width}.png`,animations:'disabled'});
    await video.click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menu').waitFor();
    await page.screenshot({path:`output/task-ui-v4/menu-${width}.png`,animations:'disabled'});
    await page.keyboard.press('Escape');
  }
  return {passed:results.length,cases:results};
}
