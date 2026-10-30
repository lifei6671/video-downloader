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
  try {
    await preview.addInitScript(() => {
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
      window.__TAURI_INTERNALS__ = {
        convertFileSrc: path => `http://asset.localhost/${encodeURIComponent(path)}`,
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async command => {
          if (command === 'list_tasks' || command === 'list_video_transcripts') return [];
          if (command === 'get_settings') return { download_directory: 'C:/fixture', filename_template: '{title}', max_downloads: 2, profiles: [], tdl: { enabled: false, path: null } };
          if (command === 'check_for_updates') return { current_version: '1.9.1', latest_version: '99.0.0', release_tag: 'v99.0.0', has_update: true, release_notes: '## 启动回归测试\n\n| 项目 | 状态 |\n| --- | --- |\n| 延迟加载 | 正常 |', published_at: null, installer: null };
          return null;
        },
      };
    });
    await preview.route('**/assets/index-*.js', async route => { await entryReady; await route.continue(); });
    await preview.route('**/assets/release-notes-*.js', async route => { await notesReady; await route.continue(); });
    await preview.goto('http://127.0.0.1:1421/', { waitUntil: 'commit' });
    await preview.getByText('正在启动…', { exact: true }).waitFor();
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
    if (await preview.getByRole('button', { name: '下载任务', exact: true }).count()) throw new Error('Entry was not delayed');
    releaseEntry();
    await preview.getByRole('button', { name: '下载任务', exact: true }).waitFor();
    if (await preview.locator('.startup-screen').count()) throw new Error('Startup screen was not replaced');
    await preview.getByRole('button', { name: '关于软件', exact: true }).click();
    if (requests.some(url => /release-notes-.*\.js/.test(url))) throw new Error('Markdown loaded before update dialog');
    await preview.getByRole('button', { name: '检查更新', exact: true }).click();
    await preview.getByText('正在加载更新说明…', { exact: true }).waitFor();
    releaseNotes();
    await preview.getByRole('heading', { name: '启动回归测试', exact: true }).waitFor();
    await preview.getByRole('cell', { name: '延迟加载', exact: true }).waitFor();
    if (!requests.some(url => /release-notes-.*\.js/.test(url))) throw new Error('Lazy chunk was not requested');
    if (errors.length) throw new Error(errors.join('\n'));
    return { result: 'PASS', viewports: 3, startupBeforeJavaScript: true, reactHandoff: true, lazyReleaseNotes: true, gfmTable: true, pageErrors: errors.length };
  } finally {
    releaseEntry();
    releaseNotes();
    await preview.unrouteAll({ behavior: 'wait' });
    await preview.close();
  }
}
