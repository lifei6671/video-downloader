// Isolated production IPC fixtures. Never reads app identity files or a real Telegram session.
async (page) => {
  const p = await page.context().newPage();
  const errors = []; p.on('pageerror', error => errors.push(error.message));
  await p.addInitScript(() => {
    const signedOut = () => ({ configured: window.configured, operation_id: null, phase: 'signed_out', account: null, last_error: null, qr: null, qr_expires_at: null });
    window.configured = false; window.telegram = signedOut(); window.calls = [];
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = { convertFileSrc: value => value, transformCallback: () => 1, unregisterCallback: () => {}, invoke: async (command, args) => {
      // Record command names only; fixture credentials are not persisted in the test report.
      window.calls.push(command);
      const ready = () => ({ configured: true, operation_id: 'login-1', phase: 'ready', account: { id: '123', name: '专用测试账号' }, last_error: null });
      if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
      if (command === 'get_backup_settings') return null;
      if (command === 'get_backup_sync_enabled') return false;
      if (['list_tasks', 'list_video_transcripts', 'list_backup_spaces', 'list_backup_assets', 'list_backup_operations'].includes(command)) return [];
      if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
      if (command === 'telegram_storage_status') { if (window.holdStatus) await new Promise(resolve => { window.releaseStatus = resolve; }); return structuredClone({ ...window.telegram, configured: window.configured }); }
      if (command === 'telegram_storage_resume') { window.telegram = {...signedOut(),operation_id:'old-login'}; return structuredClone(window.telegram); }
      if (command === 'telegram_storage_login_qr') {
        window.telegram = { ...signedOut(), configured: true, phase: 'qr', operation_id: 'login-1', qr: [[true,false],[false,true]], qr_expires_at: Math.floor(Date.now()/1000)+30 };
        return await new Promise(resolve => {
          window.scanQr = () => { window.telegram.phase = 'password'; window.telegram.qr = null; resolve(structuredClone(window.telegram)); };
          window.finishLogin = () => resolve(ready());
        });
      }
      if (command === 'telegram_storage_password') {
        if (args.password !== 'fixture-password') throw 'TELEGRAM_INVALID_PASSWORD';
        window.telegram = ready(); return structuredClone(window.telegram);
      }
      if (command === 'telegram_storage_channels') {
        if (window.failChannelsTimeout) { window.telegram = signedOut(); throw 'TELEGRAM_TIMEOUT'; }
        return [{ id: '777', title: '隔离验证专用私有频道', can_post: true, can_delete: false }, { id: '888', title: '只读频道', can_post: false, can_delete: false }];
      }
      if (command === 'telegram_storage_logout') {
        if (window.failLogout) throw 'TELEGRAM_CONNECTION_ERROR';
        window.telegram = signedOut(); return structuredClone(window.telegram);
      }
      if (command === 'telegram_storage_cancel') { window.telegram = signedOut(); window.finishLogin?.(); return structuredClone(window.telegram); }
      return null;
    } };
  });
  const button = name => p.getByRole('button', { name, exact: true });
  const selectProvider = async (name) => {
    await p.getByRole('combobox', { name: '备份提供商' }).click();
    await p.getByRole('option', { name, exact: true }).click();
  };
  const assert = (value, message) => { if (!value) throw new Error(message); };
  try {
    await p.goto(page.url().split('/').slice(0, 3).join('/') + '/');
    await button('系统设置').click(); await button('远端备份').click();
    await p.locator('#backup-key:not(:disabled)').waitFor();
    await selectProvider('Telegram');
    await p.getByText('此构建尚未配置 Telegram 应用身份。', { exact: true }).waitFor();
    assert(await button('扫码登录').count() === 0, 'Unconfigured build exposes login');
    assert(await p.locator('#backup-key').count() === 0, 'Telegram exposes S3 credentials');
    assert(await button('新建配置').count() === 0, 'Unverified provider exposes a write action');
    await p.evaluate(() => { window.configured = true; });
    await selectProvider('Cloudflare R2'); await selectProvider('Telegram');
    await button('恢复登录').click();
    await p.getByText('没有可恢复的登录，请扫码登录。', { exact: true }).waitFor();
    assert(await p.getByLabel('手机号', { exact: true }).count() === 0, 'QR login still asks for a phone number');
    await p.evaluate(() => { window.holdStatus = true; });
    await button('扫码登录').click();
    assert(await button('取消登录').isDisabled(), 'Fresh QR flow can cancel the previous login id');
    await p.keyboard.press('Escape');
    await p.getByRole('dialog', { name: '登录 Telegram 存储' }).waitFor();
    await p.evaluate(() => { window.holdStatus = false; window.releaseStatus?.(); });
    await p.getByRole('img', { name: 'Telegram 登录二维码' }).waitFor();
    const beforeQr = await p.getByRole('img', { name: 'Telegram 登录二维码' }).locator('path').getAttribute('d');
    await p.evaluate(() => { window.telegram.qr = [[false,true],[true,false]]; window.telegram.qr_expires_at += 30; });
    await p.waitForFunction(before => document.querySelector('[aria-label="Telegram 登录二维码"] path')?.getAttribute('d') !== before, beforeQr);
    await p.evaluate(() => window.scanQr());
    await p.getByLabel('两步验证密码', { exact: true }).fill('wrong'); await button('验证并登录').click();
    await p.getByRole('alert').filter({ hasText: '两步验证密码错误' }).waitFor();
    assert(await p.getByLabel('两步验证密码', { exact: true }).inputValue() === '', 'Password not cleared after submission');
    await p.getByLabel('两步验证密码', { exact: true }).fill('fixture-password'); await button('验证并登录').click();
    await p.getByRole('dialog', { name: '登录 Telegram 存储' }).waitFor({ state: 'hidden' });
    await p.getByText('专用测试账号', { exact: true }).waitFor();
    await p.getByRole('combobox', { name: '私有频道' }).waitFor();
    await p.waitForFunction(() => window.calls.includes('telegram_storage_channels'));
    assert(await button('保存配置').isDisabled(), 'Saving was enabled without a private channel');
    await p.getByRole('combobox', { name: '私有频道' }).click();
    await p.getByRole('option', { name: '隔离验证专用私有频道', exact: true }).click();
    await button('创建备份库').waitFor();
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await p.setViewportSize({ width, height });
      await button('退出存储账号').scrollIntoViewIfNeeded();
      assert(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Horizontal overflow at ' + width);
      await p.getByRole('combobox', { name: '私有频道' }).focus(); await p.keyboard.press('Enter');
      await p.getByRole('option', { name: '只读频道（只读）', exact: true }).waitFor();
      await p.keyboard.press('Escape');
    }
    await p.evaluate(() => { window.failChannelsTimeout = true; }); await selectProvider('Cloudflare R2'); await selectProvider('Telegram');
    await p.getByRole('alert').filter({ hasText: '请求超时' }).waitFor();
    await button('扫码登录').waitFor();
    assert(await p.getByText('专用测试账号', { exact: true }).count() === 0, 'Timeout left a stale account visible');
    assert(await p.getByRole('combobox', { name: '私有频道' }).isDisabled(), 'Timeout left stale channels selectable');
    await p.evaluate(() => { window.failChannelsTimeout = false; window.telegram = { configured: true, operation_id: 'login-1', phase: 'connecting', account: null, last_error: null }; });
    await selectProvider('Cloudflare R2'); await selectProvider('Telegram');
    await p.getByText('正在连接 Telegram…', { exact: true }).waitFor();
    await p.evaluate(() => { window.telegram.phase = 'qr'; window.telegram.qr = [[true]]; });
    await p.getByRole('img', { name: 'Telegram 登录二维码' }).waitFor();
    await p.keyboard.press('Escape');
    await button('扫码登录').waitFor();
    await p.evaluate(() => { window.telegram = { configured: true, operation_id: 'login-1', phase: 'ready', account: { id: '123', name: '专用测试账号' }, last_error: null }; window.failLogout = true; });
    await selectProvider('Cloudflare R2'); await selectProvider('Telegram');
    await button('退出存储账号').click();
    await p.getByRole('alert').filter({ hasText: '连接失败' }).waitFor();
    await p.getByText('专用测试账号', { exact: true }).waitFor();
    await p.evaluate(() => { window.failLogout = false; }); await button('退出存储账号').click();
    await button('扫码登录').waitFor();
    await button('扫码登录').click();
    await p.getByRole('img', { name: 'Telegram 登录二维码' }).waitFor();
    await p.keyboard.press('Escape');
    await button('扫码登录').waitFor();
    assert(await p.getByText('专用测试账号', { exact: true }).count() === 0, 'Late login response replaced canceled state');
    await selectProvider('Cloudflare R2');
    await p.locator('#backup-key:not(:disabled)').waitFor();
    assert(await p.locator('#backup-secret').count() === 1, 'S3/R2 form lost its credential fields');
    assert(!(await p.evaluate(() => window.calls)).some(name => ['save_backup_settings', 'select_backup_space', 'start_asset_backup', 'login_tdl'].includes(name)), 'Login flow wrote backup state or reused tdl');
    assert(errors.length === 0, errors.join('\n'));
    return { passed: true, viewports: 3, flows: ['missing-app', 'resume', 'qr-refresh', '2fa-retry', 'channels', 'channels-timeout', 'return-while-connecting', 'logout-retry', 'cancel-late-response', 's3-isolation'] };
  } finally { await p.close(); }
}
