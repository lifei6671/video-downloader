// Run headed with Playwright CLI against Vite to include native scrollbar arrows.
// All dependency IPC uses the isolated fixture.
async page => {
  page.setDefaultTimeout(7000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (value, message) => { if (!value) throw Error(message); };
  const area = page.getByRole('region', { name: '运行依赖', exact: true });
  const card = title => area.locator('[data-slot="card"]').filter({ has: page.locator('[data-slot="card-title"]').getByText(title, { exact: true }) });
  const calls = () => page.evaluate(() => window.backgroundTest.calls.filter(c => c.type === 'install-runtime'));
  async function load(scenario) {
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=' + scenario);
    await page.getByRole('button', { name: '系统设置', exact: true }).click();
    await page.getByRole('button', { name: '运行依赖', exact: true }).click();
    await card('FFmpeg').waitFor();
  }
  const dependencies = [['FFmpeg', '下载 FFmpeg', 'ffmpeg'], ['JavaScript 运行时', '下载 Deno', 'javascript'], ['语音转录模型 Fun-ASR-Nano', '下载语音模型', 'fun-asr-nano']];
  const cases = [];
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height });
    await load('runtime-missing');
    await card('语音转录模型 Fun-ASR-Nano').getByText('模型大小：约 1.2 GiB', { exact: true }).waitFor();
    const header = area.locator(':scope > header');
    const heading = header.getByRole('heading', { name: '运行依赖', exact: true });
    const recheck = header.getByRole('button', { name: '重新检测', exact: true });
    const policy = header.getByText('优先使用系统已有工具；语音模型仅在点击下载后保存到应用目录。', { exact: true });
    const fixedBefore = await Promise.all([heading, recheck, policy].map(item => item.boundingBox()));
    check(await area.locator('[data-slot="card"]:visible').count() === 4, 'dependency cards must be separate');
    check(await card('CUDA 可选加速').count()===0,'CUDA must not be a separate dependency');
    check(await area.getByRole('button', { name: '下载缺失依赖', exact: true }).count() === 0, 'ambiguous aggregate download label remains');
    for (const title of ['FFmpeg', 'JavaScript 运行时', 'tdl', '语音转录模型 Fun-ASR-Nano']) {
      const current = card(title);
      await current.scrollIntoViewIfNeeded();
      const geometry = await current.evaluate(element => {
        let scroll = element.parentElement;
        while (scroll && getComputedStyle(scroll).overflowY !== 'auto') scroll = scroll.parentElement;
        const r = element.getBoundingClientRect(), s = scroll.getBoundingClientRect();
        return { left: r.left - s.left, right: s.left + scroll.clientWidth - r.right, clipped: element.scrollHeight > element.clientHeight + 1 };
      });
      check(geometry.left >= 0.99 && geometry.right >= 11.99, title + ' ring or scrollbar spacing lost at ' + width);
      check(!geometry.clipped, title + ' card content clipped');
    }
    const fixedAfter = await Promise.all([heading, recheck, policy].map(item => item.boundingBox()));
    check(fixedAfter.every((box, index) => Math.abs(box.y - fixedBefore[index].y) < 1), 'runtime title/toolbar scrolled with cards');
    const scroller = area.locator(':scope > div:visible');
    await scroller.evaluate(element => { element.scrollTop = 0; });
    if (!await page.evaluate(() => navigator.userAgent.includes('HeadlessChrome'))) {
      const startTop = await scroller.evaluate(element => { element.scrollTop = Math.floor((element.scrollHeight - element.clientHeight) / 2); return element.scrollTop; });
      const bounds = await scroller.boundingBox();
      await page.mouse.move(bounds.x + bounds.width - 3, bounds.y + bounds.height - 5);
      await page.mouse.down();
      try { await page.waitForFunction(previous => document.querySelector('#settings-operation-area > div:not([hidden])').scrollTop > previous, startTop); } finally { await page.mouse.up(); }
      const down = await scroller.evaluate(element => element.scrollTop);
      await page.mouse.move(bounds.x + bounds.width - 3, bounds.y + 5);
      await page.mouse.down();
      try { await page.waitForFunction(previous => document.querySelector('#settings-operation-area > div:not([hidden])').scrollTop < previous, down); } finally { await page.mouse.up(); }
    }
    await recheck.click();
    await card('FFmpeg').getByRole('button', { name: '下载 FFmpeg', exact: true }).waitFor();
    await card('语音转录模型 Fun-ASR-Nano').scrollIntoViewIfNeeded();
    await page.screenshot({ path: '.local/runtime-ui/cards-bottom-' + width + '.png' });
    for (let index = 0; index < dependencies.length; index++) {
      const [title, label, id] = dependencies[index], current = card(title);
      const start = current.getByRole('button', { name: label, exact: true });
      await start.focus(); await page.keyboard.press('Enter');
      await current.getByRole('progressbar').waitFor();
      if (id === 'fun-asr-nano') check(await current.getByText('模型大小：约 1.2 GiB', { exact: true }).isVisible(), 'model size disappeared while downloading');
      const history = await calls();
      check(history.length === index + 1 && history[index].tool === id, 'download requested wrong or multiple dependencies');
      for (const [otherTitle, otherLabel] of dependencies.slice(index + 1)) {
        check(await card(otherTitle).getByRole('button', { name: otherLabel, exact: true }).isDisabled(), 'concurrent dependency download allowed');
        check(await card(otherTitle).getByRole('progressbar').count() === 0, 'progress leaked into another card');
      }
      await page.evaluate(() => window.backgroundTest.finishRuntimeInstall());
      await current.getByRole('progressbar').waitFor({ state: 'hidden' });
      await current.getByText('可用', { exact: true }).waitFor();
    }
    await card('FFmpeg').scrollIntoViewIfNeeded();
    await page.screenshot({ path: '.local/runtime-ui/cards-top-' + width + '.png' });
    await card('语音转录模型 Fun-ASR-Nano').getByText('CPU 本地识别',{exact:true}).waitFor();
    check(await card('语音转录模型 Fun-ASR-Nano').getByText('模型大小：约 1.2 GiB', { exact: true }).isVisible(), 'model size disappeared after installation');
    cases.push(width + 'x' + height + ': fixed title/toolbar, spacing/arrows, four intact cards, per-tool download, CPU version');
  }
  await load('runtime-mixed');
  check(await card('FFmpeg').getByRole('button', { name: '下载 FFmpeg', exact: true }).count() === 0, 'installed FFmpeg offered duplicate install');
  check(await card('JavaScript 运行时').getByRole('button', { name: '下载 Deno', exact: true }).count() === 0, 'installed Node offered Deno install');
  await card('语音转录模型 Fun-ASR-Nano').getByRole('button', { name: '重新下载模型', exact: true }).click();
  await card('语音转录模型 Fun-ASR-Nano').getByRole('button', { name: '取消下载', exact: true }).click();
  await area.getByRole('alert').filter({ hasText: '操作已取消' }).waitFor();
  check((await calls())[0].tool === 'fun-asr-nano', 'repair did not target model');
  check(await card('语音转录模型 Fun-ASR-Nano').getByRole('button', { name: '重新下载模型', exact: true }).isEnabled(), 'canceled repair lost prior installation');
  cases.push('installed tools preserved; model repair and cancellation remain local');
  await load('runtime-unsupported');
  check(await card('语音转录模型 Fun-ASR-Nano').getByRole('button', { name: '下载语音模型', exact: true }).isDisabled(), 'unsupported model install enabled');
  check(await card('FFmpeg').getByRole('button', { name: '下载 FFmpeg', exact: true }).isEnabled(), 'unsupported model disables unrelated tool');
  cases.push('unsupported model does not disable independent dependencies');
  for (const scenario of ['runtime-missing', 'runtime-mac']) {
    await load(scenario);
    const model = card('语音转录模型 Fun-ASR-Nano');
    check(await model.getByRole('combobox').count() === (scenario==='runtime-mac'?0:1), 'model versions do not match platform');
    check(await model.getByText(/1.2 GiB/).count() === 1, 'Nano download size is stale');
  }
  cases.push('Windows model version selector; macOS CPU only; correct model size');
  await load('runtime-size');
  await card('语音转录模型 Fun-ASR-Nano').getByText('模型大小：约 2.0 GiB', { exact: true }).waitFor();
  await load('runtime-size-unknown');
  await card('语音转录模型 Fun-ASR-Nano').getByText('模型大小：未知', { exact: true }).waitFor();
  cases.push('model size uses detected bytes; unavailable size remains unknown');
  return { passed: cases.length, cases, nativeArrowClicks: !await page.evaluate(() => navigator.userAgent.includes('HeadlessChrome')) };
}
