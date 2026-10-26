// UI-only IPC fixture. Rust tests verify the shared renderer and actual commits.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const input=()=>page.getByRole('textbox',{name:'文件命名规则',exact:true});
  const save=()=>page.getByRole('button',{name:'保存命名规则',exact:true});
  const preview=()=>page.getByRole('status',{name:'实时预览',exact:true});
  const path=async text=>page.waitForFunction(expected=>document.querySelector('[aria-labelledby="filename-preview-label"]')?.textContent===expected,text);
  const stored=()=>page.evaluate(()=>window.filenameSettingsTest.snapshot().filename_template);
  const open=async()=>{await page.getByRole('button',{name:'系统设置',exact:true}).click();await path('youtube/creator-sample-video.mp4')};
  const fresh=async()=>{await page.goto(base+'/tests/fixtures/filename-settings.html');await page.evaluate(()=>sessionStorage.clear());await page.reload();await open()};
  const saved=async()=>{await page.locator('[data-sonner-toast][data-type="success"]').last().waitFor();await page.waitForFunction(()=>document.querySelector('#filename-template')&&!Array.from(document.querySelectorAll('button')).some(b=>b.textContent==='保存中…'))};
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await fresh();
    check(await save().isDisabled(),'default must start clean');
    const defaultSegments=preview().locator('[data-filename-variable]');
    const defaultTags=await defaultSegments.evaluateAll(nodes=>nodes.map(el=>({key:el.dataset.filenameVariable,text:el.textContent,color:getComputedStyle(el).backgroundColor})));
    check(defaultTags.map(tag=>tag.key).join(',')==='platform,user_name,content' && new Set(defaultTags.map(tag=>tag.color)).size===3,'default variables are not separately colored');
    check(await page.getByRole('button',{name:'插入平台',exact:true}).evaluate(el=>getComputedStyle(el).cursor==='pointer'),'insert button cursor');
    await page.evaluate(async()=>{for(const weight of [400,500,600])await document.fonts.load(weight+' 14px "Alibaba PuHuiTi 3.0"')});
    check(await page.evaluate(()=>document.fonts.check('400 14px "Alibaba PuHuiTi 3.0"') && getComputedStyle(document.body).fontFamily.includes('Alibaba PuHuiTi 3.0')),'global font did not load');
    await input().fill('{platform}/{user_name}-{link_id}-{content}-{current_time}-{quality}');
    await path('youtube/creator-sample-id-sample-video-20261026_120000-1080P.mp4');
    const colors=await preview().locator('[data-filename-variable]').evaluateAll(nodes=>nodes.map(el=>getComputedStyle(el).backgroundColor));
    check(colors.length===6 && new Set(colors).size===6 && !colors.includes('rgba(0, 0, 0, 0)'),'six variable colors are not distinct');
    await input().fill('youtube-{platform}-{platform}');await path('youtube-youtube-youtube.mp4');
    check(await preview().locator('[data-filename-variable="platform"]').count()===2,'literal text was incorrectly colored as a variable');
    const previousPreview=await preview().textContent();
    await preview().evaluate(el=>{
      window.previewFlashes=[];
      window.previewObserver=new MutationObserver(()=>{if(!el.textContent || el.textContent.includes('正在预览'))window.previewFlashes.push(el.textContent)});
      window.previewObserver.observe(el,{subtree:true,childList:true,characterData:true});
    });
    await page.evaluate(()=>window.filenameSettingsTest.holdPreview=true);
    await input().fill('{link_id}');
    await page.waitForFunction(()=>window.filenameSettingsTest.pendingPreviews.length===1);
    check(await preview().textContent()===previousPreview && await preview().getAttribute('aria-busy')==='true' && await save().isDisabled(),'pending preview cleared content or enabled save');
    await input().focus();await page.keyboard.press('End');
    await page.getByRole('button',{name:'插入清晰度',exact:true}).click();
    await page.waitForFunction(()=>window.filenameSettingsTest.pendingPreviews.length===2);
    check(await preview().textContent()===previousPreview,'quick insert replaced preview with loading text');
    await page.evaluate(()=>window.filenameSettingsTest.releasePreview(1));
    await path('sample-id1080P.mp4');
    await page.evaluate(()=>window.filenameSettingsTest.releasePreview(0));
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    check(await preview().textContent()==='sample-id1080P.mp4','stale preview overwrote newer response');
    check(await page.evaluate(()=>window.previewFlashes.length)===0,'preview flashed while updating');
    await page.evaluate(()=>{window.previewObserver.disconnect();window.filenameSettingsTest.holdPreview=false});
    await input().fill('{platform}/{link_id}');
    await path('youtube/sample-id.mp4');
    await input().focus();await page.keyboard.press('End');
    await page.getByRole('button',{name:'插入清晰度',exact:true}).click();
    check(await input().inputValue()==='{platform}/{link_id}{quality}','variable was not inserted at caret');
    check(await input().evaluate(el=>el===document.activeElement),'variable insertion lost focus');
    await input().evaluate(el=>el.setSelectionRange(11,20));
    await page.getByRole('button',{name:'插入作品描述',exact:true}).click();
    check(await input().inputValue()==='{platform}/{content}{quality}','variable did not replace selection');
    await path('youtube/sample-video1080P.mp4');
    await input().evaluate(el=>el.setSelectionRange(11,20));
    await page.getByRole('button',{name:'插入作品描述',exact:true}).click();
    check(await input().evaluate(el=>el===document.activeElement),'identical replacement lost focus');
    check((await preview().textContent())==='youtube/sample-video1080P.mp4' && !await save().isDisabled(),'identical replacement stalled preview or save');
    for(const [rule,expected] of [
      ['{unknown}', '{unknown}.mp4'], ['{content', '{content.mp4'], ['content}', 'content}.mp4'],
      ['{{content}}', '{sample-video}.mp4'], ['{unknown-{content}', '{unknown-sample-video.mp4'],
      ['中文{quality}-{QUALITY}', '中文1080P-{QUALITY}.mp4'],
    ]) {
      await input().fill(rule);await path(expected);
      check(await input().getAttribute('aria-invalid')==='false' && !await save().isDisabled(),'literal text was rejected: '+rule);
    }
    for(const invalid of ['', '../a', '/absolute', 'a//b', 'a\\b']) {
      await input().fill(invalid);await page.waitForFunction(()=>document.querySelector('#filename-template')?.getAttribute('aria-invalid')==='true');
      check(await save().isDisabled(),'invalid rule can save: '+invalid);
    }
    const literalRule='{platform}/{user_name}-{link_id}-{quality-{unknown}';
    await input().fill(literalRule);await path('youtube/creator-sample-id-{quality-{unknown}.mp4');
    check((await preview().locator('[data-filename-variable]').evaluateAll(nodes=>nodes.map(node=>node.dataset.filenameVariable))).join(',')==='platform,user_name,link_id','literal braces received variable colors');
    await save().waitFor({state:'visible'});
    await page.waitForFunction(()=>!Array.from(document.querySelectorAll('button')).find(b=>b.textContent==='保存命名规则')?.disabled);
    await page.keyboard.press('Enter');await saved();
    check(await stored()===literalRule,'Enter did not persist literal braces');
    await page.reload();await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await path('youtube/creator-sample-id-{quality-{unknown}.mp4');
    check(await input().inputValue()===literalRule && await save().isDisabled(),'reload lost saved literal braces');
    await page.screenshot({path:'.local/naming-literal/settings-'+width+'.png',animations:'disabled'});
    await page.getByRole('button',{name:'恢复默认',exact:true}).click();
    await path('youtube/creator-sample-video.mp4');
    check(await stored()===literalRule,'restore default silently saved draft');
    await save().click();await saved();
    await page.screenshot({path:'.local/font-preview/settings-'+width+'.png',animations:'disabled'});
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow at '+width);
    check(await page.getByRole('button',{name:'插入平台',exact:true}).evaluate(el=>el.getBoundingClientRect().height>=52),'variable labels are clipped');
    await input().fill('x'.repeat(180));
    await path('x'.repeat(180)+'.mp4');
    await save().scrollIntoViewIfNeeded();
    check(await save().evaluate(el=>{
      const button=el.getBoundingClientRect(),card=el.closest('[data-slot="card"]').getBoundingClientRect();
      return button.top>=card.top && button.bottom<=card.bottom-12 && button.bottom<=innerHeight;
    }),'long preview clips save button instead of scrolling');
    cases.push(width+': preview/insertion/selection/focus/validation/Enter/persistence/default/layout');
  }
  await fresh();await input().fill('{link_id}');await path('sample-id.mp4');
  await page.evaluate(()=>window.filenameSettingsTest.failSave=true);await save().click();
  await page.locator('[data-sonner-toast][data-type="error"]').waitFor();
  check(await stored()==='{platform}/{user_name}-{content}' && await input().inputValue()==='{link_id}','failure lost saved setting or draft');
  await page.evaluate(()=>{window.filenameSettingsTest.failSave=false;window.filenameSettingsTest.holdSave=true});
  await save().click();await page.getByRole('button',{name:'保存中…',exact:true}).waitFor();
  check(await input().isDisabled() && await page.getByRole('button',{name:'插入平台',exact:true}).isDisabled(),'pending save did not freeze input');
  await page.evaluate(()=>{window.filenameSettingsTest.holdSave=false;window.filenameSettingsTest.release()});await saved();
  check(await stored()==='{link_id}','retry did not persist draft');
  cases.push('failure preserves draft and setting; pending save freezes controls; retry succeeds');
  return {passed:cases.length,cases};
}
