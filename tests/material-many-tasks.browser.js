async page => {
  await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?many-materials=1');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  if (await page.getByRole('button', { name: '切换为信息流', exact: true }).isVisible()) await page.getByRole('button', { name: '切换为信息流', exact: true }).click();
  await page.locator('[data-task-id="task-10000"]').getByRole('button', { name: '收藏', exact: true }).waitFor({ state: 'visible' });
  await page.waitForFunction(() => { const calls = window.libraryTest.calls.filter(c => c.method === 'collectionBatch'); return calls.length >= 2 && [...document.querySelectorAll('[data-task-id] button')].some(b => b.getAttribute('aria-label') === '收藏' && !b.disabled); });
  const sizes = await page.evaluate(() => window.libraryTest.calls.filter(c => c.method === 'collectionBatch').map(c => c.ids.length));
  if (!sizes.includes(10000) || !sizes.includes(1) || sizes.some(n => n > 10000)) throw Error('task collection batching exceeded the API limit');
  return { passed: 1, cases: ['10001 completed tasks remain collectible using bounded batch queries'] };
}
