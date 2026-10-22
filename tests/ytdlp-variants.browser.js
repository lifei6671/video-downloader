// playwright-cli -s=ytdlp run-code (Get-Content tests/ytdlp-variants.browser.js -Raw)
async page => {
  const fixture = 'http://127.0.0.1:1420/tests/fixtures/background-download.html';
  const check = (value, message) => { if (!value) throw Error(message); };
  const input = page.locator('#video-input');
  const dialog = page.getByRole('alertdialog');
  const parse = async scenario => {
    await page.goto(fixture + '?scenario=' + scenario);
    await page.waitForFunction(() => window.backgroundTest?.loaded);
    await input.fill('https://www.youtube.com/watch?v=test1234567');
    await page.locator('form').filter({has:input}).getByRole('button',{name:'重新解析',exact:true}).click();
    await page.getByText('新作品 1',{exact:true}).waitFor();
  };
  const choose = async resolution => {
    const button = page.getByRole('radio').filter({hasText:resolution});
    await button.focus(); await page.keyboard.press('Space');
    await page.waitForFunction(value=>window.backgroundTest.snapshot().find(t=>t.id==='next-1').video.selected_format_id===value,
      resolution.startsWith('3840')?'2'.repeat(24):'1'.repeat(24));
  };
  const download = () => page.getByRole('button',{name:'下载视频',exact:true}).click();
  await parse('variant');
  await dialog.waitFor(); await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
  await choose('3840');
  check(!await dialog.isVisible(),'4K matched the previous 1080P record');
  await download();
  check((await page.evaluate(()=>window.backgroundTest.calls)).find(c=>c.type==='download').format==='2'.repeat(24),'wrong selected variant submitted');
  await page.evaluate(()=>window.backgroundTest.progress({bytes:65536,total_bytes:null,total_bytes_estimate:131072,speed_bps:0,eta_seconds:2.5}));
  await page.getByText('预计剩余 3 秒',{exact:true}).waitFor();
  check((await page.locator('body').innerText()).includes('64.0 KB / 约 128.0 KB'),'estimated total lost its marker');
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'horizontal overflow');
    await page.screenshot({path:'.local/ytdlp-progress-'+width+'.png'});
  }
  await page.evaluate(()=>window.backgroundTest.progress({merging:true,speed_bps:null,eta_seconds:null}));
  await page.getByText('正在合并音视频…',{exact:true}).waitFor();
  await parse('variant');
  await dialog.waitFor(); await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
  await choose('3840'); await choose('1920'); await download();
  await dialog.waitFor();
  check((await page.evaluate(()=>window.backgroundTest.calls)).every(c=>c.type!=='download'),'same variant bypassed confirmation');
  await parse('variant-missing'); await download();
  check(!await dialog.isVisible(),'missing file blocked redownload');
  await parse('variant-legacy');
  await dialog.waitFor(); await dialog.getByRole('button',{name:'暂不下载',exact:true}).click();
  await choose('3840'); await dialog.waitFor();
  await parse('variant-race'); await download(); await dialog.waitFor();
  await dialog.getByRole('button',{name:'重新下载',exact:true}).click();
  check((await page.evaluate(()=>window.backgroundTest.calls)).filter(c=>c.type==='download').map(c=>c.confirm).join(',')==='false,true','backend race did not require confirmation');
  return {passed:true,cases:['format switch','exact duplicate','legacy','missing file','completion race','ETA','estimates','merging','three viewports']};
}
