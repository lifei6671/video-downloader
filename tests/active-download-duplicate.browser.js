// Browser-only IPC fixtures; never downloads media or reads real sessions.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (value, message) => { if (!value) throw Error(message); };
  const dialog = page.getByRole('alertdialog');
  const results = [];
  const load = async platform => {
    await page.goto(`${base}/tests/fixtures/background-download.html?scenario=${platform}`);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await page.getByRole('button', {name:'后台下载',exact:true}).waitFor();
    const input=page.locator('#video-input');
    await input.fill(platform==='telegram'?'https://t.me/c/100/9':'https://www.douyin.com/video/123');
    await page.locator('form').filter({has:input}).getByRole('button',{name:'重新解析',exact:true}).click();
    await page.getByText('新作品 1',{exact:true}).waitFor();
  };
  const inject = async (status, submitted = true, differentFormat = false) => {
    await page.evaluate(async ({status, submitted, differentFormat}) => {
      const {api} = await import(performance.getEntriesByType('resource').find(e => e.name.includes('/src/api.ts')).name);
      const original = window.backgroundTest.snapshot()[0];
      window.ongoingDuplicate = {...structuredClone(original),id:'ongoing-duplicate',status,
        download_requested:submitted,created_at:1,path:null,completed_at:null};
      if (differentFormat) {
        window.ongoingDuplicate.video.selected_format_id = 'other-format';
        window.ongoingDuplicate.video.formats.push({format_id:'other-format',container:'mp4',codec:'h264',width:1280,height:720});
      }
      api.tasks = async () => [...window.backgroundTest.snapshot(),window.ongoingDuplicate];
      window.dispatchEvent(new Event('focus'));
    }, {status, submitted, differentFormat});
    await page.waitForTimeout(150);
  };
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for (const platform of ['telegram','douyin']) {
      for (const status of ['queued','downloading']) {
        await load(platform);
        await inject(status);
        await dialog.waitFor();
        check(await dialog.getByRole('heading',{name:'该规格已有下载任务',exact:true}).count()===1, 'ongoing duplicate uses completed-history wording');
        check(!(await dialog.innerText()).includes('本地文件仍存在'), 'ongoing duplicate claims a local file already exists');
        check((await page.evaluate(()=>window.backgroundTest.calls)).filter(c=>c.type==='download').length===0, 'duplicate started without confirmation');
        await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
        await dialog.waitFor({state:'detached'});
        check((await page.evaluate(()=>window.backgroundTest.calls)).filter(c=>c.type==='download').length===0, 'dismissing duplicate created a transfer');
        await page.getByRole('button',{name:'后台下载',exact:true}).click();
        await dialog.waitFor();
        await dialog.getByRole('button',{name:'重新下载',exact:true}).click();
        await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='download'));
        const calls = await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='download'));
        check(calls.length===1 && calls[0].confirm===true, 'confirmed duplicate was submitted twice or without confirmation');
        check(await page.locator('#video-input').inputValue()==='', 'confirmed background action did not clear its input');
        check(await page.locator('[data-sonner-toast][data-type="error"]').count()===0, 'normal duplicate confirmation produced an error Toast');
        results.push(`${width} ${platform} ${status}: prompt, cancel, one explicit background confirmation`);
      }
    }
  }
  for (const platform of ['telegram','douyin']) {
    await load(platform);
    await inject('parsing',true);
    await dialog.waitFor();
    results.push(`${platform}: already submitted parsing task requires confirmation`);
    for (const status of ['ready','failed','canceled','parsing']) {
      await load(platform);
      await inject(status,false);
      check(await dialog.count()===0, `${platform} ${status} unsubmitted/terminal task blocked a new request`);
    }
    results.push(`${platform}: failed, canceled and unsubmitted previews do not block`);
  }
  await load('douyin');
  await inject('queued',true,true);
  check(await dialog.count()===0, 'different Douyin format was treated as the same variant');
  await page.getByRole('button',{name:'后台下载',exact:true}).click();
  const differentVariantCalls=(await page.evaluate(()=>window.backgroundTest.calls)).filter(c=>c.type==='download');
  check(differentVariantCalls.length===1&&differentVariantCalls[0].confirm===false, 'different variant did not submit once without confirmation');
  results.push('Douyin different selected format remains independently downloadable');
  for (const platform of ['telegram','douyin']) {
    await load(platform);
    await inject('downloading');
    await dialog.waitFor();
    await dialog.getByRole('button',{name:'重新下载',exact:true}).click();
    await page.waitForFunction(()=>window.backgroundTest.calls.some(c=>c.type==='download'));
    await page.evaluate(async()=>{
      window.backgroundTest.fail('next-1');
      const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
      api.retry=async id=>{
        window.backgroundTest.calls.push({type:'retry',id});
        const task=window.backgroundTest.snapshot().find(t=>t.id===id);
        return {...task,status:'ready',error:null,revision:task.revision+1};
      };
    });
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    const feed=page.getByRole('button',{name:'切换为信息流',exact:true});
    if(await feed.count())await feed.click();
    await page.locator('[data-task-id="next-1"]').getByRole('button',{name:'重试下载',exact:true}).click();
    await dialog.waitFor();
    check(await dialog.getByRole('heading',{name:'该规格已有下载任务',exact:true}).count()===1,'same-task retry reused a previous duplicate consent');
    check((await page.evaluate(()=>window.backgroundTest.calls)).filter(c=>c.type==='download').length===1,'retry silently started another duplicate');
    await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
    results.push(`${platform}: failed same-task retry requires fresh duplicate confirmation`);
  }
  for (const platform of ['telegram','douyin']) {
    await load(platform);
    await page.evaluate(async()=>{
      const {api}=await import(performance.getEntriesByType('resource').find(e=>e.name.includes('/src/api.ts')).name);
      const original=window.backgroundTest.snapshot()[0];
      const previous={...structuredClone(original),id:'racing-download',status:'queued',download_requested:true,path:null,created_at:1};
      const download=api.download;
      let raced=false;
      window.duplicateRaceCalls=[];
      api.tasks=async()=>[...window.backgroundTest.snapshot(),...(raced?[previous]:[])];
      api.download=async(id,confirm=false)=>{
        window.duplicateRaceCalls.push(confirm);
        if(!confirm){raced=true;throw 'ALREADY_DOWNLOADED';}
        return download(id,confirm);
      };
    });
    await page.getByRole('button',{name:'后台下载',exact:true}).click();
    await dialog.waitFor();
    check(await dialog.getByRole('heading',{name:'该规格已有下载任务',exact:true}).count()===1, 'backend race did not recover to the ongoing confirmation dialog');
    await dialog.getByRole('button',{name:'重新下载',exact:true}).click();
    await page.waitForFunction(()=>window.duplicateRaceCalls.length===2);
    check(await page.evaluate(()=>JSON.stringify(window.duplicateRaceCalls)==='[false,true]'), 'backend rejection was retried without explicit confirmation');
    results.push(`${platform}: backend race refreshes identity and requires explicit confirmation`);
  }
  return {passed:results.length,results};
}
