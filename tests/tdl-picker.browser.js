// Existing Playwright CLI + npm run dev. All file picker/config/check IPC is mocked.
async page=>{
 const base=page.url().split('/').slice(0,3).join('/');
 const check=(v,m)=>{if(!v)throw Error(m)};
 const card=page.locator('[data-slot="card"]').filter({has:page.getByText('tdl',{exact:true})});
 const picker=card.getByRole('button',{name:'选择 tdl 可执行文件',exact:true});
 const input=page.getByRole('textbox',{name:'tdl 可执行文件路径',exact:true});
 const calls=()=>page.evaluate(()=>window.tdlPickerTest.calls);
 const idle=()=>picker.waitFor({state:'visible'}).then(()=>page.waitForFunction(()=>!document.querySelector('button[aria-label="选择 tdl 可执行文件"]')?.disabled));
 const load=async()=>{await page.goto(base+'/tests/fixtures/tdl-picker.html');await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'运行依赖',exact:true}).click();await idle()};
 const reset=()=>page.evaluate(()=>window.tdlPickerTest.calls=[]);
 const result=[];
 for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
  await page.setViewportSize({width,height});await load();
  check(await card.getByRole('button',{name:'检测 tdl',exact:true}).count()===0,'old check button remains');
  check(await card.getByRole('button',{name:'保存 tdl 路径',exact:true}).count()===0,'old save button remains');
  check(await card.locator('[data-slot="card-footer"]').count()===0,'empty footer remains');
  check(await picker.evaluate(b=>b.textContent.trim()==='' && !!b.querySelector('svg')),'picker must be icon only');
  const bounds=await input.boundingBox(),button=await picker.boundingBox();
  check(button.x>bounds.x && Math.abs(button.y-bounds.y)<8,'picker must follow input on same line');
  await picker.hover();await page.getByRole('tooltip').filter({hasText:'选择 tdl 可执行文件'}).waitFor();await page.keyboard.press('Escape');
  await reset();await picker.focus();await page.keyboard.press('Enter');await idle();
  const events=await calls();check(events.map(e=>e.type).join(',')==='choose,save,check','selection must save then check exactly once');
  await page.locator('[data-sonner-toast][data-type="success"]').filter({hasText:'tdl 路径已保存。'}).first().waitFor();
  check(await card.getByText('tdl 路径已保存。',{exact:true}).count()===0,'saved notice must not appear under card');
  check(events[2].path===events[1].config.path && events[2].savedPath===events[2].path,'checked before persistence or wrong path');
  check(events[1].config.enabled===true,'experimental mode changed');
  check(await input.inputValue()===events[2].path,'input did not show saved path');
  check((await page.evaluate(()=>window.tdlPickerTest.snapshot())).tdl.session_directory==='test-only-existing-session','session changed');
  check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'viewport overflow');
  await page.screenshot({path:'.local/tdl-picker-'+width+'.png',animations:'disabled'});
  await reset();await picker.click();await idle();
  check((await calls()).map(e=>e.type).join(',')==='choose,save,check','same path must recheck exactly once');
  result.push(width+' inline icon/tooltip/keyboard/save-then-check/same-path/session preservation');
 }
 await load();await reset();await page.evaluate(()=>window.tdlPickerTest.selected=null);await picker.click();await idle();
 check((await calls()).map(e=>e.type).join(',')==='choose','cancel saved or checked');
 check(await page.locator('[data-sonner-toast][data-type="success"]').count()===0,'cancel showed success toast');
 check((await page.evaluate(()=>window.tdlPickerTest.snapshot())).tdl.path===null,'cancel changed path');result.push('cancel preserves config');
 for(const failure of ['failPicker','failSave','failCheck']) {
  await load();await reset();await page.evaluate(key=>window.tdlPickerTest[key]=true,failure);await picker.click();await idle();
  await card.getByRole('alert').waitFor();const events=await calls();
  check(events.map(e=>e.type).join(',')===({'failPicker':'choose','failSave':'choose,save','failCheck':'choose,save,check'})[failure],'incorrect failure order');
  const snapshot=await page.evaluate(()=>window.tdlPickerTest.snapshot());
  check((await page.locator('[data-sonner-toast][data-type="success"]').count()>0)===(failure==='failCheck'),'save failure showed success or persisted path lacked toast');
  check((snapshot.tdl.path!==null)===(failure==='failCheck'),'save rollback or check failure config lost');
  if(failure==='failCheck')check(await card.getByText('不可用',{exact:true}).count()===1,'failed check reported usable');
  result.push(failure+' visible error and correct config');
 }
 for(const phase of ['holdPicker','holdSave','holdCheck']) {
  await load();await reset();await page.evaluate(key=>window.tdlPickerTest[key]=true,phase);await picker.click();
  await page.waitForFunction(type=>window.tdlPickerTest.calls.some(e=>e.type===type),({holdPicker:'choose',holdSave:'save',holdCheck:'check'})[phase]);
  check(await picker.isDisabled() && await input.isDisabled(),'pending controls enabled');
  await page.evaluate(()=>window.tdlPickerTest.release());await idle();
  check((await calls()).map(e=>e.type).join(',')==='choose,save,check','pending duplicate or missed operation');result.push(phase+' busy guard');
 }
 await load();await reset();await input.fill('C:\\Custom Tools\\tdl.exe');await input.press('Enter');await idle();
 check((await calls()).map(e=>e.type).join(',')==='save,check','manual Enter not saved/checked');
 await reset();await input.fill('');await input.press('Enter');await idle();
 check((await calls()).map(e=>e.type).join(',')==='save,check' && (await calls())[1].path===null,'clearing path must restore auto detection');
 const count=(await calls()).length;
 await page.getByRole('button',{name:'下载任务',exact:true}).click();await page.getByRole('button',{name:'系统设置',exact:true}).click();await page.getByRole('button',{name:'运行依赖',exact:true}).click();
 check((await calls()).length===count,'navigation retriggered check');result.push('manual Enter/clear auto-detect/navigation persistence');
 return {passed:result.length,cases:result};
}
