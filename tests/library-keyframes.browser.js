// Synthetic IPC / PNG resources. Actual FFmpeg timestamps and lifecycle are tested separately.
async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, text) => { if (!ok) throw Error(text); }, results = [];
  await page.addInitScript(() => {
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    window.frameUrls = { created: [], revoked: [] };
    URL.createObjectURL = blob => { const url = create(blob); window.frameUrls.created.push(url); return url; };
    URL.revokeObjectURL = url => { window.frameUrls.revoked.push(url); revoke(url); };
  });
  const detail = () => page.getByRole('region', { name: '视频资料详情', exact: true });
  const panel = () => page.getByRole('region', { name: '关键帧', exact: true });
  const ready = () => page.waitForFunction(() => document.querySelector('.library-keyframes')?.getAttribute('aria-busy') === 'false' && document.querySelector('.library-clips')?.getAttribute('aria-busy') === 'false');
  const load = async () => {
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1');
    await page.getByRole('button', { name: '知识库', exact: true }).click();
    await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click(); await ready();
    await page.evaluate(() => window.libraryDetailTest.seedClip('a0', 'f-a0', 500, 1250)); await ready();
  };
  const sample = async count => {
    await detail().getByRole('button', { name: /^提取关键帧 范围测试/ }).click();
    await page.getByRole('menuitem', { name: `提取 ${count} 帧`, exact: true }).click();
  };
  const images = () => panel().getByRole('button', { name: /^查看关键帧/ });
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); await load(); await sample(3);
    await panel().getByText('已保存 3 张关键帧', { exact: true }).waitFor(); await ready();
    await page.waitForFunction(() => document.querySelectorAll('.library-frame-image img').length === 3 && [...document.querySelectorAll('.library-frame-image img')].every(image => image.complete && image.naturalWidth === 1));
    check(await page.evaluate(() => { const rows = window.keyframesTest.snapshot(); return rows.length === 3 && rows[0].requested_ms === 625 && rows[0].actual_ms === 700 && rows.every(row => row.clip_id && row.sample_count === 3); }), 'uniform sampling metadata');
    await images().first().focus(); await page.keyboard.press('Enter'); await page.getByRole('dialog').getByRole('img', { name: /^关键帧 / }).waitFor();
    await page.keyboard.press('Escape'); await page.getByRole('dialog').waitFor({ state: 'hidden' });
    check(await images().first().evaluate(button => button === document.activeElement), 'preview close did not restore image trigger focus');
    const urls = await panel().locator('img').evaluateAll(imgs => imgs.map(image => image.src));
    await panel().scrollIntoViewIfNeeded();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
    await page.screenshot({ path: 'output/playwright/wb304-' + width + '.png', animations: 'disabled' });
    await page.getByRole('button', { name: '下载中心', exact: true }).click();
    check(await page.evaluate(urls => urls.every(url => window.frameUrls.revoked.includes(url)), urls), 'hidden gallery did not revoke URLs');
    await page.getByRole('button', { name: '知识库', exact: true }).click(); await ready();
    await images().first().waitFor();
    results.push(width + ': sampled images, exact metadata, image preview/Escape, no overflow, hidden URL release and return');
  }
  await load();
  await panel().getByRole('button', { name: '当前位置取帧', exact: true }).click();
  await panel().getByRole('alert').filter({ hasText: '请先播放视频' }).waitFor();
  await detail().getByRole('button', { name: '播放视频', exact: true }).click();
  await page.waitForFunction(() => { const v = document.querySelector('video'); return v && !v.paused && v.currentTime > 0; });
  await detail().locator('video').evaluate(v => { v.pause(); v.currentTime = .5; });
  await page.waitForFunction(() => !document.querySelector('video').seeking);
  await panel().getByRole('button', { name: '当前位置取帧', exact: true }).click(); await panel().getByText('已保存 1 张关键帧', { exact: true }).waitFor(); await ready();
  check(await page.evaluate(() => window.keyframesTest.calls.filter(c => c.method === 'start').at(-1).request.position_ms) === 500, 'current media position was not used');
  await page.evaluate(() => window.libraryDetailTest.changeFile('a0', 'f-a0', { missing: true })); await ready();
  await panel().getByText('源文件缺失', { exact: true }).waitFor(); await images().first().click(); await page.getByRole('dialog').getByRole('img', { name: /^关键帧 / }).waitFor(); await page.keyboard.press('Escape');
  check(await panel().getByRole('button', { name: '当前位置取帧', exact: true }).isDisabled(), 'missing source permits extraction');
  await panel().getByRole('button', { name: /^删除关键帧/ }).click(); await ready(); await panel().getByText('暂无关键帧', { exact: true }).waitFor();
  check(await page.evaluate(() => window.libraryDetailTest.clips().length) === 1, 'image deletion deleted clip');
  results.push('capture uses real media clock; missing source preserves PNG viewing and disables extraction; deletion only removes image');

  await load(); await page.evaluate(() => window.keyframesTest.mode('hold')); await sample(3);
  await panel().getByRole('button', { name: '取消提取关键帧', exact: true }).click(); await panel().getByText('本次关键帧提取已取消', { exact: true }).waitFor();
  check(await page.evaluate(() => window.keyframesTest.snapshot().length) === 0, 'canceled batch committed images');
  await page.evaluate(() => window.keyframesTest.mode('failure')); await sample(3);
  await panel().getByRole('alert').filter({ hasText: '采样位置没有可用画面' }).waitFor();
  check(await page.evaluate(() => window.keyframesTest.snapshot().length) === 0, 'failed batch committed images');
  await page.evaluate(() => window.keyframesTest.mode('success')); await sample(3); await panel().getByText('已保存 3 张关键帧', { exact: true }).waitFor(); await ready();
  await page.evaluate(() => window.keyframesTest.fail('delete')); await panel().getByRole('button', { name: /^删除关键帧/ }).first().click(); await panel().getByRole('alert').waitFor();
  check(await page.evaluate(() => window.keyframesTest.snapshot().length) === 3, 'failed delete removed resource');
  await panel().getByRole('button', { name: /^删除关键帧/ }).first().click(); await ready();
  check(await page.evaluate(() => window.keyframesTest.snapshot().length) === 2, 'delete retry did not work');
  results.push('cancel/failure preserve empty batch; retry completes; failed delete retains image and retries');

  await load(); await page.evaluate(() => window.keyframesTest.mode('hold-start')); await sample(3);
  await page.getByRole('button', { name: '下载中心', exact: true }).click();
  await page.evaluate(() => window.keyframesTest.releaseStart());
  await page.getByRole('button', { name: '知识库', exact: true }).click(); await ready();
  await page.waitForFunction(() => [...document.querySelectorAll('.library-keyframes button')].some(button => button.textContent === '取消提取关键帧' && !button.disabled));
  await panel().getByRole('button', { name: '取消提取关键帧', exact: true }).click(); await panel().getByText('本次关键帧提取已取消', { exact: true }).waitFor();
  await page.evaluate(() => { window.keyframesTest.mode('success'); window.keyframesTest.emitOld(); });
  await sample(3); await panel().getByText('已保存 3 张关键帧', { exact: true }).waitFor();
  check(await page.evaluate(() => window.keyframesTest.subscriptions()) === 1, 'duplicate keyframe subscriptions');
  results.push('late start response across navigation does not strand busy state; stale event cannot replace current run; one subscription');
  await load();
  await page.getByRole('button', { name: '下载中心', exact: true }).click();
  await page.evaluate(() => { window.keyframesTest.holdRead(); window.keyframesTest.mode('hold'); });
  await page.getByRole('button', { name: '知识库', exact: true }).click(); await ready();
  await sample(3);
  await panel().getByRole('button', { name: '取消提取关键帧', exact: true }).waitFor();
  await page.evaluate(() => window.keyframesTest.releaseRead());
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  check(await panel().getByRole('button', { name: '取消提取关键帧', exact: true }).isVisible(), 'delayed null cleared running operation');
  await panel().getByRole('button', { name: '取消提取关键帧', exact: true }).click();
  await panel().getByText('本次关键帧提取已取消', { exact: true }).waitFor();
  results.push('delayed initial null operation cannot overwrite a newer start/read');
  return { status: 'PASSED', nativeTauri: false, passed: results.length, cases: results };
}
