async page => {
  page.setDefaultTimeout(10000);
  const base=page.url().split('/').slice(0,3).join('/'), results=[];
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(base+'/tests/fixtures/update-manifest.html');
    const button=page.getByRole('button',{name:'检查更新',exact:true});
    await button.waitFor();
    for(const [code,text] of [
      ['UPDATE_ACCESS_DENIED','更新服务拒绝访问'],
      ['UPDATE_RATE_LIMITED','更新服务请求过于频繁'],
      ['UPDATE_MANIFEST_UNAVAILABLE','更新信息暂不可用'],
      ['UPDATE_INVALID_RESPONSE','更新信息格式异常'],
      ['UPDATE_TIMEOUT','获取更新信息超时'],
      ['UPDATE_NETWORK_ERROR','无法连接更新服务'],
    ]) {
      await page.evaluate(code=>window.updateManifestTest.error=code,code);
      await button.click();
      await page.getByRole('alert').filter({hasText:text}).waitFor();
      const message=await page.getByRole('alert').innerText();
      check(code==='UPDATE_RATE_LIMITED'||!message.includes('请求过于频繁'),`${code} misreported as rate limited`);
      check(await button.isEnabled(),'check button remains disabled after error');
      check(await page.getByRole('dialog').count()===0,'error opened update dialog');
    }
    await page.getByRole('button',{name:'查看发行版本',exact:true}).click();
    check(await page.evaluate(()=>window.updateManifestTest.opened.at(-1).page==='releases'),'release-page action missing');
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`error overflow at ${width}`);
    results.push({width,case:'distinct-errors-and-release-page',passed:true});

    await page.evaluate(()=>window.updateManifestTest.error=null);
    await button.click();
    const dialog=page.getByRole('dialog');
    await dialog.getByRole('heading',{name:'发现新版本 v1.2.0'}).waitFor();
    await dialog.getByRole('heading',{name:'静态清单更新说明'}).waitFor();
    await dialog.getByRole('button',{name:'下载更新',exact:true}).click();
    await dialog.getByRole('button',{name:'取消下载',exact:true}).waitFor();
    check(await page.evaluate(()=>window.updateManifestTest.started.join()==='v1.2.0'),'download lost selected tag');
    await dialog.getByRole('button',{name:'取消下载',exact:true}).click();
    await dialog.getByText('下载已取消',{exact:true}).waitFor();
    check(await page.evaluate(()=>window.updateManifestTest.canceled.join()==='fixture-download'),'cancel lost download identity');
    await page.keyboard.press('Escape');
    await dialog.waitFor({state:'hidden'});
    await button.focus();
    results.push({width,case:'notes-download-and-cancel',passed:true});

    await page.evaluate(()=>{window.updateManifestTest.result.has_update=false;window.updateManifestTest.result.current_version='1.2.0'});
    await page.keyboard.press('Enter');
    await page.getByText('当前已是最新版本',{exact:true}).waitFor();
    check(await page.getByRole('dialog').count()===0,'latest result opened an update dialog');
    results.push({width,case:'keyboard-latest-version',passed:true});
  }
  return {passed:results.length,results};
}
