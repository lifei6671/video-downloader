// Production commands, SQLite and WebView2. Restricted to the marked synthetic WB307 profile.
async page => {
  const browser = await page.context().browser().browserType().connectOverCDP('http://127.0.0.1:9337');
  const p = browser.contexts()[0].pages().find(p => p.url() === 'http://127.0.0.1:1420/');
  if (!p) throw Error('WB307 WebView2 not found'); p.setDefaultTimeout(20000);
  const check = (ok, why) => { if (!ok) throw Error(why); }, results = [];
  const invoke = (command, args = {}) => p.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  check((await invoke('get_settings')).download_directory.includes('wb307-final'), 'wrong native profile');
  const found = await invoke('search_library_assets', { request: {} });
  const rows = found.items.filter(row => row.asset.video_id === '7300000000000000001' || row.asset.video_id === '7300000000000000002');
  check(rows.length === 2, 'expected two seeded video assets');
  rows.sort((a, b) => a.asset.video_id.localeCompare(b.asset.video_id));
  const ids = rows.map(row => row.asset.id);
  const a = ids[0], b = ids[1];
  // One analysis dimension intentionally absent for B. This performs no AI/network operation.
  const content = { kind: 'copywriting', opening: '原生对比开头😀', viewpoint: '', evidence: '', turn: '', ending: '', takeaways: '可借鉴点' };
  await invoke('create_library_analysis', { assetId: a, input: { content, origin: 'ai', model_name: '合成模型记录', citations: [] } });
  const previous = await invoke('list_library_notes', { assetId: a });
  const previousIds = previous.filter(n => n.kind === 'comparison').map(n => n.id);
  await p.getByRole('button', { name: '知识库', exact: true }).click();
  const oldWorkspace = p.getByRole('region', { name: '跨视频对比工作区', exact: true });
  if (await oldWorkspace.isVisible()) await oldWorkspace.getByRole('button', { name: '返回知识库', exact: true }).click();
  for (const id of ids) await p.locator(`[data-library-asset="${id}"]`).getByRole('checkbox').check();
  await p.getByRole('button', { name: '对比所选', exact: true }).click();
  await p.waitForFunction(ids => ids.every(id => document.querySelector(`[data-comparison-asset="${id}"]`)?.getAttribute('aria-busy') === 'false'), ids);
  check(await p.locator(`[data-comparison-asset="${a}"]`).innerText().then(t => t.includes('原生对比开头😀') && t.includes('AI / 合成模型记录')), 'native structure missing');
  check(await p.locator(`[data-comparison-asset="${b}"]`).innerText().then(t => t.includes('未知') && !t.includes('原生对比开头😀')), 'missing dimension borrowed');
  const title = '原生比较' + Date.now();
  await p.getByRole('textbox', { name: '比较标题', exact: true }).fill(title);
  await p.getByRole('textbox', { name: '比较结论 1', exact: true }).fill('两个来源独立，未知维度不作推测😀');
  for (const id of ids) await p.getByRole('checkbox', { name: `结论 1 来源 ${id}`, exact: true }).check();
  await p.getByRole('combobox', { name: '比较笔记所属视频', exact: true }).selectOption(a);
  await p.getByRole('button', { name: '保存比较笔记', exact: true }).click();
  await p.getByRole('dialog').getByRole('heading', { name: title, exact: true }).waitFor();
  const notes = await invoke('list_library_notes', { assetId: a });
  const id = notes.find(n => n.kind === 'comparison' && !previousIds.includes(n.id)).id;
  const saved = await invoke('get_library_note', { assetId: a, noteId: id });
  check(saved.origin === 'human' && saved.comparison.points[0].sources.length === 2 && saved.citations.length === 2, 'native note shape');
  for (const source of saved.comparison.points[0].sources) {
    check(source.document && saved.citations.some(c => c.asset_id === source.pin.asset_id && c.document_id === source.document.id), 'native citation mixed');
    const sourceLinks = p.getByRole('dialog').locator(`[data-comparison-source="${source.pin.asset_id}"]`);
    await sourceLinks.getByRole('button', { name: `原文 v${source.document.version}`, exact: true }).click();
    const original = await invoke('get_library_document', { assetId: source.pin.asset_id, documentId: source.document.id });
    await p.getByRole('dialog').filter({ hasText: '来源：' }).locator('pre').filter({ hasText: original.text }).waitFor();
    await p.keyboard.press('Escape');
  }
  await p.keyboard.press('Escape');
  results.push('native multi-select, source-bound structure and unknown dimensions; human comparison retains AI source/model; SQLite citations and exact original readers');
  const pins = saved.comparison.points[0].sources.map(s => s.pin);
  const wrong = JSON.parse(JSON.stringify(pins)); wrong[0].document_id = pins[1].document_id;
  const rejects = async (command, args) => { try { await invoke(command, args); throw Error('unexpected success'); } catch (e) { check(!String(e).includes('unexpected success'), 'foreign reference accepted'); } };
  await rejects('create_library_comparison', { assetId: a, input: { title: '不应写入', asset_ids: ids, points: [{ text: '错列', sources: wrong }] } });
  await invoke('edit_library_analysis', { assetId: a, noteId: saved.comparison.points[0].sources.find(s => s.pin.asset_id === a).pin.copywriting.id, content: { ...content, opening: '后续修订' }, expectedRevision: 1 });
  await rejects('create_library_comparison', { assetId: a, input: { title: '不应写入', asset_ids: ids, points: [{ text: '过期', sources: pins }] } });
  await invoke('edit_library_comparison', { assetId: a, noteId: id, input: { title, point_texts: ['原生修改结论，保留旧拆解引用😀'] }, expectedRevision: 1 });
  const edited = await invoke('get_library_note', { assetId: a, noteId: id });
  check(edited.revision === 2 && JSON.stringify(edited.comparison.points[0].sources) === JSON.stringify(saved.comparison.points[0].sources), 'native edit changed pinned source');
  check((await invoke('list_library_notes', { assetId: a })).filter(n => n.kind === 'comparison').length === previousIds.length + 1, 'failed save wrote note');
  results.push('production Core rejects foreign document and stale analysis; text edit CAS retains prior source snapshot and old revision');
  await p.screenshot({ path: 'output/playwright/wb307-native.png' });
  return { status: 'PASSED', nativeTauri: true, cases: results, assetId: a, noteId: id, note: edited };
}
