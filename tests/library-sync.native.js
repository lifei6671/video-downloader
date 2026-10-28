// Actual Tauri commands / SQLite / asset protocol in the marked WB305 profile only.
async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9335');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB305 WebView2 not found'); p.setDefaultTimeout(12000);
  const invoke = (command, args = {}) => p.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  check((await invoke('get_settings')).download_directory.includes('wb305-final'), 'not isolated WB305 profile');
  const assets = await invoke('search_library_assets', { request: {} });
  const shortId = assets.items.find(row => row.asset.video_id === '7300000000000000001').asset.id;
  const longId = assets.items.find(row => row.asset.title === 'WB305 实测语音段长样本').asset.id;
  const detail = () => p.getByRole('region', { name: '视频资料详情', exact: true });
  const reader = () => p.getByRole('region', { name: '同步阅读', exact: true });
  const rows = () => reader().locator('[data-segment]');
  const open = async id => {
    await p.getByRole('button', { name: '知识库', exact: true }).click();
    if (await detail().isVisible()) await detail().getByRole('button', { name: '返回列表', exact: true }).click();
    await p.locator('[data-library-asset="' + id + '"]').getByRole('button', { name: /^查看视频资料/ }).click();
    await reader().waitFor();
  };
  const seekPaused = async seconds => {
    await detail().locator('video').evaluate((v, seconds) => { v.pause(); v.currentTime = seconds; }, seconds);
    await p.waitForFunction(() => !document.querySelector('video').seeking);
  };
  await open(shortId);
  const short = await invoke('get_library_asset', { assetId: shortId });
  const docId = short.documents.find(doc => doc.model_name === 'WB305 fixture').id;
  const doc = await invoke('get_library_document', { assetId: shortId, documentId: docId });
  check(doc.timing.segments.length === 2 && !doc.source_changed, 'native persisted source / timing');
  const sourceFile = short.files.find(file => file.id === doc.summary.file_id);
  if (!(await detail().getByRole('combobox', { name: '文件版本', exact: true }).innerText()).includes('原视频.mp4')) {
    await detail().getByRole('combobox', { name: '文件版本', exact: true }).click(); await p.getByRole('option').filter({ hasText: '原视频.mp4' }).click();
  }
  await rows().nth(1).click(); await p.waitForFunction(() => document.querySelector('video')?.currentTime >= .8 && !document.querySelector('video').paused);
  check((await detail().locator('video').getAttribute('src')).startsWith('http://asset.localhost/'), 'not real native media protocol');
  await seekPaused(1); check(await rows().nth(1).getAttribute('aria-current') === 'true', 'native segment follow');
  await seekPaused(.5); check(await reader().locator('[aria-current="true"]').count() === 0, 'native gap highlight');
  await reader().getByLabel('文案段落', { exact: true }).focus(); await p.keyboard.press('Space'); await p.waitForFunction(() => !document.querySelector('video').paused);
  await p.keyboard.press('Space'); await p.waitForFunction(() => document.querySelector('video').paused);
  await reader().getByRole('button', { name: '选取片段', exact: true }).click(); await rows().first().click(); await rows().nth(1).click();
  await reader().getByRole('button', { name: '收藏所选片段', exact: true }).click();
  const clips = p.getByRole('region', { name: '片段收藏', exact: true });
  check(await clips.getByRole('textbox', { name: '片段开始时间', exact: true }).inputValue() === '0.200', 'native selected start');
  check(await clips.getByRole('textbox', { name: '片段结束时间', exact: true }).inputValue() === '1.200', 'native selected end');
  await clips.getByRole('textbox', { name: '片段标题', exact: true }).fill('原生同步引用😀');
  await clips.getByRole('button', { name: '保存片段', exact: true }).click(); await clips.locator('form').waitFor({ state: 'hidden' });
  const clip = (await invoke('list_library_clips', { assetId: shortId })).find(clip => clip.title === '原生同步引用😀');
  check(clip.document_id === docId && clip.document_version === doc.summary.version && clip.quote === doc.text && clip.file_id === sourceFile.id, 'native immutable clip binding');
  results.push('native persisted timeline + real asset video: .8s jump, 1s highlight, .5s gap; scoped Space; saved .2–1.2s clip binds exact document/version/source/text');

  await open(longId);
  const long = await invoke('get_library_asset', { assetId: longId });
  const natural = await invoke('get_library_document', { assetId: longId, documentId: long.documents[0].id });
  check(natural.timing.segments.length === 79 && !natural.source_changed, 'long timing source');
  const index = natural.timing.segments.findIndex(s => s.start_ms > 200000), segment = natural.timing.segments[index];
  await rows().nth(index).click();
  await p.waitForFunction(start => { const v = document.querySelector('video'); return v && v.currentTime >= start / 1000 && !v.paused; }, segment.start_ms);
  await seekPaused((segment.start_ms + Math.min(200, (segment.end_ms - segment.start_ms) / 2)) / 1000);
  check(await rows().nth(index).getAttribute('aria-current') === 'true', 'actual VAD long follow');
  const container = reader().getByLabel('文案段落', { exact: true });
  await reader().getByRole('switch', { name: '自动滚动', exact: true }).click(); await container.evaluate(el => { el.scrollTop = 0; });
  const next = natural.timing.segments[index + 1]; await seekPaused((next.start_ms + 100) / 1000);
  check(await container.evaluate(el => el.scrollTop) === 0, 'native scroll-off moved text');
  await reader().getByRole('switch', { name: '自动滚动', exact: true }).click();
  check(await container.evaluate(el => el.scrollTop) > 0, 'native scroll-on did not follow');
  await reader().scrollIntoViewIfNeeded(); await p.screenshot({ path: 'output/playwright/wb305-native.png', animations: 'disabled' });
  await detail().locator('video').evaluate(v => { window.wb305Retired = v; });
  await p.getByRole('button', { name: '下载中心', exact: true }).click();
  check(await p.evaluate(() => window.wb305Retired.paused && !window.wb305Retired.hasAttribute('src')), 'native navigation did not release media');
  results.push('297.586s audio + synthetic video, reused 79 actual VAD cues: >200s seek/highlight, scroll off/on, media released on navigation');
  await browser.close();
  return { status: 'PASSED', nativeTauri: true, cases: results, selectedClip: clip.id, longSegments: 79 };
}
