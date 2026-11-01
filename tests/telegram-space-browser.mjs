// Synthetic IPC only: no real Telegram account, files or channel writes.
async page => {
  const p = await page.context().newPage();
  const failures = []; p.on('pageerror', error => failures.push(error.message));
  await p.addInitScript(() => {
    const s3 = { space_id: 's3', library_id: 's3-lib', display_name: '对象存储', provider_type: 's3', profile: 'r2', endpoint: 'https://fixture.r2.cloudflarestorage.com', bucket: 'bucket', prefix: 'yingqu/', region: 'auto', path_style: true, enabled: true, credentials_configured: false, connection_pending: false, last_error: null };
    window.spaces = [s3]; window.current = 's3'; window.enabled = true; window.noLibrary = true; window.calls = [];
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = { convertFileSrc: value => value, transformCallback: () => 1, unregisterCallback: () => {}, invoke: async (command, args) => {
      const config = args?.config;
      window.calls.push({ command, provider: config?.provider_type, create: config?.create_library, activate: config?.activate, space: args?.spaceId });
      if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
      if (command === 'get_backup_settings') return structuredClone(window.spaces.find(s => s.space_id === window.current) ?? null);
      if (command === 'get_backup_sync_enabled') return window.enabled;
      if (command === 'list_backup_spaces') return structuredClone(window.spaces);
      if (['list_tasks', 'list_video_transcripts', 'list_backup_assets', 'list_backup_operations'].includes(command)) return [];
      if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
      if (command === 'telegram_storage_status') return { configured: !window.unconfigured, phase: window.signedOut ? 'signed_out' : window.statusPhase ?? 'ready', operation_id: 'login', account: window.signedOut ? null : { id: '123', name: '测试账号' }, last_error: null };
      if (command === 'telegram_storage_channels') { if(window.holdChannels) await new Promise(resolve=>{window.releaseChannels=resolve;}); return [{ id: '777', title: '专用私有频道', can_post: true, can_delete: false }, { id: '888', title: '只读频道', can_post: false, can_delete: false }]; }
      if (command === 'save_backup_settings') {
        if (config.provider_type !== 'telegram' || config.endpoint || config.bucket || config.prefix || config.credentials) throw new Error('Telegram received S3 configuration');
        if (window.holdSave) await new Promise(resolve => { window.releaseSave = resolve; });
        if (!config.create_library && window.noLibrary) throw 'BACKUP_LIBRARY_NOT_FOUND';
        window.noLibrary = false;
        const value = { ...config, credentials: undefined, create_library: undefined, space_id: 'tg', library_id: 'telegram-lib', telegram_channel_title: config.telegram.channel_id === '777' ? '专用私有频道' : '只读频道', enabled: window.current === 'tg' && window.enabled, credentials_configured: false, connection_pending: false, last_error: window.spaceError ?? null };
        window.spaces = [...window.spaces.filter(s => s.space_id !== 'tg'), value]; return structuredClone(value);
      }
      if (command === 'select_backup_space') { window.current = args.spaceId; window.spaces.forEach(s => { s.enabled = s.space_id === window.current; }); return structuredClone(window.spaces.find(s => s.space_id === window.current)); }
      if (command === 'get_backup_space_usage') return { assets: 0, operations: 0 };
      if (command === 'delete_backup_space') { window.spaces = window.spaces.filter(s => s.space_id !== args.spaceId); return null; }
      if (command === 'get_backup_credentials') throw new Error('Unexpected credential read');
      return null;
    } };
  });
  const button = name => p.getByRole('button', { name, exact: true });
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const provider = async name => { await p.getByRole('combobox', { name: '备份提供商' }).click(); await p.getByRole('option', { name, exact: true }).click(); };
  try {
    await p.goto(page.url().split('/').slice(0, 3).join('/') + '/');
    await button('系统设置').click(); await button('远端备份').click(); await p.locator('#backup-key').waitFor();
    await p.getByRole('tab', { name: '新建空间', exact: true }).click(); await p.locator('#backup-space-name').fill('知识库测试'); const nameTop = (await p.locator('#backup-space-name').boundingBox()).y; await provider('Telegram');
    assert(await p.locator('#backup-space-name').inputValue() === '知识库测试', 'Changing provider discarded the entered space name');
    assert(Math.abs((await p.locator('#backup-space-name').boundingBox()).y - nameTop) < 1, 'Provider switch moved the space name');
    assert(await button('保存配置').isDisabled(), 'Missing channel can be saved'); await p.getByRole('combobox', { name: '私有频道' }).click(); await p.getByRole('option', { name: '专用私有频道', exact: true }).click();
    assert(await p.locator('#backup-key').count() === 0, 'Telegram exposed S3 credentials');
    await button('保存配置').click(); await p.getByRole('alert').filter({ hasText: '备份库' }).waitFor();
    assert((await p.evaluate(() => window.spaces)).length === 1, 'Read-only connection silently created a library');
    await p.evaluate(() => { window.holdSave = true; }); await button('创建备份库').click();
    await p.waitForFunction(() => typeof window.releaseSave === 'function');
    assert(await p.getByRole('tab', { name: /对象存储/ }).isDisabled(), 'Space tab remained active while saving');
    assert(await button('停用远端备份').isDisabled(), 'Global toggle remained active while saving');
    await p.evaluate(() => { window.holdSave = false; window.releaseSave(); });
    await p.getByRole('tab', { name: '知识库测试', exact: true }).waitFor(); await p.getByText('备份配置已保存', { exact: true }).waitFor();
    assert(await p.evaluate(() => window.current) === 's3', 'Saving Telegram changed the current space');
    await button('设为默认').click(); await p.waitForFunction(() => window.current === 'tg');
    await p.getByRole('tab', { name: /对象存储/ }).click(); await button('设为默认').click(); await p.waitForFunction(() => window.current === 's3');
    await p.getByRole('tab', { name: '知识库测试', exact: true }).click();
    await p.evaluate(() => { window.spaceError = 'TELEGRAM_INDEX_CHANGED'; });
    await button('重新连接').click();
    await p.getByRole('alert').filter({ hasText: '备份历史被修改或删除' }).waitFor();
    assert(await p.evaluate(() => window.spaces.find(s => s.space_id === 'tg').connection_pending) === false, 'Fixture must exercise a connected space error');
    await p.evaluate(() => { window.spaceError = null; }); await button('重新连接').click();
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await p.setViewportSize({ width, height }); await button('删除配置').scrollIntoViewIfNeeded();
      assert(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Horizontal overflow at ' + width);
      assert(await button('重新连接').isVisible(), 'Reconnect action is missing');
      const a = await button('重新连接').boundingBox(), b = await button('删除配置').boundingBox();
      assert(Math.abs(a.y - b.y) < 1, 'Saved actions wrap onto separate rows');
      assert(await button('退出存储账号').count() === 0, 'Saved configuration exposes logout');
      assert(await p.getByRole('combobox', { name: '备份提供商' }).count() === 0, 'Saved Telegram provider remains editable');
      await p.locator('.backup-settings-actions').getByText('Telegram', { exact: true }).waitFor();
      assert(await p.getByRole('combobox', { name: '私有频道' }).count() === 0, 'Saved target remains editable');
      await p.getByText('专用私有频道', { exact: true }).waitFor();
    }
    const channelReads=await p.evaluate(()=>window.calls.filter(c=>c.command==='telegram_storage_channels').length);
    await p.evaluate(()=>{window.spaces.find(s=>s.space_id==='tg').telegram_channel_title=null;window.holdChannels=true;});
    for(let i=0;i<3;i++) {
      await button('下载任务').click();await button('系统设置').click();await button('远端备份').click();
      await p.getByRole('tab',{name:'知识库测试',exact:true}).click();
      await p.getByText('频道 777',{exact:true}).waitFor();
      assert(await button('重新连接').isEnabled(),'Saved config waits for channel enumeration');
      assert(await p.getByText('正在连接 Telegram…',{exact:true}).count()===0,'Menu switch falsely shows connection');
    }
    assert(await p.evaluate(()=>window.calls.filter(c=>c.command==='telegram_storage_channels').length)===channelReads,'Saved config enumerates channels on remount');
    await p.evaluate(()=>{window.holdChannels=false;window.statusPhase='connecting';});
    await button('下载任务').click();await button('系统设置').click();await button('远端备份').click();
    await p.getByRole('tab',{name:'知识库测试',exact:true}).click();
    await p.getByText('正在连接 Telegram…',{exact:true}).waitFor();
    assert(await p.getByRole('dialog').count()===0,'Background session recovery opens login dialog');
    await p.evaluate(()=>{window.statusPhase='ready';});
    await p.getByText('正在连接 Telegram…',{exact:true}).waitFor({state:'detached'});
    await p.evaluate(() => { window.signedOut = true;window.spaces.find(s=>s.space_id==='tg').telegram_channel_title='专用私有频道'; });
    await button('下载任务').click();await button('系统设置').click();await button('远端备份').click();
    await p.getByRole('tab', { name: /对象存储/ }).click(); await p.getByRole('tab', { name: '知识库测试', exact: true }).click();
    await button('扫码登录').waitFor();
    assert(await button('退出存储账号').count() === 0, 'Saved signed-out configuration exposes logout');
    await p.getByText('专用私有频道', { exact: true }).waitFor();
    await p.evaluate(() => { window.unconfigured = true; });
    await p.getByRole('tab', { name: /对象存储/ }).click(); await p.getByRole('tab', { name: '知识库测试', exact: true }).click();
    await p.getByText('此构建尚未配置 Telegram 应用身份。', { exact: true }).waitFor();
    assert(await button('删除配置').isEnabled(), 'Unconfigured build cannot remove saved local config');
    await p.evaluate(() => { window.signedOut = false; window.unconfigured = false; });
    await p.getByRole('tab', { name: '新建空间', exact: true }).click(); await provider('Telegram');
    await p.getByRole('combobox', { name: '私有频道' }).click(); await p.getByRole('option', { name: '只读频道（只读）', exact: true }).click();
    assert(await button('创建备份库').isDisabled(), 'Read-only channel can create a library');
    await p.getByRole('tab', { name: '知识库测试', exact: true }).click();
    await button('删除配置').click(); await p.waitForFunction(() => window.spaces.length === 1);
    assert(await p.evaluate(() => window.current) === 's3', 'Deleting Telegram changed the S3 selection');
    await p.locator('#backup-key').waitFor();
    const calls = await p.evaluate(() => window.calls);
    assert(calls.filter(c => c.command === 'save_backup_settings').every(c => c.provider === 'telegram' && c.activate === false), 'Saving violated activation or provider isolation');
    assert(!calls.some(c => ['login_tdl', 'telegram_storage_logout', 'delete_asset', 'release_asset'].includes(c.command)), 'Configuration flow touched account or media lifecycle');
    assert(failures.length === 0, failures.join('\n'));
    return { passed: true, viewports: 3, flows: ['connect-missing-library', 'explicit-create', 'busy-feedback', 'save-without-activation', 'switch-telegram-s3', 'readonly-channel', 'saved-target-readonly', 'persistent-space-error', 'local-config-delete', 'no-credential-access'] };
  } finally { await p.close(); }
}
