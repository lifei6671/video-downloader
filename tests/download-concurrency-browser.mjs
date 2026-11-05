// Production UI with synthetic IPC; no real downloads or settings writes.
async page => {
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p=await page.context().newPage();p.setDefaultTimeout(7000);await p.setViewportSize({width,height});
    await p.addInitScript(()=>{
      window.concurrencyQA={settings:{download_directory:'C:\\fixture',filename_template:'{title}',max_downloads:3,profiles:[]},values:[]};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
        const qa=window.concurrencyQA;
        if(command==='get_settings')return structuredClone(qa.settings);
        if(command==='set_max_downloads') {
          if(!Number.isInteger(args.value)||args.value<1||args.value>5)throw 'CONFIG_INVALID';
          qa.values.push(args.value);qa.settings.max_downloads=args.value;return structuredClone(qa.settings);
        }
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(['list_tasks','list_backup_assets','list_backup_operations','list_video_transcripts','list_backup_spaces'].includes(command))return [];
        return null;
      }};
    });
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
      await p.getByRole('button',{name:'系统设置',exact:true}).click();
      const select=p.getByRole('combobox',{name:'同时下载数量',exact:true});
      if((await select.textContent()).trim()!=='3 个')throw Error('Default is not three');
      await p.getByText('默认 3 个；降低上限不打断已开始的下载。',{exact:true}).waitFor();
      await select.click();
      const choices=await p.getByRole('option').allTextContents();
      if(choices.map(value=>value.trim()).join(',')!=='1 个,2 个,3 个,4 个,5 个')throw Error('Unexpected concurrency choices: '+choices);
      await p.getByRole('option',{name:'5 个',exact:true}).click();
      await p.waitForFunction(()=>window.concurrencyQA.settings.max_downloads===5);
      await select.click();await p.getByRole('option',{name:'4 个',exact:true}).click();
      await p.waitForFunction(()=>window.concurrencyQA.settings.max_downloads===4);
      await p.getByRole('button',{name:'下载作品',exact:true}).click();
      await p.getByRole('button',{name:'系统设置',exact:true}).click();
      if((await select.textContent()).trim()!=='4 个')throw Error('Selection reset after navigation');
      if((await p.evaluate(()=>window.concurrencyQA.values.join(',')))!=='5,4')throw Error('Incorrect IPC values');
      results.push({width,height,default:3,maximum:5,selectionSaved:true});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
