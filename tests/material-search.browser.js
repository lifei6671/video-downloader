async page => {
  page.setDefaultTimeout(8000);
  const check = (value, message) => { if (!value) throw Error(message); };
  await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1');
  await page.getByRole('button', { name: '素材库', exact: true }).click();
  const ready = () => page.waitForFunction(() => document.querySelector('.library-writing')?.getAttribute('aria-busy') === 'false');
  async function search(query) {
    await page.getByRole('textbox', { name: '搜索素材库' }).fill(query);
    await page.getByRole('button', { name: '搜索', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.library-list')?.getAttribute('aria-busy') === 'false');
    await page.locator('[data-library-asset="a0"] .library-hit').click(); await ready();
  }
  await search('旧版本识别原文');
  check(await page.getByRole('textbox', { name: '原始文案内容' }).inputValue() === '旧版本识别原文😀', 'search did not open pinned historical edition');
  await page.getByRole('tab', { name: 'AI 总结', exact: true }).click(); await ready();
  await page.getByRole('textbox', { name: 'AI 总结内容', exact: true }).fill('只属于AI 总结搜索标记');
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('button', { name: '保存', exact: true, disabled: true }).waitFor();
  check(await page.evaluate(() => window.writingTest.snapshot('a0').notes.some(n => n.kind === 'copywriting' && n.body === '只属于AI 总结搜索标记')), 'new inspiration type or body lost');
  await page.getByRole('button', { name: '返回列表', exact: true }).click();
  await search('只属于AI 总结搜索标记');
  check(await page.getByRole('textbox', { name: 'AI 总结内容', exact: true }).inputValue() === '只属于AI 总结搜索标记', 'note search did not open matching note');

  return { passed: 1, cases: ['search opens exact historical document and note / existing summary remains editable'] };
}
