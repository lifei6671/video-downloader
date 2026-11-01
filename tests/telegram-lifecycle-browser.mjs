// Synthetic IPC only. No real channel, session or local media is changed.
async page => {
  const results = [];
  for (const permission of [true, false]) for (const taskBacked of [false, true]) {
    const p = await page.context().newPage();
    const errors = []; p.on('pageerror', e => errors.push(e.message));
    await p.addInitScript(({canDelete, taskBacked}) => {
      localStorage.setItem('yingqu.task-view', 'list');
      window.calls = [];
      window.asset = { asset_id: 'fixture', task_id: null, metadata: { title: 'Telegram 合成资料', platform: 'youtube' }, original_filename: 'fixture.mp4', content_sha256: 'a'.repeat(64), content_bytes: 100, asset_state: 'active', path: 'C:/fixture/video.mp4', local_state: 'present', space_id: 'tg', remote_state: 'backed_up', has_cover: false, has_transcript: true, remote_revision: 4, remote_deleted: false, has_pending_local_changes: false, conflict_count: 0 };
      window.task = taskBacked ? { id: 'fixture-task', asset_id: 'fixture', revision: 1, status: 'completed', video: { ...window.asset.metadata, video_id: '12345678901', kind: 'video', canonical_url: 'https://www.youtube.com/watch?v=12345678901', resolver: 'yt-dlp', duration_ms: 1000, width: 1920, height: 1080 }, bytes: 100, total_bytes: 100, speed_bps: null, error: null, path: window.asset.path, created_at: 1, completed_at: 2, file_missing: false, canceling: false, download_requested: true, cover_cached: false, files: [] } : null;
      if (taskBacked) window.asset.task_id = 'fixture-task';
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = { convertFileSrc: x => x, transformCallback: () => 1, unregisterCallback() {}, invoke: async (command, args) => {
        window.calls.push({ command, args });
        const settings = { space_id: 'tg', library_id: 'lib', display_name: '知识库', provider_type: 'telegram', profile: 'telegram', endpoint: '', bucket: '', prefix: '', region: 'auto', path_style: true, enabled: true, credentials_configured: false, protocol: 2, connection_pending: false, last_error: null, can_delete_owned_media: canDelete };
        if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
        if (command === 'get_backup_settings') return settings;
        if (command === 'list_backup_spaces') return [settings];
        if (command === 'get_backup_sync_enabled') return true;
        if (command === 'list_backup_assets') return [structuredClone(window.asset)];
        if (command === 'list_tasks') return window.task ? [structuredClone(window.task)] : [];
        if (['list_video_transcripts', 'list_backup_operations'].includes(command)) return [];
        if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
        if (command === 'release_local_video') throw 'BACKUP_CHECKSUM_MISMATCH';
        if (command === 'delete_asset_and_backup') return;
        if (command === 'start_asset_backup') return;
        return null;
      } };
    }, {canDelete:permission,taskBacked});
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/') + '/');
      await p.getByRole('button', { name: '下载任务', exact: true }).click();
      const row = p.locator(taskBacked ? '[data-task-id="fixture-task"]' : '[data-asset-id="fixture"]'); await row.waitFor();
      await row.click({ button: 'right' });
      const item = name => p.getByRole('menuitem', { name, exact: true });
      if (await item('从当前同步库删除').count() !== (permission ? 1 : 0)) throw new Error('delete permission not reflected in menu');
      if (!permission && await item('删除本地视频').count()) throw new Error('unavailable Telegram deletion exposed as local delete');
      await item('释放本机空间').click();
      await p.getByText('远端内容校验失败', { exact: false }).first().waitFor();
      if (!await p.evaluate(() => window.asset.path)) throw new Error('failed verification removed local state');
      if (permission && !taskBacked) {
        await row.click({ button: 'right' }); await item('从当前同步库删除').click();
        await p.getByText('已加入删除队列', { exact: true }).waitFor();
        if (!await p.evaluate(() => window.asset.path)) throw new Error('intent prematurely removed local file');
      }
      await p.evaluate(() => { window.asset.remote_deleted = true; window.asset.remote_state = 'deleted'; });
      await row.getByRole('button', { name: '重新上传', exact: true }).waitFor();
      await row.getByRole('button', { name: '重新上传', exact: true }).click();
      await p.waitForFunction(() => window.calls.some(c => c.command === 'start_asset_backup' && c.args.spaceId === 'tg' && c.args.assetId === 'fixture'));
      for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
        await p.setViewportSize({ width, height });
        if (await p.evaluate(() => document.documentElement.scrollWidth > innerWidth)) throw new Error('viewport overflow');
      }
      if (errors.length) throw new Error(errors.join('\n'));
      results.push({ taskBacked, canDelete: permission, releaseFailureRetainsLocal: true, explicitReupload: true, viewports: 3 });
    } finally { await p.close(); }
  }
  return { passed: true, results };
}
