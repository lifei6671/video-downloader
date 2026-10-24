// Browser pixel-ratio emulation; this does not change the operating system DPI.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const browser=page.context().browser();
  const results=[];
  for(const scale of [1.25,1.5]) {
    const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:scale});
    try {
      const target=await context.newPage();
      await target.goto(base+'/tests/fixtures/background-download.html?scenario=visual');
      await target.getByRole('button',{name:'后台下载',exact:true}).waitFor();
      for(const name of ['下载作品','下载任务','系统设置','关于软件']) {
        await target.getByRole('button',{name,exact:true}).click();
        if(name==='系统设置')await target.getByRole('button',{name:'平台账号',exact:true}).click();
        const fonts=await target.evaluate(()=>[...new Set([...document.querySelectorAll('h1,h2,h3,p,button,input,textarea,[data-slot="card-title"]')].filter(n=>n.getClientRects().length).map(n=>getComputedStyle(n).fontSize))]);
        if(fonts.some(size=>!['12px','14px','20px'].includes(size)))throw Error(name+' extra font size: '+fonts.join(','));
        if(await target.evaluate(()=>document.documentElement.scrollWidth>innerWidth))throw Error(name+' overflows at '+scale);
        if(name==='下载任务')await target.screenshot({path:'output/playwright/visual-redesign/scale-'+scale+'.png'});
      }
      results.push({deviceScaleFactor:scale,pages:4,result:'PASS'});
    } finally { await context.close(); }
  }
  return {emulatedScale:results};
}
