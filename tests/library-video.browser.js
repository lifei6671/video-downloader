// Real VP8/WebM and H.264/MP4 decoding, isolated native calls, generated media only.
async page => {
  page.setDefaultTimeout(8000);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const detail = () => page.getByRole('region', { name: '视频资料详情', exact: true });
  const video = () => detail().locator('video');
  const readyDetail = () => page.waitForFunction(() => document.querySelector('.library-asset-detail')?.getAttribute('aria-busy') === 'false');
  const readyVideo = () => page.waitForFunction(() => document.querySelector('.library-asset-detail video')?.readyState >= 2);
  const load = async () => { await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1'); await page.getByRole('button', { name: '知识库', exact: true }).click(); await page.waitForFunction(() => document.querySelector('.library-list')?.getAttribute('aria-busy') === 'false'); };
  const open = async (id = 'a0') => { await page.locator('[data-library-asset="' + id + '"]').getByRole('button', { name: /^查看视频资料/ }).click(); await readyDetail(); };
  const play = async () => { await detail().getByRole('button', { name: /^(播放视频|重新播放)$/ }).click(); };
  const capture = async () => video().evaluate(element => { window.retiredLibraryVideo = element; });
  const released = () => page.waitForFunction(() => window.retiredLibraryVideo.paused && !window.retiredLibraryVideo.hasAttribute('src'));
  const ranges = [];
  const recordRange = request => { if (request.url().endsWith('/player.webm')) ranges.push(request.headers().range); };
  page.on('request', recordRange);
  await page.route('**/unsupported.webm', route => route.fulfill({ status: 200, contentType: 'video/webm', body: 'not a playable video' }));
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); await load(); await open(); await play(); await readyVideo();
    const decoded = await video().evaluate(async element => { await element.play(); element.pause(); element.currentTime = 1.5; element.volume = 0.3; return { width: element.videoWidth, height: element.videoHeight, controls: element.controls, fit: getComputedStyle(element).objectFit }; });
    await page.waitForFunction(() => { const v = document.querySelector('.library-asset-detail video'); return v && !v.seeking && Math.abs(v.currentTime - 1.5) < 0.1; });
    check(decoded.width === 160 && decoded.height === 90 && decoded.controls && decoded.fit === 'contain', 'landscape decode/controls/contain');
    check(await video().evaluate(v => v.paused && Math.abs(v.volume - 0.3) < 0.01), 'pause/volume failed');
    await capture();
    await detail().getByRole('combobox', { name: '文件版本' }).click(); await page.getByRole('option', { name: '版本 2：竖屏样本.mp4', exact: true }).click();
    await released(); check(await video().count() === 0, 'file switch kept old media');
    await play(); await readyVideo();
    check(await video().evaluate(v => v.videoWidth === 90 && v.videoHeight === 160 && v.controls && getComputedStyle(v).objectFit === 'contain'), 'portrait H264 decode/fit');
    check(await page.evaluate(() => window.libraryDetailTest.calls.some(call => call.method === 'playback' && call.assetId === 'a0' && call.fileId === 'portrait')), 'wrong file playback IDs');
    await video().evaluate(v => { v.pause(); });
    await page.screenshot({ path: '.local/wb114-playback-' + width + '.png', animations: 'disabled' });
    await capture(); await page.getByRole('button', { name: '下载中心', exact: true }).click(); await released();
    await page.getByRole('button', { name: '知识库', exact: true }).click(); await readyDetail();
    check(await video().count() === 0, 'return navigation started playback automatically');
    await play(); await readyVideo(); await capture();
    await detail().getByRole('button', { name: '返回列表', exact: true }).click(); await released();
    check(await page.evaluate(() => window.libraryDetailTest.calls.every(call => call.method !== 'open')), 'normal decode auto-launched system player');
    results.push(width + ': real VP8/H264 decode/seek/volume/contain/file IDs/file switch/back/navigation release');
  }
  check(ranges.some(range => range?.startsWith('bytes=')), 'browser made no media byte-range request');
  await load(); await open(); await page.evaluate(() => window.libraryDetailTest.mode('unsupported')); await play();
  await detail().getByRole('alert').filter({ hasText: '内置播放器无法播放此视频' }).waitFor();
  check(await page.evaluate(() => !window.libraryDetailTest.calls.some(call => call.method === 'open')), 'codec failure auto-launched system application');
  await capture(); await detail().getByRole('button', { name: '系统播放器', exact: true }).click(); await released();
  check(await page.evaluate(() => window.libraryDetailTest.calls.some(call => call.method === 'open' && call.assetId === 'a0' && call.fileId === 'f-a0' && !('path' in call))), 'fallback did not use owned file IDs');
  results.push('decode failure visible; manual system fallback uses asset/file IDs and releases video');

  await load(); await open(); await page.evaluate(() => window.libraryDetailTest.mode('audio')); await play();
  await detail().getByRole('alert').filter({ hasText: '无法显示视频画面' }).waitFor();
  check(await video().evaluate(v => v.paused), 'audio-only stream kept playing');
  await load(); await open(); await page.evaluate(() => window.libraryDetailTest.mode('prepare-error')); await play();
  await detail().getByRole('alert').filter({ hasText: '无法读取本地视频，请尝试系统播放器' }).waitFor();
  check(await page.evaluate(() => !window.libraryDetailTest.calls.some(call => call.method === 'open')), 'prepare error auto-launched external');
  results.push('audio-only/prepare errors keep manual fallback available');

  await load(); await open(); await page.evaluate(() => window.libraryDetailTest.mode('missing')); await play();
  await detail().getByText('文件缺失', { exact: true }).waitFor();
  check(await detail().getByRole('button', { name: '播放视频', exact: true }).isDisabled(), 'prepare-time missing source did not refresh file state');
  check(await detail().getByRole('list', { name: '文案版本摘要' }).isVisible(), 'missing source removed materials');
  await load(); await open(); await play(); await readyVideo(); await capture();
  await page.evaluate(() => window.libraryDetailTest.changeFile('a0', 'f-a0', { missing: true }));
  await detail().getByText('文件缺失', { exact: true }).waitFor(); await released();
  results.push('prepare-time missing and live missing event update file status and stop media');

  await load(); await open(); await page.evaluate(() => window.libraryDetailTest.mode('hold')); await play(); await capture();
  await detail().getByRole('button', { name: '返回列表', exact: true }).click();
  await page.evaluate(() => window.libraryDetailTest.releaseSources()); await released();
  check(await page.locator('.library-asset-detail video').count() === 0, 'late source reopened player');
  await page.evaluate(() => window.libraryDetailTest.holdDetails());
  await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click();
  await detail().getByRole('button', { name: '返回列表', exact: true }).click();
  await page.locator('[data-library-asset="a1"]').getByRole('button', { name: /^查看视频资料/ }).click();
  await page.evaluate(() => window.libraryDetailTest.releaseDetails()); await readyDetail();
  await detail().getByRole('heading', { name: '含有100%_字面的文案', exact: true }).waitFor();
  check(await detail().innerText().then(text => !text.includes('source-a0')), 'stale detail overwrote new asset');
  results.push('late playback URL after back and stale detail after asset switch are ignored');
  page.off('request', recordRange);
  await page.unroute('**/unsupported.webm');
  return { passed: results.length, cases: results };
}
