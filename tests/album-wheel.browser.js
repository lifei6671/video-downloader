// Production album viewer, isolated images/media and mocked native IPC.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const dialog = () => page.locator('.album-dialog');
  const selected = async number => {
    await page.waitForFunction(number => document.querySelector(`.album-dialog [aria-label="查看第 ${number} 张图片"]`)?.getAttribute('aria-pressed') === 'true', number);
  };
  const current = () => dialog().locator('[aria-pressed="true"]').getAttribute('aria-label');
  // Allow a new gesture after the implementation's 420 ms idle window.
  const idle = () => page.waitForTimeout(450);
  const open = async scenario => {
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=' + scenario);
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
    await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('图集');
    await page.locator('[data-task-id="visual-0"]').getByRole('button', { name: '查看图集', exact: true }).click();
    await dialog().waitFor();
  };
  const wheel = async deltaY => {
    await dialog().locator('.album-stage').hover();
    await page.mouse.wheel(0, deltaY);
  };
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height });
    await open('visual-album-sizing');
    await dialog().locator('.album-stage .album-still:not([aria-hidden]) > img:last-child').evaluate(image => image.decode());
    const background = page.locator('.task-browser [role="tabpanel"]');
    const backgroundScroll = await background.evaluate(element => element.scrollTop);
    await wheel(-120);
    await page.waitForTimeout(30);
    check(await current() === '查看第 1 张图片', 'first image wrapped backwards');
    await idle(); await wheel(120); await selected(2);
    check(await dialog().locator('.album-still-outgoing').count() === 0, 'reduced motion retained an outgoing image');
    // Supply input timestamps explicitly: browser IPC/render latency must not
    // turn a simulated inertial burst into several separate gestures.
    let stamp = await page.evaluate(() => performance.now() + 500);
    const input = async (deltaY, at) => dialog().locator('.album-gallery').evaluate((element, { deltaY, at }) => {
      const event = new WheelEvent('wheel', { deltaY, bubbles: true, cancelable: true });
      Object.defineProperty(event, 'timeStamp', { value: at });
      return element.dispatchEvent(event);
    }, { deltaY, at });
    await input(120, stamp); await selected(3);
    await input(80, stamp + 20); await input(40, stamp + 40);
    check(await current() === '查看第 3 张图片', 'one wheel burst skipped multiple images');
    stamp += 500; await input(-120, stamp); await selected(2);
    results.push(`${width}: wheel directions and gesture coalescing`);
    await dialog().locator('.album-gallery').evaluate(element => [
      { deltaX: 100, deltaY: 0 }, { deltaY: 100, ctrlKey: true }, { deltaY: 100, shiftKey: true }, { deltaY: 0 },
    ].forEach(options => element.dispatchEvent(new WheelEvent('wheel', { ...options, bubbles: true, cancelable: true }))));
    check(await current() === '查看第 2 张图片', 'horizontal scrolling or zoom changed the selected image');
    const handled = !await input(80, stamp + 20);
    check(handled, 'vertical wheel default scrolling was not prevented');
    results.push(`${width}: horizontal/modified wheel preserved; vertical scrolling contained`);
    await dialog().getByRole('button', { name: '查看第 6 张图片', exact: true }).click();
    check(await dialog().locator('.album-still-outgoing').count() === 0, 'reduced motion animated a dot click');
    stamp += 500; await input(120, stamp);
    check(await current() === '查看第 6 张图片', 'last image wrapped forwards');
    stamp += 500; await input(-120, stamp); await selected(5);
    await page.waitForFunction(() => document.querySelector('.album-dialog .album-stage .album-still:not([aria-hidden]) > img:last-child')?.naturalWidth === 2400);
    await dialog().getByRole('region', {name:'任务详细信息'}).dispatchEvent('wheel', {deltaY:120,bubbles:true,cancelable:true});
    check(await current() === '查看第 5 张图片', 'wheel over details changed media');
    check(await background.evaluate(element => element.scrollTop) === backgroundScroll, 'background scrolled while changing images');
    results.push(`${width}: first/last boundaries, thumbnail state and aspect ratio`);
    await dialog().locator('.album-gallery').evaluate(element => { window.closedWheelDialog = element; });
    await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'detached' });
    check(await page.evaluate(() => window.closedWheelDialog.dispatchEvent(new WheelEvent('wheel', { deltaY: 100, cancelable: true }))), 'wheel listener retained after closing');
  }
  await open('visual-album-sizing&long-details=1');
  const details = dialog().getByRole('region', {name:'任务详细信息'});
  await details.hover();
  await page.mouse.wheel(0, 160);
  await page.waitForFunction(() => document.querySelector('.task-details-content').scrollTop > 0);
  check(await current() === '查看第 1 张图片', 'scrolling the details selected another image');
  results.push('right pane scrolls independently with a real wheel gesture');
  await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'detached' });
  await open('visual-album-live');
  await page.waitForFunction(() => document.querySelector('.album-dialog video')?.currentTime > 0.1);
  await dialog().locator('video').evaluate(video => { window.wheelOldVideo = video; });
  await wheel(120); await selected(2);
  await page.waitForFunction(() => document.querySelector('.album-dialog video')?.videoWidth > document.querySelector('.album-dialog video')?.videoHeight);
  check(await page.evaluate(() => window.wheelOldVideo.paused && !window.wheelOldVideo.hasAttribute('src')), 'wheel switch retained previous live clip');
  await idle(); await wheel(120); await selected(3);
  check(await dialog().locator('video').count() === 0, 'wheel did not switch live clip to a still');
  results.push('live-photo wheel selection releases playback and supports mixed albums');
  await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'detached' });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const openWithSlowImage = async reverseDuringFade => {
    await open('visual-album-sizing');
    await dialog().getByRole('img', { name: '第 1 张图片' }).evaluate(image => image.decode());
    await dialog().locator('.album-stage').evaluate(async (stage, reverseDuringFade) => {
      const { api } = await import('/src/api.ts');
      const readImage = api.image;
      const gate = new Promise(resolve => { window.releaseAlbumImage = resolve; });
      api.image = async (id, index) => {
        if (index === 1) await gate;
        return readImage(id, index);
      };
      window.albumFadeEvents = [];
      for (const event of ['animationstart', 'animationend']) {
        stage.addEventListener(event, ({ type, animationName }) => {
          if (animationName !== 'album-wheel-fade') return;
          window.albumFadeEvents.push(type);
          if (reverseDuringFade && window.albumFadeEvents.length === 1) {
            stage.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true, cancelable: true }));
          }
        });
      }
    }, reverseDuringFade);
  };
  await openWithSlowImage(false);
  await wheel(120); await selected(2);
  const outgoing = dialog().locator('.album-still-outgoing');
  check(await outgoing.evaluate(layer => getComputedStyle(layer).opacity === '1' && layer.lastElementChild.naturalWidth === 900), 'previous image disappeared before the next image decoded');
  await page.evaluate(() => window.releaseAlbumImage());
  await outgoing.waitFor({ state: 'detached' });
  check(await page.evaluate(() => window.albumFadeEvents.join(',') === 'animationstart,animationend'), 'wheel switch did not run a complete fade');
  check(await dialog().getByRole('img', { name: '第 2 张图片' }).evaluate(image => image.naturalWidth === 1600), 'next image was not ready after the fade');
  results.push('a slow incoming image keeps the previous image visible, then runs the wheel fade');
  await openWithSlowImage(true);
  await wheel(120); await selected(2);
  await outgoing.waitFor();
  await idle();
  await page.evaluate(() => window.releaseAlbumImage());
  await selected(1);
  await outgoing.waitFor({ state: 'detached' });
  check(await dialog().getByRole('img', { name: '第 1 张图片' }).evaluate(image => image.naturalWidth === 900), 'reversing during a fade did not restore the previously loaded image');
  check(await page.evaluate(() => window.albumFadeEvents.join(',') === 'animationstart,animationend,animationstart,animationend'), 'reverse gesture interrupted an unfinished fade');
  results.push('reversing an in-progress fade restores the previous image');
  await openWithSlowImage(false);
  const fixedSize = await dialog().boundingBox();
  await dialog().getByRole('button', { name: '查看第 2 张图片', exact: true }).click();
  await selected(2);
  check(await outgoing.evaluate(layer => getComputedStyle(layer).opacity === '1' && layer.lastElementChild.naturalWidth === 900), 'dot click hid the current slide before the next image decoded');
  await page.evaluate(() => window.releaseAlbumImage());
  await outgoing.waitFor({ state: 'detached' });
  check(await page.evaluate(() => window.albumFadeEvents.join(',') === 'animationstart,animationend'), 'dot click did not run a complete fade');
  await page.evaluate(() => { window.albumFadeEvents = []; });
  const nextDot = dialog().getByRole('button', { name: '查看第 3 张图片', exact: true });
  await nextDot.focus();
  await page.keyboard.press('Enter');
  await selected(3);
  await page.waitForFunction(() => window.albumFadeEvents.join(',') === 'animationstart,animationend');
  const afterSwitch = await dialog().boundingBox();
  check(fixedSize.width === afterSwitch.width && fixedSize.height === afterSwitch.height, 'dot activation changed the dialog size');
  results.push('click and keyboard dot activation share the fade and keep the dialog size fixed');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const [width, height, feed] of [[1440, 900, false], [1280, 800, true], [1024, 680, false]]) {
    await page.setViewportSize({ width, height });
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-media-wheel');
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
    await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播');
    await page.locator('[data-asset-id="wheel-cloud"]').waitFor();
    const toggle = page.getByRole('button', { name: feed ? '切换为信息流' : '切换为列表', exact: true });
    if (await toggle.isVisible()) await toggle.click();
    check(await page.getByRole('list', { name: '媒体任务列表' }).getAttribute('data-view') === (feed ? 'feed' : 'list'), 'test opened the wrong task view');
    const trigger = page.locator('[data-task-row="visual-3"]').getByRole('button', { name: '播放视频', exact: true });
    const stage = page.locator('.task-media-panel, .video-player-dialog:not(.task-media-dialog) .video-player-stage');
    const active = page.locator('[data-slot="dialog-content"][data-state="open"]');
    const scroll = async direction => {
      await page.waitForTimeout(780);
      await stage.hover();
      await page.mouse.wheel(0, direction * 120);
    };
    const expectVideo = async title => {
      await active.getByRole('heading', { name: title, exact: true }).waitFor();
      await page.waitForFunction(() => {
        const video = document.querySelector('.video-player-dialog video');
        return video && !video.paused && video.currentTime > 0;
      });
    };
    const expectAlbum = async (title, number, count) => {
      await active.getByRole('heading', { name: title, exact: true }).waitFor();
      await dialog().locator('.album-gallery > [role="status"]').filter({ hasText: new RegExp('^' + number + ' / ' + count + '，') }).waitFor();
    };
    await trigger.press('Enter');
    await expectVideo('连播视频 A');
    await scroll(-1);
    check(await active.getByRole('heading', { name: '连播视频 A' }).isVisible(), 'first work wrapped backwards');
    await active.evaluate(element => {
      window.wheelStartDialog = element;
      window.wheelStartVideo = element.querySelector('video');
      element.addEventListener('wheel', event => { window.lastMediaWheel = event.timeStamp; }, { once: true });
    });
    await scroll(1);
    await expectAlbum('连播图集 B', 1, 3);
    await stage.evaluate(element => {
      const tail = new WheelEvent('wheel', { deltaY: 60, bubbles: true, cancelable: true });
      Object.defineProperty(tail, 'timeStamp', { value: window.lastMediaWheel + 20 });
      element.dispatchEvent(tail);
    });
    await expectAlbum('连播图集 B', 1, 3);
    check(await active.evaluate(element => element === window.wheelStartDialog && window.wheelStartVideo.paused && !window.wheelStartVideo.hasAttribute('src')), 'work switch remounted the dialog or left the old video playing');
    await scroll(1); await expectAlbum('连播图集 B', 2, 3);
    await scroll(1); await expectAlbum('连播图集 B', 3, 3);
    await scroll(1); await expectAlbum('连播单张图集 C', 1, 1);
    await scroll(1); await expectVideo('连播视频 D');
    await scroll(1); await expectVideo('连播远端视频 E');
    await scroll(1);
    check(await active.getByRole('heading', { name: '连播远端视频 E' }).isVisible(), 'last work wrapped forwards');
    await scroll(-1); await expectVideo('连播视频 D');
    await scroll(-1); await expectAlbum('连播单张图集 C', 1, 1);
    await scroll(-1); await expectAlbum('连播图集 B', 3, 3);
    await scroll(-1); await expectAlbum('连播图集 B', 2, 3);
    await scroll(-1); await expectAlbum('连播图集 B', 1, 3);
    await scroll(-1); await expectVideo('连播视频 A');
    check(await page.evaluate(() => window.backgroundTest.calls.filter(call => call.type === 'prepare-playback').map(call => call.id).join(',') === 'visual-3,visual-9,wheel-cloud,visual-9,visual-3'), 'playback prepared the wrong works');
    const bounds = await active.boundingBox();
    check(bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= width && bounds.y + bounds.height <= height, 'media dialog exceeds viewport');
    await page.keyboard.press('Escape');
    await active.waitFor({ state: 'detached' });
    check(await trigger.evaluate(element => element === document.activeElement), 'closing after wheel navigation lost focus restoration');
    results.push(`${width}: mixed media ${feed ? 'feed' : 'list'} follows filtered order in both directions and autoplays videos`);
  }
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-media-wheel');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播');
  const play = page.locator('[data-task-row="visual-9"]').getByRole('button', { name: '播放视频', exact: true });
  await play.press('Enter');
  const stage = page.locator('.task-media-panel, .video-player-dialog:not(.task-media-dialog) .video-player-stage');
  const heading = title => page.getByRole('dialog').getByRole('heading', { name: title, exact: true });
  await heading('连播视频 D').waitFor();
  await page.waitForFunction(() => document.querySelector('.video-player-stage video')?.currentTime > 0.05);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const load = api.assetPlaybackSource;
    api.assetPlaybackSource = async id => {
      await new Promise(resolve => { window.releaseWheelVideo = resolve; });
      return load(id);
    };
    const animate = Element.prototype.animate;
    window.workSlides = [];
    Element.prototype.animate = function (frames, options) {
      if (this.matches('.media-work-frame, .video-player-stage video')) {
        window.workSlides.push(frames.map(frame => frame.transform));
      }
      return animate.call(this, frames, options);
    };
  });
  const send = (delta, mode = 0) => stage.dispatchEvent('wheel', { deltaY: delta, deltaMode: mode, bubbles: true, cancelable: true });
  await send(-8); await send(-10);
  check(await heading('连播视频 D').isVisible(), 'tiny wheel deltas started a switch');
  check(await page.evaluate(() => window.workSlides.length === 0), 'tiny wheel deltas started an animation');
  await idle(); await send(120);
  await heading('连播远端视频 E').waitFor();
  const heldFrame = page.locator('canvas.media-work-frame');
  await heldFrame.waitFor();
  check(await heldFrame.evaluate(canvas => canvas.getContext('2d').getImageData(0, 0, 1, 1).data[3] === 255), 'loading did not preserve the previous video frame');
  await idle(); await send(-120);
  await page.waitForTimeout(800); await send(-120);
  check(await heading('连播远端视频 E').isVisible() && await heldFrame.isVisible(), 'wheel changed work during a slow load');
  check(await page.evaluate(() => window.workSlides.length === 0), 'slide started before the next video played');
  await page.evaluate(() => window.releaseWheelVideo());
  await page.waitForFunction(() => document.querySelector('.video-player-stage video')?.currentTime > 0);
  await heldFrame.waitFor({ state: 'detached' });
  await send(-120);
  check(await heading('连播远端视频 E').isVisible(), 'wheel skipped the video immediately after the slide');
  await page.waitForTimeout(750); await send(-1, 1);
  await heading('连播视频 D').waitFor();
  await page.waitForFunction(() => window.workSlides.length === 4);
  check(await page.evaluate(() => JSON.stringify(window.workSlides) === JSON.stringify([
    ['translateY(0)', 'translateY(-100%)'],
    ['translateY(100%)', 'translateY(0)'],
    ['translateY(0)', 'translateY(100%)'],
    ['translateY(-100%)', 'translateY(0)'],
  ])), 'work slides did not follow the wheel direction');
  await heldFrame.waitFor({ state: 'detached' });
  await page.evaluate(() => { window.previousWheelRelease = window.releaseWheelVideo; });
  await page.waitForTimeout(750); await send(120);
  await heldFrame.waitFor();
  await page.waitForFunction(() => window.releaseWheelVideo !== window.previousWheelRelease);
  await page.keyboard.press('Escape');
  await page.getByRole('dialog').waitFor({ state: 'detached' });
  await page.evaluate(() => window.releaseWheelVideo());
  await page.waitForTimeout(180);
  check(await page.getByRole('dialog').count() === 0, 'closing during a slide reopened the player');
  results.push('small deltas, playback-gated wheel, retained frames, directional slide, and cancellation');
  await play.press('Enter');
  await page.waitForFunction(() => document.querySelector('.video-player-stage video')?.currentTime > 0.05);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    api.assetPlaybackSource = async () => { throw 'FILE_MISSING'; };
  });
  await send(120);
  await heading('连播远端视频 E').waitFor();
  await page.getByRole('dialog').getByRole('alert').waitFor();
  await heldFrame.waitFor({ state: 'detached' });
  await page.waitForTimeout(750); await send(-120);
  await heading('连播视频 D').waitFor();
  results.push('source failure clears the old frame and allows browsing back');
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-media-wheel');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播');
  await page.locator('[data-task-row="visual-0"]').getByRole('button', { name: '查看图集', exact: true }).click();
  await dialog().getByRole('img', { name: '第 1 张图片' }).evaluate(image => image.decode());
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const load = api.image;
    const gate = new Promise(resolve => { window.releaseLastAlbumImage = resolve; });
    api.image = async (id, index) => {
      if (id === 'visual-0' && index === 2) await gate;
      return load(id, index);
    };
  });
  await dialog().getByRole('button', { name: '查看第 3 张图片', exact: true }).click();
  await dialog().locator('.album-still-outgoing > img:last-child').waitFor();
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    window.albumSwitchHadFrame = false;
    Element.prototype.animate = function (frames, options) {
      if (this.matches('.media-work-frame')) {
        window.albumSwitchHadFrame = this.getContext('2d').getImageData(0, 0, 1, 1).data[3] === 255;
      }
      return animate.call(this, frames, options);
    };
  });
  await send(120);
  await heading('连播单张图集 C').waitFor();
  await page.waitForFunction(() => window.albumSwitchHadFrame);
  await page.evaluate(() => window.releaseLastAlbumImage());
  results.push('switching away from a slow boundary image preserves the visible album frame');
  return { passed: results.length, results };
}
