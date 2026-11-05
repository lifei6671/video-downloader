// UI fixture only: native-looking paths never touch the filesystem.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const results=[];
  const input=()=>page.getByRole('textbox',{name:'文件命名规则',exact:true});
  const preview=()=>page.getByRole('status',{name:'实时预览',exact:true});
  const save=()=>page.getByRole('button',{name:'保存命名规则',exact:true});
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const waitPreview=expected=>page.waitForFunction(value=>document.querySelector('.filename-preview')?.textContent===value && document.querySelector('.filename-preview')?.getAttribute('aria-busy')==='false',expected);
  async function open(directory) {
    await page.goto(base+'/tests/fixtures/filename-settings.html?directory='+encodeURIComponent(directory));
    await page.evaluate(()=>sessionStorage.clear());await page.reload();
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
  }
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) for(const windows of [true,false]) {
    const separator=windows?'\\':'/';
    const directory=windows?'F:\\Yingqu':'/Users/test/Videos';
    const native=value=>value.replaceAll('/',separator);
    await page.setViewportSize({width,height});await open(directory);
    await waitPreview(native('youtube/creator-sample-video.mp4'));
    check(await input().inputValue()===native('{platform}/{user_name}-{content}'),'saved rule not displayed natively');
    check(await save().isDisabled(),'display conversion marked saved rule dirty');
    check((await page.locator('#filename-template-help').textContent()).includes('用 '+separator+' 分隔文件夹'),'help uses wrong separator');
    await page.getByText('示例路径：'+directory+separator+native('youtube/creator-sample-video.mp4'),{exact:true}).waitFor();
    await input().fill(native('{platform}/{link_id}'));
    await waitPreview(native('youtube/sample-id.mp4'));
    await input().focus();await page.keyboard.press('Home');await page.keyboard.press('ArrowRight');
    // Select a variable after a separator, insert replacement and retain caret.
    await input().evaluate(el=>{el.focus();el.setSelectionRange(11,20);});
    await page.getByRole('button',{name:'插入作品描述',exact:true}).click();
    check(await input().inputValue()===native('{platform}/{content}'),'variable selection insertion failed');
    check(await input().evaluate(el=>document.activeElement===el&&el.selectionStart===20&&el.selectionEnd===20),'insert caret moved');
    await waitPreview(native('youtube/sample-video.mp4'));
    check(await preview().locator('[data-filename-variable]').count()===2,'variable colors lost');
    await save().click();
    await page.waitForFunction(()=>window.filenameSettingsTest.snapshot().filename_template==='{platform}/{content}');
    await page.reload();await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await waitPreview(native('youtube/sample-video.mp4'));
    check(await save().isDisabled() && await input().inputValue()===native('{platform}/{content}'),'saved rule failed reload');
    if(windows){
      await input().fill('{platform}/{link_id}');await waitPreview('youtube\\sample-id.mp4');
      check(await input().inputValue()==='{platform}\\{link_id}','legacy slash input was not normalized');
      await input().fill('ab');await input().evaluate(el=>{el.focus();el.setSelectionRange(1,1);});
      await page.keyboard.type('/x');
      check(await input().inputValue()==='a\\xb','normalizing a slash moved the caret to the end');
      await input().evaluate(el=>{el.focus();el.setSelectionRange(1,2);});
      await page.keyboard.type('/y');
      check(await input().inputValue()==='a\\yxb','equivalent separator replacement moved the caret');
      await input().fill('ab');await input().evaluate(el=>{el.focus();el.setSelectionRange(1,1);});
      await page.keyboard.type('\\x');
      check(await input().inputValue()==='a\\xb','native separator moved the caret');
    }
    for(const invalid of [native('../escape'),separator+'absolute',native('a//b')]) {
      await input().fill(invalid);
      await page.waitForFunction(()=>document.querySelector('#filename-template')?.getAttribute('aria-invalid')==='true');
      check(await save().isDisabled(),'invalid path became savable');
    }
    await page.getByRole('button',{name:'恢复默认',exact:true}).click();
    await waitPreview(native('youtube/creator-sample-video.mp4'));
    check(await input().inputValue()===native('{platform}/{user_name}-{content}'),'default does not match native separator');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    results.push({width,windows,nativeInput:true,nativePreview:true,canonicalPersistence:true,validation:true});
  }
  for(const [directory,prefix] of [
    ['C:\\','C:\\'],['C:/Videos/','C:\\Videos\\'],
    ['\\\\server\\share\\','\\\\server\\share\\'],
    ['\\\\?\\C:\\Videos','C:\\Videos\\'],
    ['\\\\?\\UNC\\server\\share','\\\\server\\share\\'],['/','/'],
  ]) {
    await open(directory);
    const relative=prefix==='/'?'youtube/creator-sample-video.mp4':'youtube\\creator-sample-video.mp4';
    await waitPreview(relative);
    await page.getByText('示例路径：'+prefix+relative,{exact:true}).waitFor();
    results.push({directory,nativeRoot:true});
  }
  return {passed:results.length,results};
}
