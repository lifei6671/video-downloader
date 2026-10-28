async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9341');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('native page missing');
  p.setDefaultTimeout(10000);
  const settings = await p.evaluate(() => window.__TAURI_INTERNALS__.invoke('get_settings'));
  if (!settings.download_directory.includes('wb311-final')) throw Error('wrong profile');
  await p.getByRole('button', { name: '系统设置', exact: true }).click();
  await p.getByRole('navigation', { name: '设置分类' }).getByRole('button', { name: 'MCP', exact: true }).click();
  const panel = p.locator('.mcp-settings');
  await panel.getByText('已关闭', { exact: true }).waitFor();
  await panel.getByText(/追加分类标签，以及启动和取消转录/).waitFor();
  await panel.getByRole('switch', { name: '启用 MCP' }).click();
  await panel.getByText('运行中', { exact: true }).waitFor();
  await panel.getByRole('switch', { name: '允许 AI 写入' }).focus(); await p.keyboard.press('Space');
  await panel.getByText('已允许写入', { exact: true }).waitFor();
  await p.screenshot({ path: '.local/wb311/native-settings.png', fullPage: true });
  await panel.getByRole('switch', { name: '启用 MCP' }).click();
  await panel.getByText('已关闭', { exact: true }).waitFor();
  return { status: 'PASSED', nativeScopeCopyAndKeyboardPermission: true };
}
