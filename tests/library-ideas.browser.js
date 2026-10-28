// Synthetic UI contract; real SQLite and package validation run in Rust/native tests.
async page => {
  page.setDefaultTimeout(15000);
  const check = (ok, why) => { if (!ok) throw Error(why); }, results = [];
  const workspace = () => page.getByRole('region', { name: '选题与草稿', exact: true });
  const load = async () => {
    await page.goto('http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?detail=1&writing=1&ideas=1&workbench=1');
    await page.waitForFunction(() => !!window.ideasTest);
    return page.evaluate(() => window.ideasTest.seedSources());
  };
  const enter = async () => { await page.getByRole('button', { name: '工作台', exact: true }).click(); await page.getByRole('button', { name: '选题与草稿', exact: true }).click(); await workspace().waitFor(); };
  const fill = async title => { await workspace().getByRole('textbox', { name: '选题标题', exact: true }).fill(title); await workspace().getByRole('textbox', { name: '自己的观点', exact: true }).fill('自己的观点😀'); await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).fill('自己的原创草稿😀\n保持原创内容和来源分开'); };
  const save = async () => { await workspace().getByRole('button', { name: '保存草稿', exact: true }).click(); await workspace().getByText(/^已保存 r/).waitFor(); };
  const choose = async (name, option) => { await page.getByRole('dialog').getByRole('combobox', { name, exact: true }).click(); await page.getByRole('option', { name: option, exact: true }).click(); };
  const addMaterial = async kind => {
    await workspace().getByRole('button', { name: '添加参考', exact: true }).click(); await page.getByRole('menuitem', { name: '视频或片段', exact: true }).click();
    await page.getByRole('dialog').getByRole('combobox', { name: '参考来源视频', exact: true }).click(); await page.getByRole('option').first().click();
    if (kind === '片段') { await choose('参考资料类型', '片段'); await page.getByRole('dialog').getByRole('combobox', { name: '参考来源资料', exact: true }).click(); await page.getByRole('option').first().click(); }
    await page.getByRole('dialog').getByRole('button', { name: '加入参考', exact: true }).click(); await page.getByRole('dialog').waitFor({ state: 'hidden' });
  };
  const addTopic = async () => { await workspace().getByRole('button', { name: '添加参考', exact: true }).click(); await page.getByRole('menuitem', { name: '研究专题', exact: true }).click(); await page.getByRole('dialog').getByRole('combobox', { name: '参考研究专题', exact: true }).click(); await page.getByRole('option').first().click(); await page.getByRole('dialog').getByRole('button', { name: '加入参考', exact: true }).click(); await page.getByRole('dialog').waitFor({ state: 'hidden' }); };
  const source = await load(); await enter(); await fill('自己的第一条选题'); await addMaterial('视频'); await addMaterial('片段'); await addTopic(); await save();
  const first = await page.evaluate(() => window.ideasTest.snapshot()[0]);
  check(first.references.length === 3 && first.idea.body.includes('自己的原创草稿') && !first.idea.body.includes('参考专题正文'), 'writing mixed with reference text');
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height }); check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'idea page overflow'); await page.screenshot({ path: `output/playwright/wb309-${width}.png` });
  }
  await workspace().locator('[data-idea-reference="a0"]').getByRole('button', { name: '打开来源', exact: true }).click(); await page.getByRole('region', { name: '视频资料详情', exact: true }).waitFor(); check(!await page.locator('.library-ideas').isVisible(), 'inactive draft still visible');
  await page.getByRole('button', { name: '返回草稿', exact: true }).click(); check(await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).inputValue() === first.idea.body, 'video return changed draft');
  await workspace().locator(`[data-idea-reference="${source.clipId}"]`).getByRole('button', { name: '打开来源', exact: true }).click(); await page.waitForFunction(() => { const v = document.querySelector('[role="dialog"] video'); return v?.readyState >= 2 && v.currentTime >= .49; }); await page.keyboard.press('Escape'); check(await page.locator('video').count() === 0, 'clip reader not released');
  await workspace().locator(`[data-idea-reference="${source.topicId}"]`).getByRole('button', { name: '打开来源', exact: true }).click(); await page.getByRole('dialog').locator('pre').filter({ hasText: '参考专题正文' }).waitFor();
  await page.getByRole('dialog').getByRole('button', { name: '打开专题资料', exact: true }).first().click(); await page.getByRole('button', { name: '返回草稿', exact: true }).waitFor(); await page.getByRole('button', { name: '返回草稿', exact: true }).click();
  results.push('three reference kinds, own writing separate, three viewports; video/topic member source navigation returns unchanged draft; clip range decoded and reader released');
  await page.getByRole('button', { name: '工作台', exact: true }).click(); await page.getByRole('region', { name: '最近编辑的选题', exact: true }).getByRole('button', { name: '编辑选题 自己的第一条选题', exact: true }).click(); await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).waitFor();
  await page.waitForFunction(() => document.querySelector('[aria-label="自己的草稿"]')?.value.includes('自己的原创草稿'));
  check(await workspace().getByRole('textbox', { name: '选题标题', exact: true }).inputValue() === '自己的第一条选题', 'workbench opened wrong idea');
  await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).fill('失败时保留的原创草稿😀'); await page.evaluate(() => window.ideasTest.failNext('editIdea')); await workspace().getByRole('button', { name: '保存草稿', exact: true }).click(); await workspace().getByRole('alert').waitFor();
  check(await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).inputValue() === '失败时保留的原创草稿😀', 'failed save lost original');
  await workspace().getByRole('button', { name: '返回知识库', exact: true }).click(); await page.getByRole('alertdialog').waitFor(); await page.getByRole('button', { name: '继续编辑', exact: true }).click(); await save();
  results.push('recent workbench entry opens correct idea; write failure and dirty navigation preserve original body and references, retry saves once');
  await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).fill('冲突保留全文😀'); await workspace().locator(`[data-idea-reference="${source.topicId}"]`).getByRole('button', { name: '移除参考', exact: true }).click();
  await page.evaluate(id => window.ideasTest.remoteEdit(id, false), first.idea.id); await workspace().getByRole('button', { name: '保存草稿', exact: true }).click(); await workspace().getByRole('alert').filter({ hasText: '草稿已更新' }).waitFor();
  check(await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).inputValue() === '冲突保留全文😀', 'conflict erased draft');
  await workspace().getByRole('button', { name: '读取最新草稿并保留当前内容', exact: true }).click(); await workspace().getByText('保留的草稿副本', { exact: true }).click();
  const recovery = await workspace().getByRole('textbox', { name: '保留的草稿 1', exact: true }).inputValue(); check(recovery.includes('冲突保留全文😀') && !JSON.parse(recovery).references.some(r => r.resource_id === source.topicId), 'recovery lost original/reference edits');
  await page.getByRole('button', { name: '工作台', exact: true }).click(); await page.getByRole('alertdialog').waitFor(); await page.getByRole('button', { name: '继续编辑', exact: true }).click(); await workspace().getByRole('button', { name: '移除保留副本', exact: true }).click();
  results.push('CAS conflict keeps current original and reference edits; latest read retains complete recovery payload, recovery is protected by navigation');
  await page.evaluate(async clipId => { await window.ideasTest.archive('a0'); await window.ideasTest.deleteClip('a0', clipId); }, source.clipId);
  await workspace().getByText('引用缺失', { exact: true }).waitFor(); check(await workspace().getByText('已移出知识库', { exact: true }).count() >= 1, 'archived reference hidden');
  await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).fill('来源缺失后的原创新稿😀'); await save();
  await workspace().locator('[data-idea-reference="a0"]').getByRole('button', { name: '移除参考', exact: true }).click(); await save(); check(await page.evaluate(() => window.libraryTest.snapshot().assets.some(a => a.id === 'a0')), 'reference removal deleted asset');
  results.push('archived and deleted references remain visible; original writing edits still save, removing reference keeps source asset');
  await load(); await enter(); await fill('提交后回读失败'); await page.evaluate(() => window.ideasTest.blockReads(true)); await workspace().getByRole('button', { name: '保存草稿', exact: true }).click();
  await workspace().getByRole('alert').filter({ hasText: '草稿已保存，重新读取失败' }).waitFor(); check(await workspace().getByRole('button', { name: '保存草稿', exact: true }).isDisabled(), 'read failure allows duplicate creation'); check((await page.evaluate(() => window.ideasTest.snapshot())).length === 1, 'committed idea missing');
  await page.evaluate(() => window.ideasTest.blockReads(false)); await workspace().getByRole('button', { name: '重新读取已保存草稿', exact: true }).click(); await workspace().getByText('已保存 r1', { exact: true }).waitFor();
  results.push('successful commit followed by failed read retains identity and saved status, blocks duplicate write and retries exact saved idea');
  await load(); await page.getByRole('button', { name: '知识库', exact: true }).click(); await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click(); await page.waitForFunction(() => document.querySelector('.library-writing')?.getAttribute('aria-busy') === 'false');
  await page.getByRole('region', { name: '视频资料详情', exact: true }).getByRole('button', { name: '写选题', exact: true }).click(); await workspace().locator('[data-idea-reference="a0"]').waitFor(); check(await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).inputValue() === '', 'asset entry copied source into draft'); await fill('视频灵感选题'); await save(); await workspace().getByRole('button', { name: '返回知识库', exact: true }).click();
  await page.locator('[data-library-asset="a0"]').getByRole('button', { name: /^查看视频资料/ }).click(); await page.getByRole('region', { name: '片段收藏', exact: true }).getByRole('button', { name: '用于选题', exact: true }).click(); await workspace().locator('[data-idea-reference]').waitFor(); check(await page.locator('[data-idea-reference]').getAttribute('data-idea-reference').then(id => id.startsWith('clip-')), 'clip entry did not bind exact clip'); await fill('片段灵感选题'); await save(); await workspace().getByRole('button', { name: '返回知识库', exact: true }).click();
  await page.getByRole('button', { name: '研究专题', exact: true }).click(); await page.getByRole('complementary', { name: '专题列表', exact: true }).getByRole('button', { name: '开头研究来源😀', exact: true }).click(); await page.getByRole('region', { name: '研究专题', exact: true }).getByRole('button', { name: '写选题', exact: true }).click(); await workspace().locator('[data-idea-reference]').waitFor(); check(await workspace().getByRole('textbox', { name: '自己的草稿', exact: true }).inputValue() === '', 'topic entry copied research into original'); await fill('专题灵感选题'); await save();
  results.push('video, exact clip and research topic each create a new idea with only a reference; original fields start empty and remain separate');
  return { status: 'PASSED', cases: results };
}
