// Run against the installed, isolated release build, without Vite or mock IPC.
async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  check(!page.url().startsWith('http://127.0.0.1:1420'), 'must run the installed embedded frontend');
  await page.getByRole('button', { name: '系统设置', exact: true }).click();
  await page.getByRole('navigation', { name: '设置分类' }).getByRole('button', { name: 'MCP', exact: true }).click();
  const panel = page.locator('.mcp-settings');
  const toggle = panel.getByRole('switch', { name: '启用 MCP' });
  const copy = panel.getByRole('button', { name: '复制配置', exact: true });
  await panel.getByText('已关闭', { exact: true }).waitFor();
  check(await copy.isDisabled(), 'installed service did not default off');
  await toggle.click(); await panel.getByText('运行中', { exact: true }).waitFor();
  await copy.click(); await panel.getByText('配置已复制', { exact: true }).waitFor();
  const toml = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  check(toml === await panel.getByLabel('客户端配置内容').textContent(), 'installed TOML clipboard mismatch');
  await panel.getByRole('radio', { name: '通用 JSON' }).click(); await copy.click();
  await panel.getByText('配置已复制', { exact: true }).waitFor();
  const json = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  check(json === await panel.getByLabel('客户端配置内容').textContent(), 'installed JSON clipboard mismatch');
  const info = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('get_mcp_connection'));
  const config = JSON.parse(json).mcpServers.yingqu;
  check(config.command.includes('安装 Program Files') && !config.command.includes('target\\release'), 'config still points at a build tree');
  check(info.adapter_path === config.command && info.status.endpoint === config.args[1], 'installed config differs from live state');
  check(config.args.length === 2 && config.args[0] === '--endpoint', 'installed argument array changed');
  await page.screenshot({ path: '.local/wb207/installed-settings.png', fullPage: true });
  return { status: 'PASSED', installed: true, embeddedFrontend: page.url(), defaultOff: true, nativeClipboard: ['codex', 'json'], toml, json, info };
}
