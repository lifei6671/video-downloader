// Browser-only IPC fixtures; preference storage belongs to the isolated profile.
async page => {
  page.setDefaultTimeout(8000);
  const base=page.url().split('/').slice(0,3).join('/');
  const key='yingqu.task-view';
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const list=()=>page.getByRole('list',{name:'媒体任务列表',exact:true});
  const tasks=()=>page.getByRole('button',{name:'下载任务',exact:true}).click();
  const reload=async()=>{await page.reload();await tasks();await list().waitFor()};
  await page.addInitScript(()=>{
    window.renderedTaskViews=[];
    new MutationObserver(()=>{
      const list=document.querySelector('.task-list');
      if(list)window.renderedTaskViews.push(list.dataset.view);
    }).observe(document,{childList:true,subtree:true,attributes:true,attributeFilter:['data-view']});
  });
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height});
    await page.evaluate(key=>localStorage.removeItem(key),key);
    await page.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');await tasks();
    check(await list().getAttribute('data-view')==='list','first use is not list');
    await page.getByRole('button',{name:'切换为信息流',exact:true}).click();
    check(await page.evaluate(key=>localStorage.getItem(key),key)==='feed','feed selection was not saved');
    await page.getByRole('tab',{name:'YouTube',exact:true}).click();
    await page.getByRole('button',{name:'系统设置',exact:true}).click();await tasks();
    check(await list().getAttribute('data-view')==='feed','navigation lost selected view');
    await reload();
    check(await list().getAttribute('data-view')==='feed','new app instance lost feed selection');
    check(await page.evaluate(()=>window.renderedTaskViews.length>0&&window.renderedTaskViews.every(v=>v==='feed')),'startup briefly rendered list before restoring feed');
    check(await page.getByRole('button',{name:'切换为列表',exact:true}).getAttribute('aria-pressed')==='true','restored icon/toggle does not match feed');
    await page.getByRole('button',{name:'切换为列表',exact:true}).click();await reload();
    check(await list().getAttribute('data-view')==='list' && await page.evaluate(key=>localStorage.getItem(key),key)==='list','new list choice did not replace persisted feed');
    check(await page.evaluate(()=>window.renderedTaskViews.every(v=>v==='list')),'restored list flashed another view');
    results.push(`${width}: first-use list, persist both choices, navigation/reload, correct icon, no startup view flash`);
  }
  await page.getByRole('button',{name:'切换为信息流',exact:true}).click();
  const other=await page.context().newPage();
  await other.goto(base+'/tests/fixtures/background-download.html?scenario=visual-list');
  await other.getByRole('button',{name:'下载任务',exact:true}).click();
  check(await other.getByRole('list',{name:'媒体任务列表',exact:true}).getAttribute('data-view')==='feed','new page in same profile did not restore preference');
  await other.close();
  for(const value of ['unknown','',null]){
    await page.evaluate(({key,value})=>value===null?localStorage.removeItem(key):localStorage.setItem(key,value),{key,value});
    await reload();check(await list().getAttribute('data-view')==='list','invalid/missing preference did not default to list');
  }
  await page.evaluate(()=>{
    const original=Storage.prototype.setItem;
    Storage.prototype.setItem=function(key,value){if(key==='yingqu.task-view')throw new DOMException('Preference write failed','QuotaExceededError');return original.call(this,key,value)};
  });
  await page.getByRole('button',{name:'切换为信息流',exact:true}).click();
  await page.locator('[data-sonner-toast][data-type="error"]').filter({hasText:'操作失败'}).waitFor();
  check(await list().getAttribute('data-view')==='list','failed persistence pretended to save a new view');
  results.push('same-profile new page restores preference; invalid/missing values use list; storage failure uses existing error feedback');
  return {passed:results.length,results};
}
