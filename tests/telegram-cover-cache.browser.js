// Existing Playwright CLI, with npm run dev. This uses only mock IPC and image bytes.
async page => {
  const base = page.url().split('/').slice(0,3).join('/');
  const check = (value,message) => {if(!value)throw Error(message)};
  const results=[];
  for (const platform of ['douyin','tiktok','youtube','telegram']) for (const restored of [false,true]) {
    await page.goto(base+'/tests/fixtures/background-download.html?scenario='+(restored?'cached-cover-'+platform:platform));
    await page.waitForFunction(()=>window.backgroundTest.loaded);
    if(!restored) {
      await page.getByRole('button',{name:'后台下载',exact:true}).click();
      await page.evaluate(()=>window.backgroundTest.completeCached('original'));
    }
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    const cover=page.getByRole('img',{name:'作品封面',exact:true});
    await cover.waitFor();
    await cover.evaluate(image=>image.decode());
    const snapshot=await page.evaluate(()=>window.backgroundTest.snapshot()[0]);
    check(snapshot.cover_path===null && snapshot.cover_cached,'expected only SQLite-backed cover flag');
    check((await cover.getAttribute('src')).startsWith('blob:'),'not rendered through local IPC bytes');
    for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await page.setViewportSize({width,height});
      check(await cover.isVisible(),'cover missing at '+width);
      check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
    }
    results.push(platform+': '+(restored?'restored cached task shows cover without image path':'completion event displays cached cover without image path'));
  }
  return {passed:results.length,cases:results};
}
