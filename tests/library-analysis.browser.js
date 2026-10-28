// UI contract with synthetic persistence; real video clock and PNG decoding.
async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  const writing = () => page.getByRole('region', { name: '文案与笔记', exact: true });
  const choose = async (name, option) => { await writing().getByRole('combobox', { name, exact: true }).click(); await page.getByRole('option', { name: option, exact: true }).click(); };
  const load = async () => {
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1');
    await page.waitForFunction(() => !!window.analysisTest);
    await page.getByRole('button', { name: '知识库', exact: true }).click();
    await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click();
    await page.waitForFunction(() => document.querySelector('.library-writing')?.getAttribute('aria-busy') === 'false');
  };
  const fresh = async kind => { await writing().getByRole('tab', { name: '笔记', exact: true }).click(); await writing().getByRole('button', { name: '新建笔记', exact: true }).click(); await choose('笔记类型', kind); };
  const save = async () => { await writing().getByRole('button', { name: '保存', exact: true }).click(); await writing().getByRole('combobox', { name: '拆解内容来源', exact: true }).waitFor({ state: 'hidden' }); };
  const abandon = async () => { await page.getByRole('button', { name: '下载中心', exact: true }).click(); if (await page.getByRole('alertdialog').isVisible()) await page.getByRole('button', { name: '放弃修改', exact: true }).click(); };
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); await load(); await fresh('文案拆解');
    for (const label of ['开头', '观点', '论据', '转折', '结尾', '可借鉴点']) await writing().getByRole('textbox', { name: `拆解${label}`, exact: true }).fill(label + '观察😀');
    await save();
    check(await page.evaluate(() => { const n = window.analysisTest.notes('a0')[0]; return n.origin === 'human' && n.model_name === null && n.analysis.content.opening === '开头观察😀' && n.body.includes('## 可借鉴点'); }), 'human template persistence');
    await writing().getByRole('textbox', { name: '拆解转折', exact: true }).scrollIntoViewIfNeeded();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
    await page.screenshot({ path: `output/playwright/wb306-${width}.png`, animations: 'disabled' });
    results.push(width + ': six fields, human provenance, saved structured body and viewport');
  }
  await load(); await fresh('文案拆解'); await choose('拆解内容来源', 'AI 结果（手动录入）');
  await writing().getByRole('textbox', { name: '拆解开头', exact: true }).fill('AI 结果的开头');
  check(await writing().getByRole('button', { name: '保存', exact: true }).isDisabled(), 'AI without model can save');
  await writing().getByRole('textbox', { name: '拆解模型名称', exact: true }).fill('合成模型');
  await page.evaluate(() => window.writingTest.failNext('note')); await save();
  await writing().getByRole('alert').filter({ hasText: '内容已保存，重新读取失败' }).waitFor();
  await writing().getByText('创建来源：未知', { exact: true }).waitFor();
  check(await page.evaluate(() => window.analysisTest.notes('a0').length) === 1, 'post-save read failure retried creation');
  await writing().getByRole('button', { name: '重新读取已保存内容', exact: true }).click();
  await writing().getByText('创建来源：AI / 合成模型', { exact: true }).waitFor();
  await writing().getByRole('textbox', { name: '拆解开头', exact: true }).fill('人工补充后的开头'); await save();
  await writing().getByText('已人工修订，保留 AI 创建来源', { exact: true }).waitFor();
  const aiId = await page.evaluate(() => { const n = window.analysisTest.notes('a0')[0]; if (n.origin !== 'ai' || n.model_name !== '合成模型' || !n.analysis.human_revised || n.revision !== 2) throw Error('AI attribution changed'); return n.id; });
  await writing().getByRole('textbox', { name: '拆解开头', exact: true }).fill('冲突时保留的本地拆解😀');
  await page.evaluate(id => window.analysisTest.remoteEdit(id), aiId);
  await writing().getByRole('button', { name: '保存', exact: true }).click();
  await writing().getByRole('alert').filter({ hasText: '内容已更新' }).waitFor();
  check(await writing().getByRole('textbox', { name: '拆解开头', exact: true }).inputValue() === '冲突时保留的本地拆解😀', 'conflict lost local fields');
  await writing().getByRole('button', { name: '重新读取', exact: true }).click();
  await writing().getByText('保留的未保存内容', { exact: true }).click();
  check(await writing().getByRole('textbox', { name: '保留内容 1', exact: true }).inputValue().then(text => text.includes('冲突时保留的本地拆解😀') && text.includes('合成模型')), 'recovered structured draft lost provenance');
  await abandon(); results.push('AI model required; post-save read failure shows unknown origin, retry keeps one record; manual edit preserves AI/model and marks revision; conflict retains structured fields/provenance');

  await load();
  const clipId = await page.evaluate(() => window.libraryDetailTest.seedClip('a0', 'f-a0', 500, 1250));
  await page.getByRole('region', { name: '视频资料详情', exact: true }).getByRole('button', { name: new RegExp('提取关键帧 范围测试 ' + clipId) }).click(); await page.getByRole('menuitem', { name: '提取 3 帧', exact: true }).click();
  await page.getByRole('region', { name: '关键帧', exact: true }).getByText('已保存 3 张关键帧', { exact: true }).waitFor();
  await fresh('制作手法拆解');
  const selectClip = async i => { await writing().getByRole('combobox', { name: `引用片段 ${i}`, exact: true }).click(); await page.getByRole('option').filter({ hasText: '范围测试 ' + clipId }).click(); };
  const selectFrame = async i => { await writing().getByRole('combobox', { name: `关联关键帧 ${i}`, exact: true }).click(); await page.getByRole('option').filter({ hasText: '00:00.700' }).click(); };
  await selectClip(1); await choose('观察依据 1', '仅看关键帧'); await selectFrame(1); await choose('观察维度 1', '声音');
  await writing().getByRole('alert').filter({ hasText: '声音观察不能仅依据关键帧' }).waitFor(); check(await writing().getByRole('button', { name: '保存', exact: true }).isDisabled(), 'image-only sound accepted');
  await choose('观察维度 1', '画面'); await writing().getByRole('textbox', { name: '观察内容 1', exact: true }).fill('画面构图与主体关系');
  for (const [index, dimension, evidence] of [[2, '字幕', '观看视频'], [3, '转场', '仅看关键帧'], [4, '声音', '聆听声音']]) {
    await writing().getByRole('button', { name: '添加观察', exact: true }).click(); await choose(`观察维度 ${index}`, dimension); await selectClip(index); await choose(`观察依据 ${index}`, evidence); if (evidence === '仅看关键帧') await selectFrame(index);
    await writing().getByRole('textbox', { name: `观察内容 ${index}`, exact: true }).fill(`${dimension}的范围观察`);
  }
  await save();
  check(await page.evaluate(() => { const n = window.analysisTest.notes('a0')[0]; return n.analysis.content.observations.length === 4 && n.analysis.content.observations.every(row => row.start_ms === 500 && row.end_ms === 1250 && row.file_id === 'f-a0'); }), 'observation range binding');
  await page.waitForFunction(() => document.querySelector('.library-analysis-observations')?.getAttribute('aria-busy') === 'false' && document.querySelector('.library-keyframes')?.getAttribute('aria-busy') === 'false');
  await writing().getByRole('tab', { name: '识别原文', exact: true }).click();
  await writing().getByRole('tab', { name: '笔记', exact: true }).click();
  await writing().getByRole('textbox', { name: '笔记内容', exact: true }).waitFor();
  await page.evaluate(() => window.analysisTest.failResources(true));
  await choose('选择笔记', /制作手法拆解/);
  await writing().getByRole('button', { name: '重新读取引用资源', exact: true }).waitFor();
  await writing().getByRole('combobox', { name: '引用片段 1', exact: true }).click();
  await page.getByRole('option').filter({ hasText: '尚未读取' }).waitFor();
  check(await page.getByRole('option').filter({ hasText: '已删除' }).count() === 0, 'read failure called existing clip deleted'); await page.keyboard.press('Escape');
  await writing().getByRole('combobox', { name: '关联关键帧 1', exact: true }).click();
  await page.getByRole('option').filter({ hasText: '尚未读取' }).waitFor();
  check(await page.getByRole('option').filter({ hasText: '已删除' }).count() === 0, 'read failure called existing image deleted'); await page.keyboard.press('Escape');
  await page.evaluate(() => window.analysisTest.failResources(false));
  await writing().getByRole('button', { name: '重新读取引用资源', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.library-analysis-observations')?.getAttribute('aria-busy') === 'false');
  await writing().getByRole('button', { name: '查看引用关键帧', exact: true }).first().click(); await page.getByRole('dialog').getByRole('img', { name: '笔记引用关键帧', exact: true }).waitFor(); await page.keyboard.press('Escape');
  await writing().getByRole('button', { name: '播放引用片段', exact: true }).first().click(); await page.getByText('片段播放完成', { exact: true }).waitFor();
  await page.evaluate(() => window.libraryDetailTest.changeFile('a0', 'f-a0', { missing: true }));
  await writing().getByText('源文件缺失', { exact: true }).first().waitFor(); check(await writing().getByRole('button', { name: '播放引用片段', exact: true }).first().isDisabled(), 'missing source playable');
  await writing().getByRole('button', { name: '查看引用关键帧', exact: true }).first().click(); await page.getByRole('dialog').getByRole('img', { name: '笔记引用关键帧', exact: true }).waitFor(); await page.keyboard.press('Escape');
  await page.evaluate(clip => window.analysisTest.dropClip('a0', clip), clipId);
  await writing().getByText('片段已删除，保留原引用', { exact: true }).first().waitFor(); await writing().getByText('关键帧已删除，保留原引用', { exact: true }).first().waitFor();
  await writing().getByRole('textbox', { name: '观察内容 1', exact: true }).fill('资源删除后仍可补充历史观察'); await save();
  results.push('four dimensions with explicit evidence; image-only sound rejected; failed resource read is unknown, not deleted, and retries; PNG preview/range playback; missing/deleted resources retain snapshots and allow text edits');

  await load();
  await writing().getByRole('textbox', { name: '识别原文全文', exact: true }).evaluate(el => { el.focus(); el.setSelectionRange(0, 6); el.dispatchEvent(new Event('select', { bubbles: true })); document.dispatchEvent(new Event('selectionchange')); });
  await writing().getByRole('button', { name: '引用所选文字写笔记', exact: true }).click(); await choose('笔记类型', '文案拆解');
  await writing().getByRole('textbox', { name: '拆解开头', exact: true }).fill('基于已保存版本的开头学习'); await page.evaluate(() => window.writingTest.failNext('createAnalysis'));
  await writing().getByRole('button', { name: '保存', exact: true }).click(); await writing().getByRole('alert').waitFor();
  check(await writing().getByRole('textbox', { name: '拆解开头', exact: true }).inputValue() === '基于已保存版本的开头学习', 'failed save cleared input');
  check(await page.evaluate(() => window.analysisTest.notes('a0').length) === 0, 'failed create stored note');
  await save(); await writing().getByRole('button', { name: '查看引用原文', exact: true }).click(); await page.getByRole('dialog').getByRole('textbox', { name: '引用文案全文', exact: true }).waitFor(); await page.keyboard.press('Escape');
  check(await page.evaluate(() => window.analysisTest.notes('a0')[0].citations[0].document_id) === 'd-a0', 'citation not bound to selected document');
  results.push('saved-version text citation opens original; failed save retains fields and retry creates exactly one note');
  return { status: 'PASSED', nativeTauri: false, passed: results.length, cases: results };
}
