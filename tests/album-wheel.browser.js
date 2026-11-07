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
  // Allow a new gesture after the implementation's 180 ms idle window.
  const idle = () => page.waitForTimeout(210);
  const open = async scenario => {
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=' + scenario);
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
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
  return { passed: results.length, results };
}
