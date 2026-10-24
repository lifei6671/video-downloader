// Isolated fixture: real components and browser animations, no user data mutations.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list-completed');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    const row=page.locator('[data-task-id="visual-3"]');
    await row.locator('.media-cover-image').evaluate(n=>n.decode());
    await row.hover({position:{x:200,y:30}});
    await page.waitForFunction(()=>getComputedStyle(document.querySelector('[data-task-id="visual-3"]')).backgroundColor==='rgb(249, 249, 251)');
    check(await page.locator('[data-task-row]').evaluateAll(nodes=>nodes.every(n=>getComputedStyle(n).borderTopWidth==='0px'&&getComputedStyle(n).borderBottomWidth==='0px')), 'row separators remain');
    check(await row.evaluate(n=>getComputedStyle(n).borderRadius==='8px'),'hover is not rounded');
    const play=row.getByRole('button',{name:'播放视频',exact:true});
    check(await play.innerText()==='' && await play.evaluate(n=>getComputedStyle(n).borderWidth==='0px'), 'play text or border remains');
    check(await page.getByRole('button',{name:'更多操作',exact:true}).count()===0,'more icon remains in task list');
    await page.screenshot({path:`output/task-ui-v6/hover-${width}.png`,animations:'disabled'});
    await row.evaluate(node => {
      node.addEventListener('contextmenu', () => {
        const start = performance.now();
        const observer = new MutationObserver(() => {
          if (!document.querySelector('.task-context-menu[data-state="open"]')) return;
          observer.disconnect();
          requestAnimationFrame(() => { window.taskMenuFirstFrame = performance.now() - start; });
        });
        observer.observe(document.body, { childList: true, subtree: true });
      }, { once: true });
    });
    await row.click({button:'right',position:{x:200,y:30}});
    const menu=page.getByRole('menu',{name:'任务操作',exact:true});
    await menu.waitFor();
    check(await menu.evaluate(n=>getComputedStyle(n).animationName)==='task-menu-in','opening animation missing');
    check(await menu.evaluate(n=>getComputedStyle(n).animationDuration)==='0.08s','menu entrance is too slow');
    check(await menu.evaluate(n=>getComputedStyle(n).width==='236px' && getComputedStyle(n).borderRadius==='10px'),'menu proportions');
    await page.waitForFunction(() => window.taskMenuFirstFrame !== undefined);
    const firstFrame = Math.round(await page.evaluate(() => window.taskMenuFirstFrame));
    const box=await menu.boundingBox();
    check(box.x>=0&&box.x+box.width<=width&&box.y>=0&&box.y+box.height<=height,'context menu outside window');
    await page.screenshot({path:`output/task-ui-v6/context-${width}.png`,animations:'disabled'});
    // Extend only the exit animation in the fixture to observe the closed state.
    await page.addStyleTag({content:'.task-context-menu[data-state="closed"] {animation-duration: 600ms !important}'});
    await page.keyboard.press('Escape');
    const exiting=page.locator('.task-context-menu[data-state="closed"]');
    await exiting.waitFor({state:'attached'});
    check(await exiting.evaluate(n=>getComputedStyle(n).animationName==='task-menu-out'&&getComputedStyle(n).pointerEvents==='none'),'exit does not animate or intercepts clicks');
    await exiting.waitFor({state:'detached'});
    await page.waitForFunction(()=>document.activeElement?.classList.contains('task-context-target'));
    await page.keyboard.press('Shift+F10');
    await menu.waitFor();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Escape');
    await exiting.waitFor({state:'detached'});
    // Nested cover menu must keep its own operations without opening both menus.
    await row.getByRole('button',{name:'查看封面',exact:true}).click({button:'right'});
    await page.getByRole('menuitem',{name:/复制封面/}).waitFor();
    check(await page.getByRole('menu',{name:'任务操作',exact:true}).count()===0,'row menu overrides cover menu');
    await page.keyboard.press('Escape');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    results.push(`${width}: first menu frame ${firstFrame}ms; rounded hover, no separators/more icon, icon-only play, context menu position, enter/exit animation, keyboard/focus, nested cover menu`);
  }
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.locator('[data-task-id="visual-3"]').click({button:'right',position:{x:200,y:30}});
  const menu=page.getByRole('menu',{name:'任务操作',exact:true});
  await menu.waitFor();
  check(await menu.evaluate(n=>getComputedStyle(n).animationName)==='none','reduced motion ignored');
  await page.keyboard.press('Escape');
  await menu.waitFor({state:'detached'});
  await page.emulateMedia({reducedMotion:'no-preference'});
  results.push('reduced motion disables menu animation');
  await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.evaluate(()=>window.backgroundTest.progress({video:null,telegram_link:null,status:'parsing'}));
  const unavailable=page.locator('[data-task-id="visual-1"]').locator('..');
  await page.waitForFunction(()=>document.querySelector('[data-task-id="visual-1"]')?.parentElement.hasAttribute('data-disabled'));
  check(await unavailable.evaluate(n=>!n.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}))), 'unavailable task exposes browser context menu');
  check(await page.getByRole('menu').count()===0,'unavailable task opens empty menu');
  results.push('no actions: suppresses native browser menu without opening empty task menu');
  return {passed:results.length,cases:results};
}
