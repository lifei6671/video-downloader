async page => {
  page.setDefaultTimeout(10000);
  const check=(ok,message)=>{if(!ok)throw Error(message)};
  await page.getByRole('button',{name:'系统设置',exact:true}).click();
  await page.getByRole('navigation',{name:'设置分类'}).getByRole('button',{name:'MCP',exact:true}).click();
  const panel=page.locator('.mcp-settings');
  const toggle=panel.getByRole('switch',{name:'启用 MCP'});
  await panel.getByText('已关闭',{exact:true}).waitFor();
  const unavailable=await page.evaluate(async()=>{try{await window.__TAURI_INTERNALS__.invoke('copy_mcp_connection_url');return null}catch(error){return String(error)}});
  check(unavailable==='MCP_CONNECTION_UNAVAILABLE','disabled URL copied');
  await toggle.click(); await panel.getByText('运行中',{exact:true}).waitFor();
  await panel.getByRole('button',{name:'复制地址',exact:true}).click();
  await panel.getByText('地址已复制',{exact:true}).waitFor();
  const url=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  const info=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('get_mcp_connection'));
  check(url===info.status.endpoint&&url===await panel.getByRole('textbox',{name:'MCP 连接地址',exact:true}).inputValue(),'native clipboard differs from live endpoint');
  check(/^http:\/\/127\.0\.0\.1:\d+\/mcp$/.test(url),'non-loopback endpoint');
  check(!('adapter_path' in info),'command helper remains in public metadata');
  return {status:'PASSED',url,json:JSON.stringify({mcpServers:{yingqu:{url}}}),info};
}
