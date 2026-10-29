async page => {
  page.setDefaultTimeout(8000);
  const check = (ok, text) => { if (!ok) throw Error(text); };
  const panel = page.locator('.mcp-settings');
  const service = panel.getByRole('switch', { name: '启用 MCP' });
  const writing = panel.getByRole('switch', { name: '允许 AI 写入' });
  const results = [];
  async function load() {
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?mcp=1&ideas=1&detail=1&writing=1&workbench=1');
    await page.getByRole('button', { name: '系统设置', exact: true }).click();
    await page.getByRole('navigation', { name: '设置分类' }).getByRole('button', { name: 'MCP', exact: true }).click();
    await panel.getByText('已关闭', { exact: true }).waitFor();
  }
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); await load();
    check(await writing.isDisabled() && await writing.getAttribute('aria-checked') === 'false', 'writing must start disabled');
    await service.click(); await panel.getByText('运行中', { exact: true }).waitFor();
    check(await writing.isEnabled() && await writing.getAttribute('aria-checked') === 'false', 'starting service granted writing');
    await writing.focus(); await page.keyboard.press('Space'); await panel.getByText('已允许写入', { exact: true }).waitFor();
    await service.click(); await panel.getByText('已关闭', { exact: true }).waitFor();
    await service.click(); await panel.getByText('运行中', { exact: true }).waitFor();
    check(await writing.getAttribute('aria-checked') === 'false', 'service restart retained writing');
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
    await page.screenshot({ path: `.local/wb310/settings-${width}.png`, fullPage: true });
    results.push({ width, height, defaultAndKeyboardAndServiceReset: 'PASS' });
  }
  await page.evaluate(() => window.mcpTest.hold('write')); await writing.click();
  await panel.getByText('正在更新权限…', { exact: true }).waitFor();
  check(await writing.isDisabled() && await service.isDisabled(), 'pending permission controls active');
  await page.evaluate(() => window.mcpTest.release()); await panel.getByText('已允许写入', { exact: true }).waitFor();
  await page.evaluate(() => { window.mcpTest.hold('read'); window.dispatchEvent(new Event('focus')); });
  await writing.click(); await panel.getByText('仅允许读取', { exact: true }).waitFor();
  await page.evaluate(() => window.mcpTest.release());
  check(await writing.getAttribute('aria-checked') === 'false', 'stale read revived write authorization');
  await page.evaluate(() => window.mcpTest.failNext('MCP_SERVICE_DISABLED')); await writing.click();
  await panel.getByRole('alert').getByText('请先开启 MCP 服务。', { exact: true }).waitFor();
  await writing.click(); await panel.getByText('已允许写入', { exact: true }).waitFor();
  await page.evaluate(() => window.mcpTest.stop()); await panel.getByText('服务异常', { exact: true }).waitFor();
  check(await writing.isDisabled() && await writing.getAttribute('aria-checked') === 'false', 'failed listener retained write UI');
  results.push({ pendingFailureStaleReadsAndListenerFailure: 'PASS' });
  // Topic/idea navigation was retired from the default UI. Their persistence and
  // provenance remain covered by Core tests; this suite verifies the current permission UI.
  return results;
}
