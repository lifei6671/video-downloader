// Run against vite preview after npm run build:
// playwright-cli --session settings-layout run-code --filename tests/settings-layout-browser.mjs
async (page) => {
  const target = await page.context().newPage();
  const errors = [];
  target.on('pageerror', error => errors.push(error.message));
  // Native Tauri CSP rejects Sonner's runtime <style> injection. Preview must
  // enforce the same boundary rather than silently accepting the injected CSS.
  await target.route('**/*', async route => {
    if (!route.request().isNavigationRequest()) return route.continue();
    const response = await route.fetch();
    await route.fulfill({ response, headers: { ...response.headers(), 'content-security-policy': "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'" } });
  });
  await target.addInitScript(() => {
    const config = { space_id: 'layout', library_id: 'layout-library', provider_type: 's3', profile: 'r2', region: 'auto', path_style: true, endpoint: 'https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com', bucket: 'fixture', prefix: 'fixture/', enabled: true, credentials_configured: true, protocol: 2, last_error: null };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      convertFileSrc: path => `http://asset.localhost/${encodeURIComponent(path)}`,
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async command => {
        if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
        if(command==='get_backup_sync_enabled')return true;
      if (command === 'get_backup_settings') return config;
        if (command === 'list_backup_spaces') return [config];
        // Loading an existing space can fail and show a notification on entry.
        if (command === 'get_backup_credentials') throw 'CREDENTIAL_STORE_UNAVAILABLE';
        if (['list_tasks', 'list_video_transcripts', 'list_backup_assets', 'list_backup_operations'].includes(command)) return [];
        if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
        return null;
      },
    };
  });
  const results = [];
  try {
    await target.goto(page.url().split('/').slice(0, 3).join('/') + '/');
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await target.setViewportSize({ width, height });
      // Sample layout on animation frames across route commits.
      await target.evaluate(() => {
        window.layoutSamples = [];
        window.sampleLayout = () => {
          const header = document.querySelector('.app-page-header');
          if (header) window.layoutSamples.push({ top: header.getBoundingClientRect().top, height: document.documentElement.scrollHeight, viewport: innerHeight });
          window.layoutFrame = requestAnimationFrame(window.sampleLayout);
        };
        window.layoutFrame = requestAnimationFrame(window.sampleLayout);
      });
      for (let repeat = 0; repeat < 3; repeat++) {
        for (const name of ['下载作品', '下载任务', '关于软件', '系统设置']) {
          await target.getByRole('button', { name, exact: true }).click();
          await target.getByRole('heading', { name, exact: true, level: 1 }).waitFor();
        }
        await target.getByRole('button', { name: '远端备份', exact: true }).click();
        await target.locator('#backup-key:not(:disabled)').waitFor();
        await target.locator('[data-sonner-toast]').first().waitFor({ state: 'visible' });
        const notification = await target.evaluate(() => {
          const toaster = document.querySelector('[data-sonner-toaster]');
          const toast = document.querySelector('[data-sonner-toast]');
          const runtimeStyle = [...document.querySelectorAll('style')].find(e => e.textContent.includes('[data-sonner-toaster]'));
          return { toaster: getComputedStyle(toaster).position, toast: getComputedStyle(toast).position, runtimeStyleBlocked: !!runtimeStyle && !runtimeStyle.sheet };
        });
        if (!notification.runtimeStyleBlocked || notification.toaster !== 'fixed' || notification.toast !== 'absolute') throw new Error(`Notification CSS unavailable under CSP: ${JSON.stringify(notification)}`);
        const scroll = target.locator('.backup-settings-scroll');
        await scroll.evaluate(element => { element.scrollTop = element.scrollHeight; });
        await target.getByRole('button', { name: '保存配置', exact: true }).waitFor({ state: 'visible' });
        const bounded = await scroll.evaluate(element => {
          const header = document.querySelector('.app-page-header').getBoundingClientRect();
          const box = element.getBoundingClientRect();
          return header.top === 0 && box.bottom <= innerHeight + 1 && (element.scrollHeight <= element.clientHeight + 1 || element.scrollTop > 0);
        });
        if (!bounded) throw new Error(`Settings scroll escaped workspace at ${width}x${height}`);
        await target.getByRole('button', { name: '关于软件', exact: true }).click();
        // Keyboard navigation must retain the same layout contract.
        await target.getByRole('button', { name: '系统设置', exact: true }).focus();
        await target.keyboard.press('Enter');
        await target.getByRole('heading', { name: '系统设置', exact: true, level: 1 }).waitFor();
      }
      const samples = await target.evaluate(() => { cancelAnimationFrame(window.layoutFrame); return window.layoutSamples; });
      if (!samples.length || samples.some(s => Math.abs(s.top) > 0.5 || s.height > s.viewport + 1)) throw new Error(`Header gap or outer scroll at ${width}x${height}: ${JSON.stringify(samples)}`);
      results.push({ width, height, frames: samples.length, repeats: 3 });
    }
    if (errors.length) throw new Error(errors.join('\n'));
    return { result: 'PASS', viewports: results, nativeStyleCsp: true, notificationOnSettingsEntry: true, internalScroll: true, keyboardNavigation: true, pageErrors: errors.length };
  } finally {
    await target.evaluate(() => cancelAnimationFrame(window.layoutFrame)).catch(() => {});
    await target.unrouteAll({ behavior: 'wait' });
    await target.close();
  }
}
