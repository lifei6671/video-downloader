// Serve .local/csp-audit/dist with vite preview (build command in docs/development.md).
async (page) => {
  const target = await page.context().newPage();
  const errors = [];
  target.on('pageerror', error => errors.push(error.message));
  await target.emulateMedia({ reducedMotion: 'reduce' });
  await target.route('**/*', async route => {
    if (!route.request().isNavigationRequest()) return route.continue();
    const response = await route.fetch();
    const body = (await response.text()).replace('<style>', '<style nonce="csp-fixture-nonce">');
    await route.fulfill({ response, body, headers: { ...response.headers(), 'content-security-policy': "default-src 'self'; script-src 'self'; style-src 'self' 'nonce-csp-fixture-nonce'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'" } });
  });
  const results = [];
  try {
    const origin = page.url().split('/').slice(0, 3).join('/');
    await target.goto(origin + '/tests/fixtures/csp-components.html');
    const untrustedStyleBlocked = await target.evaluate(() => {
      const style = document.createElement('style');
      style.textContent = '.untrusted-fixture-style { color: red; }';
      document.head.append(style);
      const blocked = !style.sheet;
      style.remove();
      return blocked;
    });
    if (!untrustedStyleBlocked) throw new Error('Fixture CSP did not reject untrusted styles');
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await target.setViewportSize({ width, height });
      for (let n = 0; n < 3; n++) {
        await target.locator('#backup-key:not(:disabled)').waitFor();
        if (await target.locator('#backup-key').inputValue() !== 'fixture-key') throw new Error('Credentials lost on mount');
        if (await target.getByRole('combobox', { name: '备份提供商' }).textContent() !== '通用 S3') throw new Error('Provider cleared by form event');
        if (await target.locator('[data-sonner-toast]').count()) throw new Error('Unexpected toast while opening saved space');
        await target.getByRole('button', { name: '切换备份表单' }).click();
        await target.getByRole('button', { name: '切换备份表单' }).click();
      }
      if ((await target.evaluate(() => window.fixtureCalls)).length) throw new Error('Mount switched backup space');
      await target.getByRole('tab', { name: 'S3 / space-one', exact: true }).dblclick();
      const nameEditor = target.getByRole('textbox', { name: '编辑空间名称', exact: true });
      await nameEditor.waitFor();
      const inlineStyle = await nameEditor.evaluate(e => ({ position: getComputedStyle(e).position, font: getComputedStyle(e).fontSize, height: e.getBoundingClientRect().height }));
      if (inlineStyle.position !== 'absolute' || inlineStyle.font !== '12px' || inlineStyle.height < 12 || inlineStyle.height > 32) throw new Error('Inline editor CSP styles missing');
      await target.keyboard.press('Escape');
      await target.getByRole('combobox', { name: '备份提供商', exact: true }).click();
      await target.getByRole('option', { name: '通用 S3', exact: true }).waitFor();
      const select = await target.evaluate(() => ({ scrollbars: getComputedStyle(document.querySelector('[data-radix-select-viewport]')).scrollbarWidth, lock: getComputedStyle(document.body).overflow }));
      if (select.scrollbars !== 'none' || select.lock !== 'hidden') throw new Error(`Select CSP styles missing: ${JSON.stringify(select)}`);
      await target.keyboard.press('Escape');
      for (const name of ['Dialog', 'AlertDialog', 'Sheet', 'DropdownMenu', 'ContextMenu']) {
        await target.getByRole('button', { name: `打开 ${name}`, exact: true }).click({ button: name === 'ContextMenu' ? 'right' : 'left' });
        await target.waitForFunction(() => !!document.body.getAttribute('data-scroll-locked'));
        const valid = await target.evaluate(() => {
          const content = document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]');
          const box = content?.getBoundingClientRect();
          const lockStyles = [...document.querySelectorAll('style')].filter(s => s.textContent.includes('body[data-scroll-locked]'));
          return getComputedStyle(document.body).overflow === 'hidden' && box?.width > 0 && box?.height > 0 && box.left >= -1 && box.right <= innerWidth + 1 && lockStyles.length > 0 && lockStyles.every(s => s.sheet && s.nonce === 'csp-fixture-nonce');
        });
        if (!valid) throw new Error(`${name} CSP layout/lock failed at ${width}`);
        if (name === 'AlertDialog') await target.getByRole('button', { name: '取消', exact: true }).click();
        else await target.keyboard.press('Escape');
        await target.waitForFunction(() => !document.body.hasAttribute('data-scroll-locked') && document.body.style.pointerEvents !== 'none');
        if (name !== 'ContextMenu') await target.waitForFunction(label => document.activeElement?.textContent === label, `打开 ${name}`);
      }
      await target.getByRole('button', { name: 'SVG 设置图标' }).hover();
      await target.getByRole('tooltip').waitFor();
      const tooltip = await target.locator('[data-slot="tooltip-content"]').evaluate(e => {
        const r = e.getBoundingClientRect();
        return r.width > 1 && r.height > 1 && r.left >= -1 && r.right <= innerWidth + 1;
      });
      if (!tooltip) throw new Error('Tooltip escaped viewport');
      const svg = await target.getByRole('button', { name: 'SVG 设置图标' }).locator('svg').evaluate(e => ({ width: e.getBoundingClientRect().width, height: e.getBoundingClientRect().height, stroke: getComputedStyle(e).stroke }));
      if (svg.width !== 16 || svg.height !== 16 || svg.stroke === 'none') throw new Error('Inline SVG styles failed');
      const progress = await target.getByRole('progressbar', { name: '圆形进度' }).evaluate(e => ({ text: e.textContent, background: getComputedStyle(e.querySelector('[data-slot="progress-indicator"]')).backgroundImage }));
      if (progress.text !== '42%' || !progress.background.includes('conic-gradient')) throw new Error('Circular progress styles failed');
      await target.getByRole('checkbox', { name: '测试复选框' }).check();
      await target.getByRole('button', { name: '展开内容' }).click();
      await target.getByText('手风琴内容', { exact: true }).waitFor({ state: 'visible' });
      await target.getByRole('button', { name: '展开内容' }).click();
      results.push({ width, height, overlays: 5, select: true, tooltip: true, svg: true });
    }
    await target.getByRole('tab', { name: 'S3 / space-two', exact: true }).click();
    await target.locator('#backup-key:not(:disabled)').waitFor();
    if ((await target.evaluate(() => window.fixtureCalls)).length) throw new Error('Tab selection switched default space');
    await target.getByRole('button', { name: '设为默认', exact: true }).click();
    await target.waitForFunction(() => window.fixtureCalls.length === 1);
    if ((await target.evaluate(() => window.fixtureCalls))[0] !== 'two') throw new Error('Wrong switch target');
    await target.goto(origin + '/tests/fixtures/csp-components.html?credential-error');
    await target.getByText('系统凭据存储不可用，配置未能读取或保存', { exact: true }).waitFor();
    if (errors.length) throw new Error(errors.join('\n'));
    return { result: 'PASS', viewports: results, mountCycles: 9, unintendedSpaceSwitches: 0, intentionalSwitch: true, credentialFailureVisible: true, pageErrors: 0 };
  } finally {
    await target.unrouteAll({ behavior: 'wait' });
    await target.close();
  }
}
