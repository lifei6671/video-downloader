async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.getByRole('button', { name: '系统设置', exact: true }).click();
  await page.getByRole('navigation', { name: '设置分类' }).getByRole('button', { name: 'MCP', exact: true }).click();
  const panel = page.locator('.mcp-settings');
  const toggle = panel.getByRole('switch', { name: '启用 MCP' });
  const copy = panel.getByRole('button', { name: '复制配置', exact: true });
  await panel.getByText('已关闭', { exact: true }).waitFor();
  check(await toggle.getAttribute('aria-checked') === 'false' && await copy.isDisabled(), 'native default is not off');
  const unavailable = await page.evaluate(async () => { try { await window.__TAURI_INTERNALS__.invoke('copy_mcp_client_config', { format: 'json' }); return null; } catch (error) { return String(error); } });
  check(unavailable === 'MCP_CONFIG_UNAVAILABLE', 'disabled backend copy accepted');
  await toggle.click(); await panel.getByText('运行中', { exact: true }).waitFor();
  await copy.click(); await panel.getByText('配置已复制', { exact: true }).waitFor();
  const toml = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  check(toml === await panel.getByLabel('客户端配置内容').textContent(), 'native TOML clipboard differs from preview');
  await panel.getByRole('radio', { name: '通用 JSON' }).click(); await copy.click();
  await panel.getByText('配置已复制', { exact: true }).waitFor();
  const json = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('read_clipboard_text'));
  check(json === await panel.getByLabel('客户端配置内容').textContent(), 'native JSON clipboard differs from preview');
  const info = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('get_mcp_connection'));
  const config = JSON.parse(json).mcpServers.yingqu;
  check(config.command.includes('中文 App With Spaces') && config.command.endsWith('yingqu-mcp.exe'), 'not actual installed space path');
  check(config.command === info.adapter_path && config.args[1] === info.status.endpoint && config.args.length === 2, 'config does not match live endpoint');
  check(!config.command.startsWith('"'), 'command contains shell quotes');
  await page.screenshot({ path: '.local/wb206/native-settings.png', fullPage: true });
  return { status: 'PASSED', defaultOff: true, nativeClipboard: ['codex', 'json'], toml, json, info };
}
