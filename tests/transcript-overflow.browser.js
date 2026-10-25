// Long text scrolls inside its fixed frame; all focus edges and actions stay visible.
async page => {
  page.setDefaultTimeout(7000);
  const base=page.url().split('/').slice(0,3).join('/');
  const cases=[];
  const check=(value,message)=>{if(!value)throw Error(message)};
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    for (const scenario of ['transcript-long','transcript-unbroken']) {
      await page.goto(base+'/tests/fixtures/background-download.html?scenario='+scenario);
      await page.getByRole('button',{name:'下载任务',exact:true}).click();
      await page.locator('[data-task-id=speech]').click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'提取文案',exact:true}).click();
      const dialog=page.getByRole('dialog',{name:'视频文案'});
      await dialog.getByRole('button',{name:'开始提取',exact:true}).click();
      await page.getByText(/正在加载语音模型/).waitFor();
      await page.evaluate(()=>window.backgroundTest.finishTranscription());
      const text=dialog.getByLabel('识别文案',{exact:true}); await text.waitFor();
      const content=await text.inputValue();
      check(content.length>15000,'fixture is not long enough');
      const scroll=text;
      const time=dialog.getByText(/^提取时间：/);
      const header=dialog.locator('[data-slot="dialog-header"]');
      const footer=dialog.locator('[data-slot="dialog-footer"]');
      const headerBefore=await header.boundingBox(),footerBefore=await footer.boundingBox();
      const geometry=await text.evaluate(element=>{
        const body=element.parentElement, stamp=element.nextElementSibling;
        return {textHeight:element.clientHeight,textScrollHeight:element.scrollHeight,
          textWidth:element.clientWidth,textScrollWidth:element.scrollWidth,
          textBottom:element.getBoundingClientRect().bottom,timeTop:stamp.getBoundingClientRect().top,
          scrollHeight:body.scrollHeight,clientHeight:body.clientHeight,scrollWidth:body.scrollWidth,clientWidth:body.clientWidth};
      });
      check(geometry.textScrollHeight>geometry.textHeight,'long text has no internal scroll at '+scenario+' '+width);
      check(geometry.timeTop>=geometry.textBottom,'text overlaps extraction time');
      check(geometry.scrollHeight<=geometry.clientHeight+1,'text frame overflowed the dialog body');
      check(geometry.textScrollWidth<=geometry.textWidth+1 && geometry.scrollWidth<=geometry.clientWidth+1,'unbroken text causes horizontal overflow');
      const box=await dialog.boundingBox();
      check(box.x>=0 && box.y>=0 && box.x+box.width<=width+1 && box.y+box.height<=height+1,'dialog exceeds viewport');
      await text.focus(); await page.keyboard.press('PageDown');
      await page.waitForFunction(()=>document.querySelector('[aria-label="识别文案"]').scrollTop>0);
      const frame=await text.boundingBox();
      for(const position of [0,0.5,1]) {
        await scroll.evaluate((element,position)=>{element.scrollTop=(element.scrollHeight-element.clientHeight)*position},position);
        const focus=await text.evaluate(element=>{
          const r=element.getBoundingClientRect(),b=element.parentElement.getBoundingClientRect(),css=getComputedStyle(element);
          const outside=Math.max(0,parseFloat(css.outlineWidth)+parseFloat(css.outlineOffset));
          return {top:r.top-outside-b.top,left:r.left-outside-b.left,right:b.right-r.right-outside,bottom:b.bottom-r.bottom-outside,
            visible:element.matches(':focus-visible') && css.outlineStyle!=='none',width:parseFloat(css.outlineWidth),scroll:element.scrollTop};
        });
        check(focus.visible && focus.width>0,'keyboard focus indicator missing');
        check([focus.top,focus.left,focus.right,focus.bottom].every(edge=>edge>=-0.5),'focus edge clipped at '+position+' '+scenario+' '+width);
        const moved=await text.boundingBox();
        check(Math.abs(moved.x-frame.x)<1 && Math.abs(moved.y-frame.y)<1 && Math.abs(moved.height-frame.height)<1,'scrolling moved the text frame');
        if(position>0)check(focus.scroll>0,'text did not scroll internally');
      }
      await page.keyboard.press('Control+A');
      check(await text.evaluate(el=>el.selectionStart===0 && el.selectionEnd===el.value.length),'long text cannot be selected in full');
      await scroll.evaluate(element=>{element.scrollTop=element.scrollHeight});
      const headerAfter=await header.boundingBox(),footerAfter=await footer.boundingBox(),timeBox=await time.boundingBox();
      check(Math.abs(headerAfter.y-headerBefore.y)<1 && Math.abs(footerAfter.y-footerBefore.y)<1,'header or actions moved with text');
      check(timeBox.y+timeBox.height<=footerAfter.y && timeBox.y>=headerAfter.y+headerAfter.height,'extraction time is not reachable at the end');
      for(const name of ['复制全文','导出 TXT','重新提取']) {
        const button=dialog.getByRole('button',{name,exact:true});
        check(await button.isVisible() && await button.isEnabled(),'footer action unavailable: '+name);
        check(await button.evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))}),'text overlays footer: '+name);
      }
      await dialog.getByRole('button',{name:'复制全文',exact:true}).click();
      await dialog.getByRole('button',{name:'导出 TXT',exact:true}).click();
      const outputs=await page.evaluate(()=>window.backgroundTest.calls.filter(c=>c.type==='copy-transcript'||c.type==='export-transcript'));
      check(outputs.length===2 && outputs.every(v=>v.text===content),'copy/export lost long text');
      await text.focus();
      await page.screenshot({path:'output/playwright/transcript-focus-'+scenario+'-'+width+'.png'});
      cases.push(scenario+' '+width+'x'+height+': internal scroll, four focus edges, keyboard select/scroll, fixed actions, complete copy/export');
    }
  }
  return {passed:cases.length,cases};
}
