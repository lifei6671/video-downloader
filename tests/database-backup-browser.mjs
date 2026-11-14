// Run with playwright-cli run-code against the production preview.
async (page) => {
  const verified = [];
  for (const provider of ['s3', 'telegram']) {
  const p = await page.context().newPage();
  const errors = [];
  p.on('pageerror', error => errors.push(error.message));
  await p.emulateMedia({ reducedMotion: 'reduce' });
  await p.addInitScript(provider => {
    const space = {
      provider_type: provider, profile: provider === 's3' ? 'r2' : 'telegram', region: 'auto', path_style: true,
      endpoint: 'https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com',
      bucket: 'test-bucket', prefix: 'test/', space_id: 'a', library_id: 'library',
      display_name: '主空间', enabled: true, credentials_configured: true, connection_pending: false,
    };
    if (provider === 'telegram') Object.assign(space, {
      endpoint: '', bucket: '', prefix: '', credentials_configured: false,
      telegram: {account_id:'123',channel_id:'777'}, telegram_channel_title:'测试频道',
    });
    const status = { enabled: false, generated_password: false, queued: false, interval_hours: 24, next_attempt_at: null, last_success_at: null, last_message_link: null, last_error: null, last_speed_bps: null, last_canceled: false, progress: null };
    window.finishDatabaseBackup = canceled => {
      status.queued = false;
      status.progress = null;
      status.next_attempt_at = 1794614400 + status.interval_hours * 3600;
      status.last_canceled = canceled;
      if (!canceled) {
        status.last_success_at = 1794614400;
        status.last_speed_bps = 1024;
        if (provider === 'telegram') status.last_message_link = 'https://t.me/c/777/9';
      }
    };
    window.databaseCalls = [];
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = { convertFileSrc: x => x, transformCallback: () => 1, unregisterCallback: () => {}, invoke: async (command, args) => {
      if (command.includes('database_backup')) window.databaseCalls.push({ command, args });
      switch (command) {
        case 'get_settings': return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [] };
        case 'get_backup_settings': return space;
        case 'list_backup_spaces': return [space];
        case 'get_backup_sync_enabled': return true;
        case 'get_backup_credentials': return { access_key_id: 'fixture-key', secret_access_key: 'fixture-secret' };
        case 'telegram_storage_status': return { configured:true, phase:'ready', operation_id:'fixture', account:{id:'123',name:'测试账号'} };
        case 'telegram_storage_channels': return [{ id:'777', title:'测试频道', can_post:true, can_delete:true }];
        case 'get_database_backup_status': return { ...status };
        case 'set_database_backup_enabled':
          status.enabled = args.enabled;
          if (args.enabled) {
            status.generated_password = args.generated;
            status.next_attempt_at = 1794614400 + status.interval_hours * 3600;
          }
          return { ...status };
        case 'set_database_backup_interval': status.interval_hours = args.hours; return { ...status };
        case 'start_database_backup':
          status.queued = true;
          status.progress = { phase: 'uploading', uploaded_bytes: 1024, total_bytes: 4096, speed_bps: 1024 };
          return { ...status };
        case 'cancel_database_backup':
          status.progress = { ...status.progress, phase: 'canceling' };
          return { ...status };
        case 'copy_database_backup_password':
          if (window.holdPasswordCopy) await new Promise(resolve => { window.finishPasswordCopy = resolve; });
          return;
        case 'copy_database_backup_link':
          if (window.holdLinkCopy) await new Promise(resolve => { window.finishLinkCopy = resolve; });
          return;
        case 'list_database_backups':
          if (args.recoveryKey === 'wrong') throw 'DATABASE_BACKUP_INVALID_PASSWORD';
          return args.recoveryKey === 'no matching backup' ? [] : [{ id: 'fixture-snapshot', created_at: 1794614400, bytes: 4096 }];
        case 'inspect_telegram_database_backup':
          if (window.holdVerification) await new Promise(resolve => { window.finishVerification = resolve; });
          if (args.recoveryKey === 'wrong') throw 'DATABASE_BACKUP_INVALID_PASSWORD';
          if (args.messageLink !== 'https://t.me/c/777/9') throw 'DATABASE_BACKUP_WRONG_CHANNEL';
          return { id: '9', created_at: 1794614400, bytes: 4096 };
        case 'restore_database_backup': return;
        case 'list_tasks': case 'list_video_transcripts': case 'list_backup_assets': case 'list_backup_operations': return [];
        default: return null;
      }
    }};
  }, provider);
  const button = name => p.getByRole('button', { name, exact: true });
  const layout = locator => locator.evaluate(el => [el.offsetLeft, el.offsetTop, el.offsetWidth, el.offsetHeight].join(','));
  const checkImmediateFeedback = locator => locator.evaluate(el => {
    const style = getComputedStyle(el);
    if (style.transitionDelay !== '0s' || style.transitionDuration !== '0.1s') throw Error('Backup feedback must start immediately and finish in 100ms');
  });
  async function checkButtonTransition(locator) {
    await locator.hover();
    const readState = async () => locator.evaluate(async el => {
      const style = getComputedStyle(el);
      if (!style.transitionProperty.includes('background-color') || style.transitionProperty.includes('scale')) throw Error('Unexpected backup button transition');
      await Promise.all(el.getAnimations().map(animation => animation.finished.catch(() => {})));
      return [style.scale, style.translate, style.transform, style.backgroundColor];
    });
    const hovered = await readState();
    await p.mouse.down();
    try {
      const pressed = await readState();
      if (String(pressed) !== String(hovered)) throw Error(`Backup button changed appearance while pressed: ${hovered} → ${pressed}`);
    } finally { await p.mouse.move(0, 0); await p.mouse.up(); }
  }
  try {
    await p.goto(page.url().split('/').slice(0, 3).join('/') + '/');
    await button('系统设置').click();
    await button('远端备份').click();
    const toggle = p.getByRole('checkbox', { name: '自动备份数据库' });
    await p.locator('#backup-database-enabled:not(:disabled)').waitFor();
    if (await toggle.isChecked()) throw Error('Database backup default must be off');
    if (!await button('立即备份').isDisabled()) throw Error('Manual backup was not gated by opt-in');
    const toggleBounds = await toggle.boundingBox();
    const labelBounds = await p.getByText('自动备份数据库', { exact: true }).boundingBox();
    if (!toggleBounds || !labelBounds || Math.abs(toggleBounds.x - labelBounds.x) > 60) throw Error('Checkbox is not adjacent to its label');
    await p.getByText('本机暂无成功备份记录').waitFor();
    await toggle.focus();
    await p.keyboard.press('Space');
    const setup = p.getByRole('dialog', { name: '设置数据库恢复密码' });
    await setup.waitFor();
    await setup.getByText('密码丢失后，无法从备份恢复数据。', { exact: true }).waitFor();
    await p.keyboard.press('Escape');
    await setup.waitFor({ state: 'hidden' });
    if (await toggle.isChecked()) throw Error('Closing password setup opted in');
    await p.waitForFunction(() => document.activeElement?.id === 'backup-database-enabled');
    await p.keyboard.press('Space');
    const password = setup.getByLabel('恢复密码', { exact: true });
    const confirmation = setup.getByLabel('确认恢复密码', { exact: true });
    await password.fill('测试 recovery password');
    await confirmation.fill('different password');
    const enable = setup.getByRole('button', { name: '启用数据库备份', exact: true });
    await setup.getByText('两次输入的密码不一致').waitFor();
    if (!await enable.isDisabled()) throw Error('Password confirmation did not gate opt-in');
    await confirmation.fill('测试 recovery password');
    await enable.click();
    await setup.waitFor({ state: 'hidden' });
    await p.getByText(/^下次自动备份/).waitFor();
    if (await p.getByRole('status', { name: '数据库备份进度' }).count()) throw Error('Enabling automatic backup started a transfer');
    await p.emulateMedia({ reducedMotion: 'no-preference' });
    await checkButtonTransition(button('立即备份'));
    await p.emulateMedia({ reducedMotion: 'reduce' });
    const frequency = p.getByRole('combobox', { name: '备份频率' });
    await frequency.click();
    for (const hours of [1, 3, 6, 24]) await p.getByRole('option', { name: `每 ${hours} 小时`, exact: true }).waitFor();
    if (await p.getByRole('option').count() !== 4) throw Error('Unexpected backup interval options');
    await p.getByRole('option', { name: '每 1 小时', exact: true }).click();
    if (await button('复制备份密码').count()) throw Error('A manually entered password was copyable');
    await button('立即备份').click();
    const transfer = p.getByRole('status', { name: '数据库备份进度' });
    await transfer.getByText('正在上传数据库备份').waitFor();
    await p.getByText('上次平均上传速度 暂无记录').waitFor();
    await transfer.locator('.animate-spin').waitFor();
    await transfer.getByText('1.0 KB/s', { exact: true }).waitFor();
    if (await transfer.getByRole('progressbar').getAttribute('aria-valuenow') !== '25') throw Error('Unexpected upload progress');
    if (!await button('立即备份').isDisabled()) throw Error('Active upload still allows duplicate starts');
    await button('取消备份').click();
    await transfer.getByText('正在取消备份').waitFor();
    if (!await button('立即备份').isDisabled() || !await button('取消备份').isDisabled()) throw Error('Cancellation did not lock the actions');
    await p.evaluate(() => window.finishDatabaseBackup(true));
    await p.getByText('上次备份已取消').waitFor();
    await button('立即备份').click();
    await transfer.getByText('正在上传数据库备份').waitFor();
    await p.evaluate(() => window.finishDatabaseBackup(false));
    await p.getByRole('status').filter({ hasText: '上次成功备份' }).waitFor();
    await p.getByText(/^下次自动备份/).waitFor();
    await p.getByText('上次平均上传速度 1.0 KB/s').waitFor();
    await button('立即备份').click();
    await transfer.getByText('正在上传数据库备份').waitFor();
    await p.getByText('上次平均上传速度 1.0 KB/s').waitFor();
    await p.evaluate(() => window.finishDatabaseBackup(false));
    await transfer.waitFor({ state: 'hidden' });
    await toggle.click();
    if (!await button('立即备份').isDisabled()) throw Error('Manual backup stayed enabled after opt-out');
    await p.getByText(/^上次成功备份/).waitFor();
    if (provider === 'telegram') {
      const link = button('复制 Telegram 数据库备份链接');
      if (!(await link.getAttribute('aria-describedby'))) throw Error('Telegram link has no explanation');
      await p.getByText(/直接读取 Telegram 私有频道中的这份备份/).waitFor();
      await p.emulateMedia({ reducedMotion: 'no-preference' });
      await checkButtonTransition(link);
      const linkLayout = await layout(link);
      const sectionLayout = await layout(p.getByRole('region', { name: '数据库备份' }));
      await p.evaluate(() => { window.holdLinkCopy = true; });
      await link.click();
      await link.and(p.locator('[data-copy-phase="copying"]')).waitFor();
      await checkImmediateFeedback(link.locator('.backup-action-pending'));
      if (await layout(link) !== linkLayout) throw Error('Copying moved the Telegram link button');
      await p.evaluate(() => { window.holdLinkCopy = false; window.finishLinkCopy(); });
      await p.getByText('Telegram 数据库备份链接已复制').waitFor();
      if (await layout(link) !== linkLayout || await layout(p.getByRole('region', { name: '数据库备份' })) !== sectionLayout) throw Error('Link confirmation moved the backup settings');
      await p.waitForTimeout(700);
      if (await link.getAttribute('data-copy-phase') !== 'copied') throw Error('Link copy confirmation disappeared too soon');
      await link.and(p.locator('[data-copy-phase="idle"]')).waitFor({ timeout: 800 });
      await p.emulateMedia({ reducedMotion: 'reduce' });
    } else if (await button('复制 Telegram 数据库备份链接').count()) throw Error('R2 offered a Telegram message link');

    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await p.setViewportSize({ width, height });
      await button('恢复数据库').focus();
      await p.keyboard.press('Enter');
      const dialog = p.getByRole('dialog', { name: '恢复数据库' });
      await dialog.waitFor();
      const bounds = await dialog.boundingBox();
      if (!bounds || bounds.x < 0 || bounds.y < 0 || bounds.x + bounds.width > width || bounds.y + bounds.height > height) throw Error('Restore dialog overflows viewport');
      await p.keyboard.press('Escape');
      await dialog.waitFor({ state: 'hidden' });
      await p.waitForFunction(() => document.activeElement?.textContent === '恢复数据库');
    }
    const telegram = provider === 'telegram';
    if (telegram) {
      await p.getByRole('tab', { name: '新建空间', exact: true }).click();
      await p.getByRole('combobox', { name: '备份提供商' }).click();
      await p.getByRole('option', { name: 'Telegram', exact: true }).click();
      await p.getByRole('combobox', { name: '私有频道' }).click();
      await p.getByRole('option', { name: '测试频道', exact: true }).click();
    }
    await button('恢复数据库').click();
    const field = p.getByLabel('恢复密码', { exact: true });
    const find = button(telegram ? '验证快照' : '查找备份');
    const message = p.getByLabel('Telegram 数据库备份链接', { exact: true });
    if (telegram) await message.fill('https://t.me/c/777/9');
    await field.fill('wrong');
    await find.click();
    await p.getByText(/^恢复密码须为/).waitFor();
    if (telegram) {
      await field.fill('fixture-recovery-key');
      await message.fill('https://t.me/c/123/9');
      await p.emulateMedia({ reducedMotion: 'no-preference' });
      await checkButtonTransition(find);
      const details = p.getByRole('dialog', { name: '恢复数据库' });
      const dialogLayout = await layout(details);
      const verifyLayout = await layout(find);
      await p.evaluate(() => { window.holdVerification = true; });
      await find.click();
      await p.locator('.backup-restore-submit[aria-busy="true"]').waitFor();
      await checkImmediateFeedback(find.locator('.backup-action-pending'));
      await p.waitForFunction(() => getComputedStyle(document.querySelector('.backup-restore-submit .backup-action-pending')).opacity === '1');
      await details.getByRole('button', { name: '关闭' }).waitFor();
      if (await layout(find) !== verifyLayout || await layout(details) !== dialogLayout) throw Error('Verifying moved the dialog or the submit button');
      await p.evaluate(() => { window.holdVerification = false; window.finishVerification(); });
      await p.getByText('快照消息不在所选频道内').waitFor();
      if (await layout(find) !== verifyLayout || await layout(details) !== dialogLayout) throw Error('Verification error moved the dialog or the submit button');
      await p.emulateMedia({ reducedMotion: 'reduce' });
      await message.fill('https://t.me/c/777/9');
    } else {
      await field.fill('no matching backup');
      await find.click();
      const empty = p.getByRole('alert').filter({ hasText: '未找到匹配的数据库备份' });
      await empty.waitFor();
      if (!await empty.evaluate(el => el.classList.contains('text-destructive'))) throw Error('Missing backup is not shown in the error color');
    }
    await field.fill('fixture-recovery-key');
    await find.click();
    await (telegram ? p.getByRole('status').filter({hasText: '备份时间'}) : p.getByRole('combobox', { name: '备份时间' })).waitFor();
    if (telegram) {
      await message.fill('https://t.me/c/777/10');
      await find.waitFor();
      await message.fill('https://t.me/c/777/9');
      await find.click();
    }
    await button('恢复并退出').click();
    await button('正在退出…').waitFor();
    const calls = await p.evaluate(() => window.databaseCalls);
    const toggles = calls.filter(call => call.command === 'set_database_backup_enabled').map(call => call.args.enabled);
    if (String(toggles) !== 'true,false') throw Error('Database opt-in did not persist correctly');
    if (calls.find(call => call.command === 'set_database_backup_enabled')?.args.password !== '测试 recovery password') throw Error('The user password was not submitted');
    if (calls.find(call => call.command === 'set_database_backup_interval')?.args.hours !== 1) throw Error('Backup interval was not saved');
    const restored = calls.find(call => call.command === 'restore_database_backup');
    if (telegram) {
      if (restored?.args.snapshotId !== 'https://t.me/c/777/9' || restored.args.telegram?.channel_id !== '777' || restored.args.spaceId) throw Error('Wrong Telegram restore target');
      if (calls.some(call => call.command === 'list_database_backups')) throw Error('Telegram recovery enumerated snapshots');
    } else if (restored?.args.snapshotId !== 'fixture-snapshot' || restored.args.spaceId !== 'a') throw Error('Wrong R2 restore target');
    await p.reload();
    await button('系统设置').click();
    await button('远端备份').click();
    await toggle.click();
    await setup.waitFor();
    const automatic = setup.locator('[data-slot="dialog-footer"]').getByRole('button', { name: '自动生成' });
    if (await password.inputValue() || await confirmation.inputValue()) throw Error('Password inputs should start empty');
    await automatic.click();
    await setup.waitFor({ state: 'hidden' });
    const copyPassword = button('复制备份密码');
    const restoreButton = button('恢复数据库');
    const copyLayout = await layout(copyPassword);
    const restoreLayout = await layout(restoreButton);
    await p.evaluate(() => { window.holdPasswordCopy = true; });
    await copyPassword.click();
    await p.locator('[data-copy-phase="copying"]').waitFor();
    if (await restoreButton.isDisabled() || !await toggle.isEnabled()) throw Error('Copying disabled unrelated controls');
    if (await layout(copyPassword) !== copyLayout || await layout(restoreButton) !== restoreLayout) throw Error('Copying shifted the action row');
    await copyPassword.click();
    await p.evaluate(() => { window.holdPasswordCopy = false; window.finishPasswordCopy(); });
    await p.getByText('备份密码已复制').waitFor();
    await p.locator('[data-copy-phase="copied"]').waitFor();
    if (await layout(copyPassword) !== copyLayout) throw Error('Copy confirmation resized the button');
    if (await p.evaluate(() => window.databaseCalls.filter(call => call.command === 'copy_database_backup_password').length) !== 1) throw Error('Repeated click triggered a second copy before completion');
    await p.waitForTimeout(700);
    if (await copyPassword.getAttribute('data-copy-phase') !== 'copied') throw Error('Password copy confirmation disappeared too soon');
    await p.locator('[data-copy-phase="idle"]').waitFor({ timeout: 800 });
    await p.emulateMedia({ reducedMotion: 'no-preference' });
    await checkButtonTransition(copyPassword);
    await copyPassword.focus();
    await p.keyboard.press('Enter');
    await p.locator('[data-copy-phase="copied"]').waitFor();
    await checkImmediateFeedback(copyPassword.locator('.backup-action-success'));
    const generatedArgs = await p.evaluate(() => window.databaseCalls.find(call => call.command === 'set_database_backup_enabled').args);
    if (!generatedArgs.generated || !/^[A-Za-z0-9_-]{24}$/.test(generatedArgs.password)) throw Error('Generated password was not saved as copyable');
    await toggle.click();
    await toggle.click();
    await setup.waitFor();
    if (errors.length) throw Error(errors.join('\n'));
    verified.push(provider);
  } finally {
    await p.close();
  }
  }
  return { passed: true, verified, viewports: 3 };
}
