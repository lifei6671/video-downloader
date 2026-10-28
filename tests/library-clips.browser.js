// UI uses synthetic IPC; video decoding and range playback use real Edge media.
async page => {
  page.setDefaultTimeout(8000);
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const detail = () => page.getByRole('region', { name: '视频资料详情', exact: true });
  const clips = () => page.getByRole('region', { name: '片段收藏', exact: true });
  const video = () => detail().locator('video');
  const ready = () => page.waitForFunction(() => document.querySelector('.library-writing')?.getAttribute('aria-busy') === 'false' && document.querySelector('.library-clips')?.getAttribute('aria-busy') === 'false');
  const load = async () => {
    if (await detail().isVisible()) {
      await detail().getByRole('button', { name: '返回列表', exact: true }).click();
      if (await page.getByRole('alertdialog').isVisible()) await page.getByRole('button', { name: '放弃修改', exact: true }).click();
    }
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1');
    await page.getByRole('button', { name: '知识库', exact: true }).click();
    await page.locator('[data-library-asset=a0]').getByRole('button', { name: /^查看视频资料/ }).click(); await ready();
  };
  const create = async () => { await clips().getByRole('button', { name: '收藏片段', exact: true }).click(); await clips().getByRole('button', { name: '标为开始', exact: true }).waitFor({ state: 'visible' }); await page.waitForFunction(() => [...document.querySelectorAll('.library-clips button')].some(b => b.textContent === '标为开始' && !b.disabled)); };
  const fill = async (start, end, title = '开场学习😀') => { await clips().getByRole('textbox', { name: '片段标题', exact: true }).fill(title); await clips().getByRole('textbox', { name: '片段开始时间', exact: true }).fill(start); await clips().getByRole('textbox', { name: '片段结束时间', exact: true }).fill(end); };
  const save = async () => { await clips().getByRole('button', { name: '保存片段', exact: true }).click(); await clips().locator('form').waitFor({ state: 'hidden' }); await ready(); };
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); await load(); await create();
    await video().evaluate(v => { v.pause(); v.currentTime = .5; });
    await page.waitForFunction(() => !document.querySelector('video').seeking);
    await clips().getByRole('button', { name: '标为开始', exact: true }).click();
    check(await clips().getByRole('textbox', { name: '片段开始时间', exact: true }).inputValue() === '0.500', 'manual start');
    await fill('0.500', '1.250'); await clips().getByRole('textbox', { name: '收藏理由', exact: true }).fill('学习开场与节奏'); await save();
    await clips().getByRole('button', { name: '播放片段', exact: true }).click();
    await detail().getByText('片段播放完成', { exact: true }).waitFor();
    const state = await video().evaluate(v => ({ time: v.currentTime, paused: v.paused }));
    check(state.paused && Math.abs(state.time - 1.25) < .05, 'did not stop at range end');
    const row = await page.evaluate(() => window.libraryDetailTest.clips()[0]);
    check(row.file_id === 'f-a0' && row.document_id === null && row.reason === '学习开场与节奏', 'manual untimed binding');
    await clips().scrollIntoViewIfNeeded();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
    await page.screenshot({ path: 'output/playwright/wb303-' + width + '.png', animations: 'disabled' });
    results.push(width + ': manual/untimed capture, save, range stop, viewport');
  }
  await load();
  await detail().getByRole('combobox', { name: '文案版本', exact: true }).click(); await page.getByRole('option', { name: 'v1 SenseVoiceSmall', exact: true }).click(); await ready();
  await detail().getByRole('textbox', { name: '识别原文全文', exact: true }).evaluate(el => { el.focus(); el.setSelectionRange(0, 6); el.dispatchEvent(new Event('select', { bubbles: true })); document.dispatchEvent(new Event('selectionchange')); });
  await detail().getByRole('button', { name: '引用所选文字收藏片段', exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll('.library-clips button')].some(b => b.textContent === '标为开始' && !b.disabled));
  await fill('0.2', '1.5', '固定旧文案'); await save();
  await clips().getByRole('button', { name: '查看引用原文 v1', exact: true }).click();
  check(await page.getByRole('textbox', { name: '引用文案全文', exact: true }).inputValue() === '旧版本识别原文😀', 'citation followed latest');
  await page.keyboard.press('Escape');
  check(await page.evaluate(() => window.libraryDetailTest.clips()[0].document_id) === 'old-a0', 'not pinned old document');
  results.push('saved text selection pins immutable v1, reader uses old content');

  await create(); await fill('0.5', '2');
  await detail().getByRole('button', { name: '返回列表', exact: true }).click();
  await page.getByRole('alertdialog').getByRole('button', { name: '继续编辑', exact: true }).click();
  check(await clips().getByRole('textbox', { name: '片段开始时间', exact: true }).inputValue() === '0.5', 'draft lost');
  await page.evaluate(() => window.libraryDetailTest.fail('createClip'));
  await clips().getByRole('button', { name: '保存片段', exact: true }).click(); await clips().getByRole('alert').waitFor();
  check(await clips().locator('form').isVisible(), 'save failure discarded draft');
  await save();
  await detail().getByRole('combobox', { name: '文件版本', exact: true }).click(); await page.getByRole('option', { name: '版本 2：竖屏样本.mp4', exact: true }).click();
  await clips().getByRole('button', { name: '播放片段', exact: true }).first().click();
  check(await detail().getByRole('combobox', { name: '文件版本', exact: true }).innerText().then(text => text.includes('版本 1')), 'clip did not select its file version');
  await detail().getByText('片段播放完成', { exact: true }).waitFor();
  await page.evaluate(() => window.libraryDetailTest.changeFile('a0', 'f-a0', { mtime_ns: 'changed' }));
  await clips().getByText('源文件已变化，无法按原范围播放', { exact: true }).first().waitFor();
  check(await clips().getByRole('button', { name: '播放片段', exact: true }).first().isDisabled(), 'changed source played');
  await clips().getByRole('button', { name: '查看引用原文 v1', exact: true }).click(); await page.getByRole('textbox', { name: '引用文案全文', exact: true }).waitFor(); await page.keyboard.press('Escape');
  results.push('draft navigation/save failure; clip selects bound file, changed source blocks playback and preserves citation');

  await load(); await page.evaluate(() => window.libraryDetailTest.mode('positive-pts')); await create();
  await clips().getByText(/^可播放范围：00:05.000–/).waitFor();
  await fill('0.2', '6'); check(await clips().getByRole('button', { name: '保存片段', exact: true }).isDisabled(), 'positive PTS head accepted');
  await fill('5.2', '20'); check(await clips().getByRole('button', { name: '保存片段', exact: true }).isDisabled(), 'unplayable tail accepted');
  await fill('5.2', '6.6', '正 PTS 范围'); await save();
  await clips().getByRole('button', { name: '播放片段', exact: true }).click(); await detail().getByText('片段播放完成', { exact: true }).waitFor();
  check(await video().evaluate(v => v.paused && Math.abs(v.currentTime - 6.6) < .05), 'positive PTS exact range stop');
  await page.evaluate(() => window.libraryDetailTest.seedClip('a0', 'f-a0', 5200, 20000)); await ready();
  await clips().getByRole('listitem').filter({ hasText: '范围测试' }).getByRole('button', { name: '播放片段', exact: true }).click();
  await detail().getByRole('alert').filter({ hasText: '片段无法完整播放' }).waitFor();
  check(await video().evaluate(v => v.paused), 'invalid range started');
  check(await page.evaluate(() => window.libraryDetailTest.clips().find(c => c.title.startsWith('范围测试')).end_ms) === 20000, 'silently shortened stored clip');
  results.push('real positive-PTS MP4 probes 5s lower bound; validates head/tail; plays 5.2–6.6s; rejects 20s stored tail without truncation');
  await page.evaluate(() => window.libraryDetailTest.changeFile('a0', 'f-a0', { missing: true }));
  await clips().getByText('源文件缺失', { exact: true }).first().waitFor();
  check(await clips().getByRole('button', { name: '播放片段', exact: true }).first().isDisabled(), 'missing clip playable');
  await clips().getByRole('button', { name: /^删除片段 正 PTS 范围$/ }).click(); await ready();
  check(await page.evaluate(() => window.libraryDetailTest.clips().length) === 1, 'delete did not remove reference only');
  results.push('missing source stays listed; explicit delete removes only clip reference');

  // Fault injection proves media event handling; it is separate from real MP4 observations.
  await load(); await page.evaluate(() => window.libraryDetailTest.seedClip('a0', 'f-a0', 200, 2000)); await ready();
  await clips().getByRole('button', { name: '播放片段', exact: true }).click();
  await page.waitForFunction(() => { const v = document.querySelector('video'); return v && !v.paused && !v.seeking; });
  await video().evaluate(v => { Object.defineProperty(v, 'duration', { configurable: true, value: 1 }); v.dispatchEvent(new Event('durationchange')); });
  await detail().getByRole('alert').filter({ hasText: '可播放范围已变化' }).waitFor();
  check(await video().evaluate(v => v.paused), 'range shrink did not pause');
  check(await detail().getByText('片段播放完成', { exact: true }).count() === 0, 'range shrink reported success');
  check(await page.evaluate(() => window.libraryDetailTest.clips()[0].end_ms) === 2000, 'range shrink rewrote saved endpoint');
  await load(); await page.evaluate(() => window.libraryDetailTest.seedClip('a0', 'f-a0', 200, 2000)); await ready();
  await clips().getByRole('button', { name: '播放片段', exact: true }).click();
  await page.waitForFunction(() => { const v = document.querySelector('video'); return v && !v.paused && !v.seeking; });
  await video().evaluate(v => { v.pause(); v.dispatchEvent(new Event('ended')); });
  await detail().getByRole('alert').filter({ hasText: '视频提前结束' }).waitFor();
  check(await detail().getByText('片段播放完成', { exact: true }).count() === 0, 'early ended reported success');
  results.push('media-event fault injection: live range shrink and early ended pause/report failure without rewriting clip');

  await load(); await detail().getByRole('tab', { name: '整理稿', exact: true }).click(); await ready();
  await detail().getByRole('textbox', { name: '整理稿内容', exact: true }).fill('两份草稿同时保留');
  await create(); await fill('0.2', '0.8', '同时保存的片段');
  await page.getByRole('button', { name: '下载中心', exact: true }).click();
  await page.getByRole('alertdialog').getByRole('button', { name: '保存后离开', exact: true }).click();
  await page.getByRole('heading', { name: '下载中心', exact: true }).waitFor();
  check(await page.evaluate(() => window.writingTest.snapshot('a0').documents.some(d => d.text === '两份草稿同时保留') && window.libraryDetailTest.clips().some(c => c.title === '同时保存的片段')), 'two editors did not both save');
  results.push('writing and clip drafts share navigation guard; save-and-leave commits both before navigating');
  return { status: 'PASSED', passed: results.length, cases: results, nativeTauri: false };
}
