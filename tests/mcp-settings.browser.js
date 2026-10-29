async page => {
  page.setDefaultTimeout(10000);
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  const panel=()=>page.locator('.mcp-settings');
  const toggle=()=>panel().getByRole('switch',{name:'启用 MCP'});
  const write=()=>panel().getByRole('switch',{name:'允许 AI 写入'});
  const copy=()=>panel().getByRole('button',{name:'复制地址',exact:true});
  const refresh=()=>panel().getByRole('button',{name:'刷新状态',exact:true});
  const load=async()=>{
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?mcp=1');
    await page.getByRole('button',{name:'系统设置',exact:true}).click();
    await page.getByRole('navigation',{name:'设置分类'}).getByRole('button',{name:'MCP',exact:true}).click();
    await panel().getByText('已关闭',{exact:true}).waitFor();
  };
  const results=[];
  for(const [width,height] of [[1440,900],[1280,800],[1024,680]]){
    await page.setViewportSize({width,height}); await load();
    check(await toggle().getAttribute('aria-checked')==='false'&&await copy().isDisabled(),'MCP not default off');
    check(await panel().getByRole('radio').count()===0,'command config format remains');
    check(!(await panel().innerText()).includes('stdio'),'stdio hint remains');
    await toggle().focus(); await page.keyboard.press('Space'); await panel().getByText('运行中',{exact:true}).waitFor();
    const url=panel().getByRole('textbox',{name:'MCP 连接地址',exact:true});
    check(await url.inputValue()==='http://127.0.0.1:17653/mcp','wrong local URL');
    check(!await url.isEditable(),'address is editable');
    await copy().click(); await panel().getByText('地址已复制',{exact:true}).waitFor();
    check(await page.evaluate(()=>window.mcpTest.copies.at(-1))===await url.inputValue(),'clipboard differs from URL');
    await write().click(); await panel().getByText('已允许写入',{exact:true}).waitFor();
    await toggle().click(); await panel().getByText('已关闭',{exact:true}).waitFor();
    check(await write().getAttribute('aria-checked')==='false'&&await copy().isDisabled(),'stop retained write permission or address');
    check(await panel().getByRole('textbox',{name:'MCP 连接地址',exact:true}).count()===0,'stopped address still offered');
    await toggle().click(); await panel().getByText('运行中',{exact:true}).waitFor();
    check(await write().getAttribute('aria-checked')==='false','restart not read-only');
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'horizontal overflow');
    await page.screenshot({path:'.local/mcp-http-followup/settings-'+width+'.png',animations:'disabled'});
    results.push({viewport:width,urlCopyKeyboardPermissions:'PASS'});
  }
  await load(); await page.evaluate(()=>window.mcpTest.failNext('MCP_HTTP_BIND_FAILED'));
  await toggle().click(); await panel().getByText('MCP 服务启动失败，本机端口可能已被占用。',{exact:true}).waitFor();
  check(await toggle().getAttribute('aria-checked')==='false','failed bind looked enabled');
  await page.evaluate(()=>window.mcpTest.hold('set')); await toggle().click();
  await panel().getByText('正在开启…',{exact:true}).waitFor(); check(await toggle().isDisabled()&&await copy().isDisabled(),'pending mutations active');
  await page.evaluate(()=>window.mcpTest.release()); await panel().getByText('运行中',{exact:true}).waitFor();
  await page.evaluate(()=>window.mcpTest.failNext('CLIPBOARD_WRITE_FAILED')); await copy().click();
  await panel().getByText('复制失败，请重试。',{exact:true}).waitFor(); await copy().click();
  await panel().getByText('地址已复制',{exact:true}).waitFor();
  await page.evaluate(()=>{window.mcpTest.hold('read');window.dispatchEvent(new Event('focus'))});
  await toggle().click(); await panel().getByText('已关闭',{exact:true}).waitFor(); await page.evaluate(()=>window.mcpTest.release());
  check(await toggle().getAttribute('aria-checked')==='false'&&await copy().isDisabled(),'stale read revived service');
  await toggle().click(); await panel().getByText('运行中',{exact:true}).waitFor();
  await page.evaluate(()=>window.mcpTest.stop()); await panel().getByText('服务异常',{exact:true}).waitFor();
  check(await copy().isDisabled(),'failed service address copyable');
  await toggle().click(); await panel().getByText('运行中',{exact:true}).waitFor();
  await page.evaluate(()=>window.mcpTest.failRead(true)); await refresh().click();
  await panel().getByText('状态不可用',{exact:true}).waitFor(); check(await toggle().isDisabled(),'read failure allows mutation');
  await page.evaluate(()=>window.mcpTest.failRead(false)); await refresh().click(); await panel().getByText('运行中',{exact:true}).waitFor();
  results.push({bindFailureCopyFailurePendingStaleReadAndStatusRecovery:'PASS'});
  return results;
}
