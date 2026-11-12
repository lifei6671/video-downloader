// After npm run build, serve dist with vite preview on 127.0.0.1:1421.
// playwright-cli --session s3-backup run-code --filename tests/startup-paint-browser.mjs
async (page) => {
  const preview = await page.context().newPage();
  const errors = [], requests = [];
  preview.on('pageerror', error => errors.push(error.message));
  preview.on('request', request => requests.push(request.url()));
  let releaseEntry, releaseNotes;
  const entryReady = new Promise(resolve => { releaseEntry = resolve; });
  const notesReady = new Promise(resolve => { releaseNotes = resolve; });
  const waitForRead = command => preview.waitForFunction(name => window.startupTest.calls.includes(name), command);
  const finishRead = (command, fail = false) => preview.evaluate(([name, rejected]) => window.startupTest.finish(name, rejected), [command, fail]);
  const splash = preview.getByText('正在启动…', { exact: true });
  const navigation = preview.getByRole('button', { name: '下载任务', exact: true });
  const checkMotion = async expected => {
    const motion = await preview.locator('.startup-mark').evaluate(mark => ({
      clip: getComputedStyle(mark.querySelector('.startup-clip')).animationName,
      icon: getComputedStyle(mark.querySelector('img')).animationName,
      ripple: getComputedStyle(mark, '::before').animationName,
    }));
    const expectedNames = expected ? ['startup-clip', 'startup-catch', 'startup-ripple'] : ['none', 'none', 'none'];
    if (Object.values(motion).join() !== expectedNames.join()) throw new Error('Unexpected startup animation: ' + JSON.stringify(motion));
  };
  try {
    await preview.emulateMedia({ reducedMotion: 'no-preference' });
    await preview.addInitScript(() => {
      const settings = { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [] };
      const video = { platform: 'douyin', video_id: '123', canonical_url: 'https://www.douyin.com/video/123', title: '已恢复的作品', resolver: 'yt-dlp', kind: 'video', selected_format_id: 'original', formats: [{ format_id: 'original', container: 'mp4' }] };
      const tasks = [{ id: 'restored', revision: 1, status: 'ready', video, bytes: 0, created_at: 1, canceling: false, download_requested: false, files: [] }];
      const reads = Object.fromEntries(Object.entries({ get_settings: settings, list_tasks: tasks }).map(([command, value]) => {
        let finish;
        const promise = new Promise((resolve, reject) => { finish = fail => fail ? reject('DATABASE_ERROR') : resolve(value); });
        return [command, { promise, finish }];
      }));
      window.startupTest = { calls: [], finish: (command, fail) => reads[command].finish(fail) };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
      window.__TAURI_INTERNALS__ = {
        convertFileSrc: path => `http://asset.localhost/${encodeURIComponent(path)}`,
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async command => {
          window.startupTest.calls.push(command);
          if (reads[command]) return reads[command].promise;
          if (command === 'list_video_transcripts') return [];
          if (command === 'check_runtime_dependencies') return { platform: 'windows', directory: 'C:/fixture', tools: [] };
          if (command === 'check_for_updates') return { current_version: '1.9.1', latest_version: '99.0.0', release_tag: 'v99.0.0', has_update: true, release_notes: '## 启动回归测试\n\n| 项目 | 状态 |\n| --- | --- |\n| 延迟加载 | 正常 |', published_at: null, installer: null };
          return null;
        },
      };
    });
    await preview.route('**/assets/main-*.js', async route => { await entryReady; await route.continue(); });
    await preview.route('**/assets/release-notes-*.js', async route => { await notesReady; await route.continue(); });
    await preview.goto(page.url().split('/').slice(0, 3).join('/') + '/', { waitUntil: 'commit' });
    await splash.waitFor();
    for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
      await preview.setViewportSize({ width, height });
      await preview.waitForFunction(() => {
        const logo = document.querySelector('.startup-screen img');
        return logo?.complete && logo.naturalWidth > 0;
      });
      const valid = await preview.locator('.startup-screen').evaluate(element => {
        const box = element.getBoundingClientRect();
        return box.width === innerWidth && box.height === innerHeight && getComputedStyle(document.body).backgroundColor === 'rgb(249, 249, 251)';
      });
      if (!valid) throw new Error(`Invalid startup screen at ${width}x${height}`);
    }
    await checkMotion(true);
    await preview.emulateMedia({ reducedMotion: 'reduce' });
    await checkMotion(false);
    await preview.emulateMedia({ reducedMotion: 'no-preference' });
    if (await navigation.count()) throw new Error('Main UI was rendered before its bundle loaded');
    releaseEntry();
    await waitForRead('get_settings');
    await splash.waitFor();
    if (await navigation.count()) throw new Error('Main UI appeared before settings loaded');
    await finishRead('get_settings');
    await waitForRead('list_tasks');
    await splash.waitFor();
    await checkMotion(true);
    if (await navigation.count()) throw new Error('Main UI appeared before tasks loaded');
    const blockedCalls = await preview.evaluate(() => {
      window.dispatchEvent(new Event('focus'));
      return window.startupTest.calls.filter(command => ['list_tasks', 'get_backup_settings', 'list_video_transcripts', 'check_runtime_dependencies'].includes(command));
    });
    if (blockedCalls.join() !== 'list_tasks') throw new Error('Unexpected IPC while loading the first page: ' + blockedCalls.join(', '));
    await finishRead('list_tasks');
    await navigation.waitFor();
    if (await preview.locator('.startup-screen').count()) throw new Error('Startup screen was not replaced');
    if (await preview.locator('#video-input').inputValue() !== 'https://www.douyin.com/video/123') throw new Error('Main UI appeared without the restored task');
    await preview.getByRole('button', { name: '系统设置', exact: true }).click();
    await preview.getByText('2 个', { exact: true }).waitFor();
    await preview.getByRole('button', { name: '关于软件', exact: true }).click();
    if (requests.some(url => /release-notes-.*\.js/.test(url))) throw new Error('Markdown loaded before update dialog');
    await preview.getByRole('button', { name: '检查更新', exact: true }).click();
    await preview.getByText('正在加载更新说明…', { exact: true }).waitFor();
    releaseNotes();
    await preview.getByRole('heading', { name: '启动回归测试', exact: true }).waitFor();
    await preview.getByRole('cell', { name: '延迟加载', exact: true }).waitFor();
    if (!requests.some(url => /release-notes-.*\.js/.test(url))) throw new Error('Lazy chunk was not requested');
    for (const failedRead of ['get_settings', 'list_tasks']) {
      await preview.reload({ waitUntil: 'commit' });
      await waitForRead('get_settings');
      if (failedRead === 'list_tasks') {
        await finishRead('get_settings');
        await waitForRead('list_tasks');
      }
      await finishRead(failedRead, true);
      await preview.getByRole('alert').getByText('无法启动映取，请从托盘退出后重试。').waitFor();
      await checkMotion(false);
      if (await navigation.count()) throw new Error('Main UI appeared despite a failed startup read');
    }
    await preview.route('**/assets/main-*.js', route => route.abort());
    await preview.reload({ waitUntil: 'commit' });
    const bundleError = preview.getByRole('alert');
    await bundleError.getByText('无法加载映取界面，请从托盘退出后重试。').waitFor();
    await bundleError.getByRole('img', { name: '映取' }).waitFor();
    await checkMotion(false);
    if (errors.length) throw new Error(errors.join('\n'));
    return { result: 'PASS', viewports: 3, startupBeforeMainBundle: true, startupMotion: true, reducedMotion: true, waitsForSettingsAndTasks: true, restoresFirstPage: true, staticFailureStates: true, lazyReleaseNotes: true, gfmTable: true, pageErrors: errors.length };
  } finally {
    releaseEntry();
    releaseNotes();
    await preview.unrouteAll({ behavior: 'wait' });
    await preview.close();
  }
}
