// Run against helper-text.html using the existing Playwright CLI and npm run dev.
async page => {
  const base=page.url().split('/').slice(0,3).join('/');
  const check=(value,message)=>{if(!value)throw Error(message)};
  const muted='rgb(100, 116, 139)';
  const foreground='rgb(24, 37, 54)';
  const checkHelpers=async(locator,label)=>{
    const styles=await locator.evaluateAll(elements=>elements.filter(el=>el.getClientRects().length).map(el=>{
      const style=getComputedStyle(el);return {size:style.fontSize,color:style.color,weight:style.fontWeight,text:el.textContent.slice(0,70)};
    }));
    check(styles.length>0,label+': missing visible explanation');
    for(const style of styles)check(style.size==='12px' && style.color===muted && style.weight==='400',label+': '+JSON.stringify(style));
    return styles.length;
  };
  const descriptions=()=>page.locator('[data-slot="field-description"], [data-slot="card-description"], [data-slot="empty-description"], [data-slot="dialog-description"], [data-slot="alert-dialog-description"], [role="note"] [data-slot="alert-description"]');
  const cases=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});await page.goto(base+'/tests/fixtures/helper-text.html');
    await page.getByText('粘贴链接开始解析',{exact:true}).waitFor();
    check(await page.locator('body').evaluate(el=>getComputedStyle(el).fontSize)==='14px','body text must remain 14px');
    await checkHelpers(descriptions(),'download empty state');
    await checkHelpers(page.locator('main > header p'),'page introduction');
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.getByText('还没有下载记录',{exact:true}).waitFor();
    await checkHelpers(descriptions(),'task empty state');
    check(await page.locator('main > header p').evaluate(el=>getComputedStyle(el).fontSize)==='14px','actual task counts were shrunk');
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await checkHelpers(page.locator('#settings-operation-area > header p'),'settings introduction');
    await checkHelpers(descriptions(),'download settings');
    await page.getByRole('button',{name:'平台账号',exact:true}).click();
    for(const name of ['抖音','TikTok','YouTube']) {
      await page.getByRole('tab',{name,exact:true}).click();
      await page.waitForFunction(()=>!document.querySelector('.settings-platforms [role="tabpanel"][data-state="active"] input[type="checkbox"]')?.disabled);
      check(await page.locator('.settings-platforms [role="tabpanel"][data-state="active"] [data-slot="card-title"]').evaluate(el=>getComputedStyle(el).fontWeight)==='600',name+' account title must use semibold across Chinese and Latin fonts');
      await checkHelpers(descriptions(),name+' account help');
    }
    await page.getByRole('radio',{name:'自定义',exact:true}).click();
    await checkHelpers(descriptions(),'YouTube custom chunk help');
    check(await page.getByRole('textbox',{name:'网页端 Cookie',exact:true}).evaluate(el=>getComputedStyle(el).fontSize)==='14px','Cookie value was shrunk');
    check(await page.getByText('HTTP / SOCKS5 地址',{exact:true}).evaluate(el=>getComputedStyle(el).fontSize)==='14px','field label was shrunk');
    await page.screenshot({path:'.local/helper-text-20261022/accounts-'+width+'.png',animations:'disabled'});
    await page.getByRole('tab',{name:'Telegram',exact:true}).click();
    check(await page.locator('.settings-platforms [role="tabpanel"][data-state="active"] [data-slot="card-title"]').evaluate(el=>getComputedStyle(el).fontWeight)==='600','Telegram account title must match the other platforms');
    await checkHelpers(descriptions(),'Telegram help and risk note');
    await checkHelpers(page.getByText('当前使用 tdl 默认会话；点击登录将创建映取独立会话，不覆盖现有登录。',{exact:true}),'Telegram session help');
    await page.getByRole('checkbox',{name:'启用 Telegram HTTP 实验模式',exact:true}).click();
    await page.getByRole('alertdialog').waitFor();
    await checkHelpers(page.locator('[data-slot="alert-dialog-description"]'),'confirmation explanation');
    await page.keyboard.press('Escape');
    await page.getByRole('button',{name:'运行依赖',exact:true}).click();
    await page.getByText('FFmpeg',{exact:true}).waitFor();
    await checkHelpers(descriptions(),'runtime cards and field explanations');
    await checkHelpers(page.getByText('优先使用系统已有工具；仅在缺失或版本不满足时下载。',{exact:true}),'runtime policy');
    await checkHelpers(page.getByText('可下载 FFmpeg 6.1.1，不会修改系统环境变量。',{exact:true}),'runtime missing-tool guidance');
    check(await page.getByText('Node.js 22.14.0',{exact:false}).first().evaluate(el=>getComputedStyle(el).fontSize)==='14px','actual dependency version was shrunk');
    await page.screenshot({path:'.local/helper-text-20261022/dependencies-'+width+'.png',animations:'disabled'});
    await page.getByRole('button',{name:'关于软件',exact:true}).click();
    await checkHelpers(descriptions(),'about card subtitles');
    await page.getByRole('button',{name:'检查更新',exact:true}).click();
    await page.getByRole('dialog').waitFor();
    await checkHelpers(page.locator('[data-slot="dialog-description"]'),'update dialog explanation');
    const releaseBody=page.getByText('这是更新说明正文，保留正文的字号和颜色。',{exact:true});
    check(await releaseBody.evaluate(el=>getComputedStyle(el).fontSize)==='14px','release notes body was shrunk');
    check(await releaseBody.evaluate(el=>getComputedStyle(el).color)===foreground,'release notes body was recolored');
    await page.keyboard.press('Escape');
    await page.getByRole('button',{name:'下载作品',exact:true}).click();
    await page.getByRole('textbox').first().fill('https://youtu.be/BaW_jenozKc');
    await page.getByRole('button',{name:'解析链接',exact:true}).click();
    const error=page.getByRole('alert').filter({hasText:'网络连接失败'});
    await error.waitFor();
    check(await error.locator('[data-slot="alert-description"]').evaluate(el=>getComputedStyle(el).color)!==muted,'error was recolored as helper text');
    await page.evaluate(()=>window.helperTextTest.resolveSuccess=true);
    await page.getByRole('button',{name:'解析链接',exact:true}).click();
    const author=page.getByText('@fixture-author',{exact:true});
    await author.waitFor();
    for(const actual of [author,page.getByText('原声：fixture-music',{exact:true})]) {
      check(await actual.evaluate(el=>getComputedStyle(el).fontSize)==='14px','actual media metadata was shrunk');
    }
    await page.waitForFunction(()=>document.querySelector('button[aria-label="查看封面"]').getAttribute('aria-disabled')==='false');
    await page.getByRole('button',{name:'查看封面',exact:true}).click();
    await page.getByRole('dialog').waitFor();
    check(await page.locator('[data-slot="dialog-description"]').evaluate(el=>getComputedStyle(el).fontSize)==='14px','actual work title was shrunk');
    await page.keyboard.press('Escape');
    await page.evaluate(()=>window.helperTextTest.queue());
    const queue=page.getByText(/等待下载\s*·\s*第\s*1\s*位/);
    await queue.waitFor();
    check(await queue.evaluate(el=>getComputedStyle(el).fontSize)==='14px','actual queue position was shrunk');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'horizontal overflow');
    cases.push(width+': download/settings/three platforms/Telegram/runtime/about/dialogs/errors; actual body and labels preserved');
  }
  return {passed:cases.length,cases};
}
