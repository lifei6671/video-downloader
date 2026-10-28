// Run with the existing Playwright CLI run-code --filename, with npm run dev.
async page => {
  page.setDefaultTimeout(5000);
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const field=page.getByRole('textbox',{name:'作品分享链接',exact:true});
  const menu=page.getByRole('menu',{name:'作品链接编辑'});
  const item=name=>menu.getByRole('menuitem',{name,exact:true});
  const open=async()=>{await field.click({button:'right',position:{x:18,y:12}});await menu.waitFor()};
  const range=async(start,end)=>{await field.evaluate((node,args)=>{node.focus();node.setSelectionRange(...args)},[start,end]);await field.dispatchEvent('contextmenu',{clientX:350,clientY:210});await menu.waitFor()};
  const value=async expected=>{await page.waitForFunction(text=>document.querySelector('#video-input').value===text,expected)};
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await page.goto(base+'/tests/fixtures/link-input.html');
    await field.fill('作品链接 abc');await open();
    check((await menu.getByRole('menuitem').allTextContents()).join(',')==='全选,剪切,复制,粘贴,清空','menu actions differ');
    await item('全选').click();
    await page.waitForFunction(()=>{const n=document.querySelector('#video-input');return document.activeElement===n && n.selectionStart===0 && n.selectionEnd===n.value.length});
    await open();await item('复制').click();
    check(await page.evaluate(()=>window.linkInputTest.text)==='作品链接 abc','copy lost selection');
    check(await field.inputValue()==='作品链接 abc','copy changed text');
    await range(5,8);await item('剪切').click();await value('作品链接 ');
    check(await page.evaluate(()=>window.linkInputTest.text)==='abc','cut copied incorrect selection');
    await page.evaluate(()=>window.linkInputTest.text='新内容\r\n第二行');
    await range(2,4);await item('粘贴').click();await value('作品新内容\n第二行 ');
    await page.waitForFunction(()=>document.querySelector('#video-input').selectionStart===9);
    await open();await item('清空').click();await value('');
    await open();check(await item('全选').isDisabled() && await item('清空').isDisabled() && await item('剪切').isDisabled() && await item('复制').isDisabled(),'empty actions enabled');
    const box=await menu.boundingBox();check(box && box.x>=0 && box.y>=0 && box.x+box.width<=width && box.y+box.height<=height,'menu outside viewport');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    await page.screenshot({path:'.local/link-input-'+width+'.png',animations:'disabled'});
    await page.keyboard.press('Escape');await field.focus();await field.press('Control+A');await page.keyboard.type('键盘输入');
    await field.press('Shift+Enter');await page.keyboard.type('第二行');await value('键盘输入\n第二行');
    check(await page.evaluate(()=>window.linkInputTest.resolved.length)===0,'Shift Enter submitted');
    await field.press('Enter');await page.waitForFunction(()=>window.linkInputTest.resolved.length===1);
    check(await page.evaluate(()=>window.linkInputTest.resolved[0])==='键盘输入\n第二行','parse input changed');
    results.push(width+': mouse menu/select/copy/cut/paste/clear/empty/focus/keyboard/layout');
  }
  await field.fill('保留原文');await page.evaluate(()=>window.linkInputTest.failWrite=true);
  await range(0,4);await item('剪切').click();await page.getByText('无法写入剪贴板，请使用 Ctrl / ⌘ + C 或 X。',{exact:true}).waitFor();await value('保留原文');
  await page.evaluate(()=>{window.linkInputTest.failRead=true;window.linkInputTest.failWrite=false});
  await range(0,4);await item('粘贴').click();await page.getByText('无法读取剪贴板，请使用 Ctrl / ⌘ + V 粘贴。',{exact:true}).waitFor();await value('保留原文');
  results.push('clipboard read/write failures preserve original');
  await page.evaluate(()=>{window.linkInputTest.failRead=false;window.linkInputTest.delayRead=true});
  await range(0,4);await item('粘贴').click();await page.waitForFunction(()=>window.linkInputTest.pendingRead!==null);
  await page.getByRole('button',{name:'下载任务',exact:true}).click();
  await page.evaluate(()=>window.linkInputTest.pendingRead('迟到的内容'));
  await page.getByRole('button',{name:'下载中心',exact:true}).click();await value('保留原文');
  results.push('late clipboard result cannot overwrite another page');
  await page.goto(base+'/tests/fixtures/link-input.html');await field.fill('abcd');
  await page.evaluate(()=>window.linkInputTest.text='插入');await range(2,2);await item('粘贴').click();await value('ab插入cd');
  await page.evaluate(()=>window.linkInputTest.text='');await range(0,2);await item('粘贴').click();
  await page.getByText('剪贴板中没有文本内容',{exact:true}).waitFor();await value('ab插入cd');
  await field.focus();await field.press('Shift+F10');await menu.waitFor();
  await page.keyboard.press('Home');await page.keyboard.press('Enter');
  await page.waitForFunction(()=>{const n=document.querySelector('#video-input');return document.activeElement===n && n.selectionStart===0 && n.selectionEnd===n.value.length});
  results.push('caret insertion/empty clipboard/Shift F10/keyboard menu selection');
  return {passed:results.length,cases:results};
}
