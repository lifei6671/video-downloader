// Run against a production browser fixture or the native WebView over loopback CDP.
// Only aggregates are returned: no task content, credentials, or IPC arguments.
async page => {
  await page.bringToFront();
  await page.evaluate(() => {
    window.__menuProbe?.stop();
    const longTasks = [];
    const observer = new PerformanceObserver(list => longTasks.push(...list.getEntries().map(e=>e.duration)));
    observer.observe({type:'longtask',buffered:false});
    window.__menuProbe={longTasks,stop:()=>observer.disconnect()};
  });
  const session = await page.context().newCDPSession(page);
  await session.send('Profiler.enable');
  await session.send('Profiler.start');
  const results=[];
  try {
    for(let round=0;round<8;round++) for(const name of ['下载任务','系统设置','关于软件','下载作品']) {
      const result=await page.evaluate(async name=>{
        const button=[...document.querySelectorAll('.app-navigation-item')].find(el=>el.textContent.trim()===name);
        if(!button)throw Error(`Missing navigation: ${name}`);
        const start=performance.now();
        button.click();
        await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
        return {name,ms:performance.now()-start,title:document.querySelector('.app-page-header h1')?.textContent,rows:document.querySelectorAll('[data-task-row], [data-asset-id]').length,nodes:document.querySelectorAll('*').length};
      },name);
      if(result.title!==name)throw Error(`Navigation did not render ${name}`);
      results.push({...result,round});
      await page.waitForTimeout(180);
    }
    const {profile}=await session.send('Profiler.stop');
    const sampleCounts=new Map();for(const id of profile.samples??[])sampleCounts.set(id,(sampleCounts.get(id)??0)+1);
    const hot=profile.nodes.map(n=>({fn:n.callFrame.functionName,url:n.callFrame.url.startsWith('http')?n.callFrame.url.split('/').pop():'native',line:n.callFrame.lineNumber,samples:sampleCounts.get(n.id)??0})).sort((a,b)=>b.samples-a.samples).slice(0,18);
    const probes=await page.evaluate(()=>({longTasks:window.__menuProbe.longTasks}));
    const calls=await page.evaluate(async()=>{
      const calls=[];
      for(const command of ['list_tasks','list_backup_operations']) for(let i=0;i<3;i++) {
        const start=performance.now();
        const result=await window.__TAURI_INTERNALS__.invoke(command);
        calls.push({command,ms:performance.now()-start,count:result.length,jsonChars:JSON.stringify(result).length});
      }
      return calls;
    });
    const summary=names=>Object.fromEntries(names.map(name=>{const values=results.filter(x=>x.name===name&&x.round>0).map(x=>x.ms).sort((a,b)=>a-b);return[name,{median:values[Math.floor(values.length/2)],p95:values[Math.ceil(values.length*.95)-1],max:Math.max(...values)}];}));
    return {summary:summary(['下载任务','系统设置','关于软件','下载作品']),results,calls,...probes,hot};
  } finally {
    await page.evaluate(()=>window.__menuProbe.stop());
    await session.detach();
  }
}
