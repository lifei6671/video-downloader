// Runs against an installed isolated build, never a Vite/browser fixture.
async page => {
  if(page.url().startsWith('http://127.0.0.1:1420'))throw Error('must use the installed app');
  await page.getByRole('button',{name:'系统设置',exact:true}).click();
  await page.getByRole('navigation',{name:'设置分类'}).getByRole('button',{name:'MCP',exact:true}).click();
  const panel=page.locator('.mcp-settings'); await panel.getByText('已关闭',{exact:true}).waitFor();
  await panel.getByRole('switch',{name:'启用 MCP'}).click(); await panel.getByText('运行中',{exact:true}).waitFor();
  await panel.getByRole('button',{name:'复制地址',exact:true}).click(); await panel.getByText('地址已复制',{exact:true}).waitFor();
  const url=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  const info=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('get_mcp_connection'));
  if(url!==info.status.endpoint||!url.startsWith('http://127.0.0.1:'))throw Error('installed HTTP address mismatch');
  return {status:'PASSED',installed:true,embeddedFrontend:page.url(),url,info};
}
