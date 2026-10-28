// Production Tauri commands / SQLite / engine, only the marked WB306 synthetic profile.
async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9336');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB306 WebView2 not found'); p.setDefaultTimeout(15000);
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  const invoke = (command, args = {}) => p.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  check((await invoke('get_settings')).download_directory.includes('wb306-final'), 'not isolated WB306 profile');
  const assets = await invoke('search_library_assets', { request: {} });
  const assetId = assets.items.find(row => row.asset.video_id === '7300000000000000001').asset.id;
  await p.getByRole('button', { name: '知识库', exact: true }).click();
  const detail = p.getByRole('region', { name: '视频资料详情', exact: true });
  if (await detail.isVisible()) await detail.getByRole('button', { name: '返回列表', exact: true }).click();
  await p.locator('[data-library-asset="' + assetId + '"]').getByRole('button', { name: /^查看视频资料/ }).click();
  const writing = p.getByRole('region', { name: '文案与笔记', exact: true });
  await p.waitForFunction(() => document.querySelector('.library-writing')?.getAttribute('aria-busy') === 'false');
  const choose = async (name, option) => { await writing.getByRole('combobox', { name, exact: true }).click(); await p.getByRole('option', { name: option, exact: true }).click(); };
  const save = async () => { await writing.getByRole('button', { name: '保存', exact: true }).click(); await writing.getByRole('combobox', { name: '拆解内容来源', exact: true }).waitFor({ state: 'hidden' }); };
  const data = await invoke('get_library_asset', { assetId });
  const original = data.files.find(f => f.path.endsWith('原视频.mp4'));
  await detail.getByRole('combobox', { name: '文件版本', exact: true }).click(); await p.getByRole('option').filter({ hasText: '原视频.mp4' }).click();
  await writing.getByRole('textbox', { name: '识别原文全文', exact: true }).evaluate(el => { el.focus(); el.setSelectionRange(0, 6); document.dispatchEvent(new Event('selectionchange')); });
  await writing.getByRole('button', { name: '引用所选文字写笔记', exact: true }).click(); await choose('笔记类型', '文案拆解');
  for (const label of ['开头', '观点', '论据', '转折', '结尾', '可借鉴点']) await writing.getByRole('textbox', { name: `拆解${label}`, exact: true }).fill(`原生${label}😀`);
  await choose('拆解内容来源', 'AI 结果（手动录入）');
  check(await writing.getByRole('button', { name: '保存', exact: true }).isDisabled(), 'native AI can omit model');
  const marker = '原生模型记录' + Date.now(); await writing.getByRole('textbox', { name: '拆解模型名称', exact: true }).fill(marker); await save();
  const copyId = (await invoke('list_library_notes', { assetId })).find(note => note.model_name === marker).id;
  let note = await invoke('get_library_note', { assetId, noteId: copyId });
  check(note.origin === 'ai' && note.analysis.content.opening === '原生开头😀' && note.citations.length === 1 && note.citations[0].asset_id === assetId, 'native structure/provenance/citation');
  await writing.getByRole('textbox', { name: '拆解开头', exact: true }).fill('原生人工修订😀'); await save();
  await writing.getByText('已人工修订，保留 AI 创建来源', { exact: true }).waitFor();
  note = await invoke('get_library_note', { assetId, noteId: copyId });
  check(note.origin === 'ai' && note.model_name === marker && note.revision === 2 && note.analysis.human_revised, 'native AI model/origin changed after manual edit');
  await writing.getByRole('button', { name: '查看引用原文', exact: true }).click(); await p.getByRole('dialog').getByRole('textbox', { name: '引用文案全文', exact: true }).waitFor(); await p.keyboard.press('Escape');
  results.push('native six-field template, model-required AI manual entry, exact saved text citation and original reader; manual revision preserves AI origin/model');
  await detail.getByRole('button', { name: /^(播放视频|重新播放)$/ }).click();
  await p.waitForFunction(() => document.querySelector('video')?.readyState >= 2 && !document.querySelector('video').seeking);
  const playable = await detail.locator('video').evaluate(v => { v.pause(); return { start_ms: Math.ceil(v.seekable.start(0) * 1000), end_ms: Math.floor(Math.min(v.duration, v.seekable.end(0)) * 1000) }; });
  const clipId = await invoke('create_library_clip', { assetId, input: { file_id: original.id, title: '原生观察范围' + marker, reason: '', start_ms: 200, end_ms: 1200, playable, document_id: note.citations[0].document_id, quote: note.citations[0].quote } });
  const operation = await invoke('start_library_keyframes', { assetId, request: { kind: 'position', file_id: original.id, position_ms: 500 } });
  const completed = await p.evaluate(async id => {
    const deadline = performance.now() + 15000;
    while (performance.now() < deadline) {
      const value = await window.__TAURI_INTERNALS__.invoke('get_library_keyframe_operation');
      if (value?.id === id && value.state !== 'running') return value;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw Error('native extraction did not reach terminal state');
  }, operation.id);
  check(completed.state === 'completed', 'native extraction failed: ' + completed.error);
  await writing.getByRole('button', { name: '新建笔记', exact: true }).click(); await choose('笔记类型', '制作手法拆解');
  for (const [index, dimension, evidence] of [[1, '画面', '仅看关键帧'], [2, '字幕', '观看视频'], [3, '转场', '仅看关键帧'], [4, '声音', '聆听声音']]) {
    if (index > 1) await writing.getByRole('button', { name: '添加观察', exact: true }).click();
    await choose(`观察维度 ${index}`, dimension);
    await writing.getByRole('combobox', { name: `引用片段 ${index}`, exact: true }).click(); await p.getByRole('option').filter({ hasText: '原生观察范围' + marker }).click();
    await choose(`观察依据 ${index}`, evidence);
    if (evidence === '仅看关键帧') { await writing.getByRole('combobox', { name: `关联关键帧 ${index}`, exact: true }).click(); await p.getByRole('option').filter({ hasText: '00:00.500' }).first().click(); }
    await writing.getByRole('textbox', { name: `观察内容 ${index}`, exact: true }).fill(`原生${dimension}范围记录`);
  }
  await save();
  const technique = (await invoke('list_library_notes', { assetId })).filter(note => note.kind === 'technique_analysis').at(-1);
  const observed = await invoke('get_library_note', { assetId, noteId: technique.id });
  check(observed.origin === 'human' && observed.analysis.content.observations.length === 4 && observed.analysis.content.observations.every(row => row.file_id === original.id && row.clip_id === clipId && row.start_ms === 200 && row.end_ms === 1200), 'native temporal binding');
  await writing.getByRole('button', { name: '查看引用关键帧', exact: true }).first().click();
  const image = p.getByRole('dialog').getByRole('img', { name: '笔记引用关键帧', exact: true }); await image.waitFor(); await p.waitForFunction(() => document.querySelector('img[alt="笔记引用关键帧"]')?.naturalHeight === 160); check(await image.evaluate(img => img.complete && img.naturalHeight === 160), 'native reference PNG not decoded'); await p.keyboard.press('Escape');
  await writing.getByRole('button', { name: '播放引用片段', exact: true }).first().click(); await detail.getByText('片段播放完成', { exact: true }).waitFor();
  await writing.locator('.library-analysis-observations').scrollIntoViewIfNeeded(); await p.screenshot({ path: 'output/playwright/wb306-native.png', animations: 'disabled' });
  const bad = { content: { kind: 'technique', observations: [{ dimension: 'sound', evidence: 'keyframe', text: '不能从图像声称已听过声音', clip_id: clipId, keyframe_id: completed.frame_ids[0] }] }, origin: 'human', model_name: null, citations: [] };
  const failure = await p.evaluate(args => window.__TAURI_INTERNALS__.invoke('create_library_analysis', args).then(() => '', error => String(error)), { assetId, input: bad });
  check(failure === 'ANALYSIS_EVIDENCE_INVALID', 'native command accepted image-only sound');
  results.push('native four temporal dimensions and declared evidence, actual engine PNG reference and range playback; Core rejects image-only sound');
  await p.getByRole('button', { name: '下载中心', exact: true }).click(); await browser.close();
  return { status: 'PASSED', nativeTauri: true, cases: results, assetId, copyId, techniqueId: technique.id, clipId, frameId: completed.frame_ids[0] };
}
