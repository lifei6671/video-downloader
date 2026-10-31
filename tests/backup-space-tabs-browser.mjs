// Run against the production preview with playwright-cli run-code --filename.
async (page) => {
  const target = await page.context().newPage();
  await target.emulateMedia({ reducedMotion: 'reduce' });
  const errors = [];
  target.on('pageerror', error => errors.push(error.message));
  await target.addInitScript(() => {
    const base = { provider_type: 's3', profile: 'r2', region: 'auto', path_style: true, endpoint: 'https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com', prefix: 'test/', enabled: false, credentials_configured: true, protocol: 2, last_error: null };
    window.tabSpaces = [{ ...base, space_id: 'a', library_id: 'lib-a', bucket: 'space-a', enabled: true }, { ...base, space_id: 'b', library_id: 'lib-b', bucket: 'space-b', profile: 's3', region: 'us-east-1', endpoint: 'https://s3.example.com' }, { ...base, space_id: 'old', library_id: 'lib-old', bucket: 'old-bucket', protocol: 2 }];
    window.tabCurrent = 'a'; window.tabCalls = []; window.tabNewId = 0; window.failTabSwitch = false;
    const credentials = {};
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = { convertFileSrc: path => `http://asset.localhost/${encodeURIComponent(path)}`, transformCallback: () => 1, unregisterCallback: () => {}, invoke: async (command, args) => {
      window.tabCalls.push({ command, args });
      if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
      if(command==='get_backup_sync_enabled')return true;
      if (command === 'get_backup_settings') return window.tabSpaces.find(s => s.space_id === window.tabCurrent) ?? null;
      if (command === 'list_backup_spaces') return window.tabSpaces.map(s => ({ ...s }));
      if (command === 'get_backup_credentials') return credentials[args.spaceId] ?? { access_key_id: `key-${args.spaceId}`, secret_access_key: `secret-${args.spaceId}` };
      if (command === 'rename_backup_space') {
        if (window.failTabRename) throw 'DATABASE_ERROR';
        if (window.holdTabRename) await new Promise(resolve => { window.releaseTabRename = resolve; });
        const value = window.tabSpaces.find(s => s.space_id === args.spaceId);
        value.display_name = args.displayName.trim(); return { ...value };
      }
      if (['list_tasks', 'list_video_transcripts', 'list_backup_assets', 'list_backup_operations'].includes(command)) return [];
      if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
      if (command === 'save_backup_settings') {
        if (args.config.activate !== false) throw 'FIXTURE_SAVE_ACTIVATED';
        const { activate, credentials: nextCredentials, ...config } = args.config;
        let value = window.tabSpaces.find(s => s.endpoint === config.endpoint && s.bucket === config.bucket && s.prefix === config.prefix);
        if (!value) { value = { ...config, space_id: `saved-${++window.tabNewId}`, library_id: `saved-library-${window.tabNewId}`, protocol: 2, credentials_configured: true, last_error: null }; window.tabSpaces.push(value); }
        Object.assign(value, config, { enabled: value.space_id === window.tabCurrent && config.enabled });
        if (nextCredentials) credentials[value.space_id] = nextCredentials;
        return { ...value };
      }
      if (command === 'select_backup_space') {
        if (window.failTabSwitch) throw 'BACKUP_NETWORK_ERROR';
        const value = window.tabSpaces.find(s => s.space_id === args.spaceId);
        window.tabCurrent = value.space_id; window.tabSpaces.forEach(s => s.enabled = s.space_id === value.space_id);
        return { ...value };
      }
      return null;
    } };
  });
  const tab = name => target.getByRole('tab', { name, exact: true });
  const button = name => target.getByRole('button', { name, exact: true });
  const ready = () => target.locator('#backup-key:not(:disabled)').waitFor();
  try {
    await target.goto(page.url().split('/').slice(0, 3).join('/') + '/');
    await button('系统设置').click(); await button('远端备份').click(); await ready();
    if (await target.getByRole('textbox', { name: '空间名称', exact: true }).count()) throw new Error('Saved space still has name field below tabs');
    await tab('R2 / space-a').hover();
    const nameTooltip = target.getByRole('tooltip'); await nameTooltip.waitFor();
    if ((await nameTooltip.textContent()).trim() !== 'R2 / space-a') throw new Error('Space tooltip exposes target details');
    await target.mouse.move(0, 0); await target.keyboard.press('Escape'); await nameTooltip.waitFor({state:'hidden'});

    if (await target.getByRole('combobox', { name: '备份空间' }).count()) throw new Error('Space dropdown remains');
    await tab('S3 / space-b').click(); await ready();
    await target.getByLabel('Secret Access Key', { exact: true }).fill('pending-b-secret');
    await tab('R2 / space-a').click(); await ready();
    if (await target.getByLabel('Secret Access Key', { exact: true }).inputValue() !== 'secret-a') throw new Error('A credential changed');
    await tab('S3 / space-b').click(); await ready();
    if (await target.getByLabel('Secret Access Key', { exact: true }).inputValue() !== 'pending-b-secret') throw new Error('Unsaved B draft lost');
    if (await target.evaluate(() => window.tabCurrent) !== 'a') throw new Error('Browsing changed default');
    if (!await button('设为默认').isDisabled()) throw new Error('Unsaved credentials may activate');
    await button('保存配置').click(); await ready();
    if (await target.evaluate(() => window.tabCurrent) !== 'a') throw new Error('Saving changed default');
    await target.evaluate(() => { window.failTabSwitch = true; });
    await button('设为默认').click(); await ready();
    await target.locator('[data-sonner-toast]').filter({ hasText: '无法连接远端存储' }).waitFor();
    if (await target.evaluate(() => window.tabCurrent) !== 'a') throw new Error('Failed switch changed default');
    await target.evaluate(() => { window.failTabSwitch = false; });
    await button('设为默认').click(); await ready();
    await target.waitForFunction(() => window.tabCurrent === 'b');
    if (!await tab('S3 / space-b').getByText('默认', { exact: true }).count()) throw new Error('Default marker missing');
    await tab('R2 / space-a').click(); await ready();
    if (await button('设为默认').isDisabled()) throw new Error('Pristine A draft became dirty after switch');
    await tab('新建空间').click();
    await target.getByLabel('空间名称', { exact: true }).fill('备用存储');
    await target.getByRole('combobox', { name: '备份提供商' }).click(); await target.getByRole('option', { name: '通用 S3', exact: true }).click();
    await target.getByLabel('S3 Endpoint', { exact: true }).fill('https://new.example.com');
    await target.getByLabel('Bucket', { exact: true }).fill('space-new');
    await target.getByLabel('Prefix', { exact: true }).fill('new/');
    await target.getByLabel('Access Key ID', { exact: true }).fill('new-key');
    await target.getByLabel('Secret Access Key', { exact: true }).fill('new-secret');
    await tab('S3 / space-b').click(); await ready();
    await tab('新建空间').click();
    if (await target.getByLabel('Secret Access Key', { exact: true }).inputValue() !== 'new-secret') throw new Error('New draft lost');
    if (await button('设为默认').count()) throw new Error('New draft exposes set-default before saving');
    if (await button('保存配置').count()) throw new Error('New draft still uses save label');
    const cleanup = target.getByRole('checkbox',{name:'自动清理已删除记录',exact:true});
    if(await cleanup.getAttribute('aria-checked')!=='false')throw new Error('Cleanup enabled by default');
    await cleanup.click();
    const days = target.locator('#backup-retention-days');
    const beforeInvalidSave = await target.evaluate(()=>window.tabCalls.filter(c=>c.command==='save_backup_settings').length);
    for(const invalid of ['0','3651','1.5']){
      await days.fill(invalid);await button('新建配置').click();
      await target.locator('#backup-retention-days-error').waitFor();
      if(await target.evaluate(()=>window.tabCalls.filter(c=>c.command==='save_backup_settings').length)!==beforeInvalidSave)throw new Error('Invalid retention days reached IPC');
    }
    await days.fill('14');
    await tab('S3 / space-b').click();await ready();await tab('新建空间').click();await ready();
    if(await days.inputValue()!=='14'||await cleanup.getAttribute('aria-checked')!=='true')throw new Error('Retention draft lost while browsing spaces');
    await button('新建配置').click(); await ready();
    await button('保存配置').waitFor();
    if (await button('新建配置').count()) throw new Error('Saved configuration still uses create label');
    if (await target.locator('#backup-retention-days').inputValue() !== '14') throw new Error('Retention setting not restored after creation');
    await tab('S3 / space-b').click(); await ready();
    if (await target.getByRole('checkbox',{name:'自动清理已删除记录',exact:true}).getAttribute('aria-checked') !== 'false') throw new Error('Retention policy leaked to another space');
    await tab('备用存储').click(); await ready();
    if (await target.locator('#backup-retention-days').inputValue() !== '14') throw new Error('Saved retention policy lost on switching tabs');
    const saved = tab('备用存储'); await saved.waitFor();
    if (await target.getByRole('tablist', { name: '备份空间' }).getByRole('tab').count() !== 5) throw new Error('New space tab was not appended');
    if (await target.evaluate(() => window.tabCurrent) !== 'b') throw new Error('New config took over default');
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await target.setViewportSize({ width, height });
      await saved.scrollIntoViewIfNeeded();
      if (await target.evaluate(() => document.documentElement.scrollWidth > innerWidth)) throw new Error('Tabs overflow window');
      await tab('新建空间').focus(); await target.keyboard.press('Home'); await ready();
      await target.waitForFunction(() => document.querySelector('[role="tab"][aria-label="R2 / space-a"]')?.getAttribute('aria-selected') === 'true');
      if (await tab('R2 / space-a').getAttribute('aria-selected') !== 'true') throw new Error('Tab keyboard navigation failed');
      if (await target.evaluate(() => window.tabCurrent) !== 'b') throw new Error('Keyboard browsing switched default');
      await saved.click(); await ready();
      const metrics = e => { const s = getComputedStyle(e); return { font: s.fontSize, left: s.paddingLeft, right: s.paddingRight, color: s.color, height: e.getBoundingClientRect().height }; };
      await button('平台账号').click();
      await tab('Telegram').click();
      const accountStyle = await tab('Telegram').evaluate(metrics);
      await button('远端备份').click(); await ready(); await saved.click(); await ready();
      const backupStyle = await saved.evaluate(metrics);
      if (JSON.stringify(accountStyle) !== JSON.stringify(backupStyle)) throw new Error(`Settings tab styles differ: ${JSON.stringify({accountStyle, backupStyle})}`);
      await target.screenshot({ path: `output/playwright/backup-tabs-named-${width}.png` });
    }
    await button('设为默认').click(); await ready(); await target.waitForFunction(() => window.tabCurrent === 'saved-1');
    await tab('R2 / old-bucket').click(); await ready();
    const renameInput = target.getByRole('textbox', { name: '编辑空间名称', exact: true });
    const renameCount = () => target.evaluate(() => window.tabCalls.filter(c => c.command === 'rename_backup_space').length);
    const firstRename = await renameCount();
    await tab('R2 / old-bucket').dblclick(); await renameInput.waitFor();
    if (await target.locator('button input').count()) throw new Error('Rename input nested inside a button');
    await renameInput.fill('取消改名'); await target.keyboard.press('Escape');
    await tab('R2 / old-bucket').waitFor();
    if (await renameCount() !== firstRename) throw new Error('Escape saved a name');
    await tab('R2 / old-bucket').focus(); await target.keyboard.press('F2'); await renameInput.waitFor();
    await renameInput.fill('素材归档');
    await renameInput.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true, bubbles: true });
    if (await renameCount() !== firstRename || !await renameInput.isVisible()) throw new Error('IME Enter saved a name');
    await target.evaluate(() => { window.holdTabRename = true; });
    await target.keyboard.press('Enter');
    await target.waitForFunction(() => !!window.releaseTabRename);
    if (!await renameInput.isDisabled()) throw new Error('Pending rename remains editable');
    await target.evaluate(() => { window.holdTabRename = false; window.releaseTabRename(); });
    await tab('素材归档').waitFor(); await ready();
    if (await renameCount() !== firstRename + 1) throw new Error('Enter and blur saved twice');
    if (await target.evaluate(() => window.tabCurrent) !== 'saved-1') throw new Error('Rename changed default');
    await button('关于软件').click(); await button('系统设置').click(); await ready();
    await tab('素材归档').click(); await ready();
    if (await target.getByRole('textbox', { name: '空间名称', exact: true }).count()) throw new Error('Saved name field returned after reload');
    await target.getByLabel('Access Key ID', { exact: true }).fill('replacement-key');
    await target.getByLabel('Secret Access Key', { exact: true }).fill('replacement-secret');
    await tab('素材归档').dblclick(); await renameInput.waitFor();
    await renameInput.fill('素材归档（二）');
    await target.evaluate(() => { window.failTabRename = true; });
    await target.keyboard.press('Enter');
    await target.waitForFunction(() => document.querySelector('.backup-space-name-editor')?.getAttribute('aria-invalid') === 'true');
    if (await renameInput.inputValue() !== '素材归档（二）') throw new Error('Failed rename discarded input');
    if (await target.evaluate(() => window.tabSpaces.find(s => s.space_id === 'old').display_name) !== '素材归档') throw new Error('Failed rename changed persisted name');
    await target.evaluate(() => { window.failTabRename = false; });
    await target.keyboard.press('Enter'); await tab('素材归档（二）').waitFor(); await ready();
    if (await target.getByLabel('Access Key ID', { exact: true }).inputValue() !== 'replacement-key') throw new Error('Local rename dropped pending credentials');
    await tab('素材归档（二）').dblclick(); await renameInput.waitFor(); await renameInput.fill('失焦保存');
    await target.getByLabel('Prefix', { exact: true }).focus();
    await tab('失焦保存').waitFor(); await ready();
    // A view change during blur must not copy the renamed space's credentials to the new view.
    await tab('失焦保存').dblclick(); await renameInput.waitFor(); await renameInput.fill('素材归档（三）');
    await tab('S3 / space-b').click(); await ready();
    await tab('素材归档（三）').waitFor();
    if (await target.getByLabel('Secret Access Key', { exact: true }).inputValue() !== 'pending-b-secret') throw new Error('Blur rename contaminated another space');
    await tab('素材归档（三）').click(); await ready();
    if (await target.getByLabel('Access Key ID', { exact: true }).inputValue() !== 'replacement-key') throw new Error('Rename lost pending credentials after view change');
    await button('保存配置').click(); await ready();
    await button('设为默认').click(); await target.waitForFunction(() => window.tabCurrent === 'old');
    if (await target.getByRole('alertdialog').count()) throw new Error('Set default opened an upgrade flow');
    const effects = await target.evaluate(() => window.tabCalls.filter(c => ['select_backup_space', 'save_backup_settings', 'upgrade_backup_space'].includes(c.command)));
    if (effects.filter(c => c.command === 'save_backup_settings').some(c => c.args.config.activate !== false)) throw new Error('Save requested activation');
    if (errors.length) throw new Error(errors.join('\n'));
    return { result: 'PASS', viewports: 3, accountTabStyleMatched: true, inlineRename: true, renameEnterBlurEscapeF2Ime: true, renameFailurePreservesDraft: true, nameReloaded: true, savedTabAdded: true, retentionPolicy: true, browsingDoesNotActivate: true, saveDoesNotActivate: true, draftsPreserved: true, switchFailureSafe: true, singleProtocol: true, effects: effects.length, pageErrors: 0 };
  } catch (error) {
    const state = await target.evaluate(() => ({ selected: [...document.querySelectorAll('[role="tablist"][aria-label="备份空间"] [role="tab"][aria-selected="true"]')].map(e => e.getAttribute('aria-label')), renaming: document.querySelector('.backup-space-name-editor')?.value, renameDisabled: document.querySelector('.backup-space-name-editor')?.disabled, names: window.tabSpaces?.map(s => [s.space_id, s.display_name]), renameCalls: window.tabCalls?.filter(c => c.command === 'rename_backup_space').map(c => c.args) }));
    throw new Error(`${String(error)}; fixture state: ${JSON.stringify(state)}`);
  } finally { await target.close(); }
}
