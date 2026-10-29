async page => {
  page.setDefaultTimeout(8000);
  await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1');
  await page.getByRole('button', { name: '素材库', exact: true }).click();
  await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click();
  await page.getByText('范围采样', { exact: true }).click();
  await page.getByRole('spinbutton', { name: '采样开始秒', exact: true }).fill('1');
  await page.getByRole('spinbutton', { name: '采样结束秒', exact: true }).fill('0');
  await page.getByRole('button', { name: '采样画面', exact: true }).click();
  await page.getByText('请输入有效的采样范围', { exact: true }).waitFor();
  await page.getByRole('spinbutton', { name: '采样开始秒', exact: true }).fill('0');
  await page.getByRole('spinbutton', { name: '采样结束秒', exact: true }).fill('1.5');
  await page.getByRole('button', { name: '采样画面', exact: true }).click();
  await page.getByText('已保存 5 张关键帧', { exact: true }).waitFor();
  if (!await page.evaluate(() => window.keyframesTest.snapshot().length === 5 && window.keyframesTest.calls.some(c => c.request?.kind === 'range' && c.request.start_ms === 0 && c.request.end_ms === 1500 && c.request.count === 5))) throw Error('range request/frames mismatch');
  return { passed: 1, cases: ['range inputs / invalid feedback / default five real PNG fixture frames'] };
}
