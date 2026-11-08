// Fixed-size playback through the production context menu with mocked media IPC.
async page => {
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (condition, message) => { if (!condition) throw Error(message); };
  const stream = page.locator('.stream-playback-dialog');
  const media = stream.locator('.stream-playback-work:not(.stream-playback-outgoing) .task-media-panel');
  const outgoing = stream.locator('.stream-playback-outgoing');
  const toolbar = stream.locator('.stream-playback-controls');
  const visibleVideo = media.locator('.stream-video-slot:not([hidden]) video');
  const standbyVideo = media.locator('.stream-video-slot[hidden] video');
  const row = id => page.locator(`[data-task-row="${id}"]`);
  const active = title => page.getByRole('dialog', { name: title, exact: true });
  const menuItem = page.getByRole('menuitem', { name: '流式播放', exact: true });
  const results = [];
  const openStream = async (trigger, keyboard = false) => {
    if (keyboard) await trigger.press('Shift+F10');
    else await trigger.click({ button: 'right' });
    if (keyboard) await menuItem.press('Enter');
    else await menuItem.click();
    await stream.waitFor();
  };
  const expectPlaying = async title => {
    await active(title).waitFor();
    await page.waitForFunction(() => {
      const video = document.querySelector('.stream-playback-dialog .stream-video-slot:not([hidden]) video');
      return video && video.readyState >= 2 && !video.paused;
    });
  };
  const wheel = async (direction, delta = 120) => {
    // The switch waits for playback, motion and the subsequent hold.
    await page.waitForTimeout(1200);
    await media.hover();
    await page.mouse.wheel(0, direction * delta);
  };

  for (const [width, height, feed] of [[1440, 900, false], [1280, 800, true], [1024, 680, false], [1920, 720, false]]) {
    await page.setViewportSize({ width, height });
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-media-wheel');
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
    await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播');
    const remote = page.locator('[data-asset-id="wheel-cloud"]');
    await remote.waitFor();
    const toggle = page.getByRole('button', { name: feed ? '切换为信息流' : '切换为列表', exact: true });
    if (await toggle.isVisible()) await toggle.click();

    const unavailable = row('visual-1').locator('.task-context-target');
    await unavailable.click({ button: 'right' });
    await page.getByRole('menu', { name: '任务操作' }).waitFor();
    check(await menuItem.count() === 0, 'unavailable work exposed streaming playback');
    await page.keyboard.press('Escape');

    const trigger = row('visual-3').locator('.task-context-target');
    await openStream(trigger, feed);
    await expectPlaying('连播视频 A');
    const details = stream.getByRole('region', { name: '任务详细信息' });
    await details.getByRole('heading', { name: '连播视频 A', exact: true }).waitFor();
    const mediaBounds = await media.boundingBox();
    const detailsBounds = await details.boundingBox();
    check(detailsBounds.x >= mediaBounds.x + mediaBounds.width - 1, 'details did not stay in the right pane');
    check(await visibleVideo.evaluate(video => getComputedStyle(video).objectFit === 'contain' && video.loop), 'video cropped the source or did not enable looping');
    if (width === 1440) {
      await toolbar.getByRole('button', { name: '封面操作' }).waitFor();
      check(await toolbar.evaluate(element => {
        const buttons = [...element.querySelectorAll('button')].map(button => button.getBoundingClientRect());
        return buttons.length === 3 && buttons.every((button, i) => button.top === buttons[0].top && button.width === buttons[0].width
          && (i === 0 || button.left - buttons[i - 1].right >= 8));
      }), 'stream controls were not aligned or had insufficient spacing');
      await toolbar.getByRole('button', { name: '封面操作' }).click();
      await page.getByRole('menu', { name: '封面操作' }).waitFor();
      await page.keyboard.press('Escape');
      await visibleVideo.evaluate(video => { video.currentTime = video.duration - 0.1; });
      await page.waitForFunction(() => {
        const video = document.querySelector('.stream-playback-dialog .stream-video-slot:not([hidden]) video');
        return video && !video.paused && video.currentTime < 0.3;
      });
    }
    const dimensions = await stream.evaluate(element => {
      const size = () => [element.offsetWidth, element.offsetHeight];
      window.streamSizes = [size()];
      window.streamDialog = element;
      const observer = new ResizeObserver(() => window.streamSizes.push(size()));
      observer.observe(element);
      return size();
    });
    const expectedWidth = Math.min(width * 0.9, height * 0.9 * 16 / 9);
    check(Math.abs(dimensions[0] - expectedWidth) <= 1 && Math.abs(dimensions[1] - expectedWidth * 9 / 16) <= 1, 'streaming did not fit the 90% viewport at 16:9');
    const detailsWidth = await details.evaluate(element => element.parentElement.offsetWidth);
    check(Math.abs(detailsWidth - Math.max(296, Math.min(expectedWidth * 0.4, 460))) <= 1, 'details did not use the doubled width');

    await wheel(1);
    await active('连播图集 B').waitFor();
    await stream.getByRole('img', { name: '第 1 张图片' }).waitFor();
    if (width === 1440) {
      await stream.locator('.album-stage').evaluate(stage => {
        window.autoAlbumFades = 0;
        stage.addEventListener('animationstart', event => {
          if (event.animationName === 'album-wheel-fade') window.autoAlbumFades++;
        });
      });
      await stream.getByRole('button', { name: '查看第 2 张图片', pressed: true }).waitFor();
      await stream.getByRole('button', { name: '查看第 3 张图片' }).click();
      await stream.getByRole('button', { name: '查看第 3 张图片', pressed: true }).waitFor();
      await stream.getByRole('button', { name: '查看第 1 张图片', pressed: true }).waitFor();
      await page.waitForFunction(() => window.autoAlbumFades >= 3);
    }
    await stream.getByRole('button', { name: '查看第 3 张图片' }).click();
    await stream.getByRole('img', { name: '第 3 张图片' }).waitFor();
    await wheel(1);
    await active('连播单张图集 C').waitFor();
    await page.waitForFunction(() => {
      const video = document.querySelector('.stream-video-slot[hidden] video');
      return video?.readyState >= 3 && !video.seeking;
    });
    check(await standbyVideo.evaluate(video => {
      window.streamPreloadedVideo = video;
      return video.paused && !video.autoplay;
    }), 'preloading played or advanced the next video');
    check(await page.evaluate(() => window.backgroundTest.calls.filter(call => call.type === 'prepare-playback' && call.id === 'visual-9').length === 1), 'next local video was not prepared in advance');
    await wheel(1);
    await expectPlaying('连播视频 D');
    await details.getByRole('heading', { name: '连播视频 D', exact: true }).waitFor();
    check(await visibleVideo.evaluate(video => video === window.streamPreloadedVideo), 'switching did not reuse the buffered video');
    check(await page.evaluate(() => window.backgroundTest.calls.filter(call => call.type === 'prepare-playback' && call.id === 'visual-9').length === 1
      && !window.backgroundTest.calls.some(call => call.type === 'prepare-playback' && call.id === 'wheel-cloud')), 'switching refetched the local video or restored the cloud video early');
    if (width === 1440) {
      await page.evaluate(async () => {
        const { api } = await import('/src/api.ts');
        const load = api.assetPlaybackSource;
        api.assetPlaybackSource = async id => {
          await new Promise(resolve => { window.releaseStreamCloud = resolve; });
          return load(id);
        };
        const animate = Element.prototype.animate;
        window.streamPageSlides = [];
        window.streamSeamAnimations = [];
        document.addEventListener('playing', event => {
          if (event.target.closest('.stream-playback-dialog')) event.stopImmediatePropagation();
        }, true);
        Element.prototype.animate = function (frames, options) {
          const animation = animate.call(this, frames, options);
          if (this.matches('.stream-playback-work')) {
            const outgoing = this.matches('.stream-playback-outgoing');
            // Expose a narrow gap at the seam, as can happen during compositing.
            if (window.streamPageSlides.length < 2) {
              animation.pause();
              animation.currentTime = 210;
              if (outgoing) this.style.translate = '0 -4px';
              window.streamSeamAnimations.push(animation);
            }
            window.streamPageSlides.push({
              outgoing,
              hasBothPanes: !!this.querySelector('.task-media-panel') && !!this.querySelector('.task-media-sidebar'),
              transforms: frames.map(frame => frame.transform),
            });
          }
          return animation;
        };
      });
    }
    await wheel(1);
    if (width === 1440) {
      await active('连播远端视频 E').waitFor();
      await outgoing.waitFor();
      check(await outgoing.evaluate(element => element.inert && element.querySelector('.task-details-title')?.textContent === '连播视频 D'
        && !!element.querySelector('canvas.media-work-frame')), 'loading did not retain the complete previous page');
      check(await stream.evaluate(element => element.querySelector('.stream-playback-work:not(.stream-playback-outgoing) .task-media-sidebar')?.inert
        && document.activeElement === element), 'covered details could be focused or clicked while the old page was visible');
      await page.waitForTimeout(450);
      await media.dispatchEvent('wheel', { deltaY: -120, bubbles: true, cancelable: true });
      check(await active('连播远端视频 E').isVisible() && await outgoing.isVisible(), 'wheel changed work during a slow load');
      check(await page.evaluate(() => window.streamPageSlides.length === 0), 'page slid before the next video was ready');
      await page.waitForFunction(() => typeof window.releaseStreamCloud === 'function');
      await page.evaluate(() => window.releaseStreamCloud());
    }
    await expectPlaying('连播远端视频 E');
    if (width === 1440) {
      await page.waitForFunction(() => window.streamSeamAnimations.length === 2);
      const seam = await stream.evaluate(element => {
        const outgoing = element.querySelector('.stream-playback-outgoing').getBoundingClientRect();
        const incoming = element.querySelector('.stream-playback-work:not(.stream-playback-outgoing)').getBoundingClientRect();
        const sidebar = element.querySelector('.stream-playback-work:not(.stream-playback-outgoing) .task-media-sidebar');
        const bounds = sidebar.getBoundingClientRect();
        return {
          x: Math.floor(bounds.left + bounds.width / 2),
          y: Math.floor((outgoing.bottom + incoming.top) / 2),
          gap: incoming.top - outgoing.bottom,
          color: getComputedStyle(sidebar).backgroundColor,
        };
      });
      check(seam.gap >= 3, 'the seam sample was covered by a page');
      const screenshot = await page.screenshot({ clip: { x: seam.x, y: seam.y, width: 1, height: 1 } });
      const seamColor = await page.evaluate(async bytes => {
        const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const context = canvas.getContext('2d');
        context.drawImage(bitmap, 0, 0);
        const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
        return `rgb(${r}, ${g}, ${b})`;
      }, Array.from(screenshot));
      check(seamColor === seam.color, `detail seam revealed ${seamColor} instead of ${seam.color}`);
      // A paused animation never emits finish. Playback still has to unlock navigation.
      await outgoing.waitFor({ state: 'detached', timeout: 2000 });
      check(await details.evaluate(element => element === document.activeElement && !element.parentElement.inert), 'details did not regain focus after the slide');
    }
    await wheel(-1, 53);
    await expectPlaying('连播视频 D');
    if (width === 1440) {
      await page.waitForFunction(() => window.streamPageSlides.length === 4);
      check(await page.evaluate(() => window.streamPageSlides.every(slide => slide.hasBothPanes)
        && JSON.stringify(window.streamPageSlides.map(slide => [slide.outgoing, ...slide.transforms])) === JSON.stringify([
          [true, 'translateY(0)', 'translateY(-100%)'],
          [false, 'translateY(100%)', 'translateY(0)'],
          [true, 'translateY(0)', 'translateY(100%)'],
          [false, 'translateY(-100%)', 'translateY(0)'],
        ])), 'video and details did not slide together in both directions');
      await outgoing.waitFor({ state: 'detached' });
    }
    check(await stream.evaluate((element, [w, h]) =>
      element === window.streamDialog && window.streamSizes.every(([width, height]) => width === w && height === h),
    dimensions), 'dialog remounted or resized while switching media');
    if (width === 1440) {
      await toolbar.getByRole('button', { name: '最大化' }).press('Enter');
      await toolbar.getByRole('button', { name: '还原大小' }).waitFor();
      const fillsWindow = () => stream.evaluate(element => element.offsetWidth === innerWidth && element.offsetHeight === innerHeight);
      await page.waitForFunction(() => {
        const dialog = document.querySelector('.stream-playback-dialog.is-maximized');
        return dialog?.offsetWidth === innerWidth && dialog.offsetHeight === innerHeight;
      });
      await wheel(-1);
      await active('连播单张图集 C').waitFor();
      check(await fillsWindow(), 'switching did not retain maximized playback');
      await toolbar.getByRole('button', { name: '还原大小' }).click();
      await toolbar.getByRole('button', { name: '最大化' }).waitFor();
      await page.waitForFunction(([width, height]) => {
        const dialog = document.querySelector('.stream-playback-dialog:not(.is-maximized)');
        return dialog?.offsetWidth === width && dialog.offsetHeight === height;
      }, dimensions);
      await toolbar.getByRole('button', { name: '关闭' }).click();
    } else {
      await page.keyboard.press('Escape');
    }
    await stream.waitFor({ state: 'detached' });
    check(await trigger.evaluate(element => element === document.activeElement), 'closing did not restore context-menu focus');

    if (!feed && width === 1024) {
      await openStream(remote, true);
      await expectPlaying('连播远端视频 E');
      await page.keyboard.press('Escape');
      await stream.waitFor({ state: 'detached' });
      check(await remote.evaluate(element => element === document.activeElement), 'remote context-menu focus was not restored');
      await row('visual-3').getByRole('button', { name: '播放视频', exact: true }).click();
      await page.getByRole('dialog', { name: '任务详情', exact: true }).getByRole('region', { name: '任务详细信息' }).waitFor();
      check(!await page.locator('.video-player-dialog video').evaluate(video => video.loop), 'normal playback unexpectedly enabled looping');
      await page.keyboard.press('Escape');
      await page.getByRole('dialog', { name: '任务详情', exact: true }).waitFor({ state: 'detached' });
      await openStream(row('visual-7').locator('.task-context-target'));
      await page.waitForFunction(() => document.querySelector('.stream-video-slot[hidden] video')?.readyState >= 3);
      await standbyVideo.evaluate(video => { window.streamClosingVideo = video; });
      await page.keyboard.press('Escape');
      await stream.waitFor({ state: 'detached' });
      check(await page.evaluate(() => window.streamClosingVideo.paused && !window.streamClosingVideo.hasAttribute('src')), 'closing kept the preload source open');

      await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播视频');
      await page.evaluate(async () => {
        const { api } = await import('/src/api.ts');
        const { writePlaybackPosition } = await import('/src/lib/playback-position.ts');
        const next = (await api.tasks()).find(task => task.id === 'visual-9');
        writePlaybackPosition(next, 1.2);
      });
      await openStream(trigger);
      await expectPlaying('连播视频 A');
      await page.waitForFunction(() => {
        const video = document.querySelector('.stream-video-slot[hidden] video');
        return video?.readyState >= 3 && !video.seeking && Math.abs(video.currentTime - 1.2) < 0.05;
      });
      await standbyVideo.evaluate(video => {
        window.warmNextVideo = video;
        window.warmTransitionShowedLoading = false;
        const slot = video.closest('.stream-video-slot');
        const observer = new MutationObserver(() => {
          if (!slot.hidden && slot.querySelector('.video-player-loading')) window.warmTransitionShowedLoading = true;
        });
        observer.observe(slot, { subtree: true, childList: true, attributes: true, attributeFilter: ['hidden'] });
      });
      await wheel(1);
      await expectPlaying('连播视频 D');
      check(await visibleVideo.evaluate(video => video === window.warmNextVideo && !window.warmTransitionShowedLoading), 'preloaded video-to-video switch showed a loading indicator or recreated the video');
      await page.keyboard.press('Escape');
      await stream.waitFor({ state: 'detached' });

      await page.evaluate(async () => {
        const { api } = await import('/src/api.ts');
        const prepare = api.playbackSource;
        window.warmAttempts = 0;
        api.playbackSource = async id => {
          if (id === 'visual-9' && ++window.warmAttempts === 1) throw 'FILE_MISSING';
          return prepare(id);
        };
      });
      await openStream(trigger);
      await expectPlaying('连播视频 A');
      await page.waitForFunction(() => window.warmAttempts === 1);
      await wheel(1);
      await expectPlaying('连播视频 D');
      check(await page.evaluate(() => window.warmAttempts === 2 && !window.backgroundTest.calls.some(call => call.type === 'play')), 'failed preload did not retry on selection');
      await page.keyboard.press('Escape');
      await stream.waitFor({ state: 'detached' });
    }
    results.push(`${width}x${height}: fixed 16:9 streaming across video, gallery and backup`);
  }
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-media-wheel');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('连播');
  await page.locator('[data-asset-id="wheel-cloud"]').waitFor();
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const load = api.assetPlaybackSource;
    api.assetPlaybackSource = async id => {
      if (!window.delayStreamAsset) throw 'FILE_MISSING';
      await new Promise(resolve => { window.releaseStreamAsset = resolve; });
      return load(id);
    };
  });
  await openStream(row('visual-9').locator('.task-context-target'));
  await expectPlaying('连播视频 D');
  await wheel(1);
  await active('连播远端视频 E').getByRole('alert').waitFor();
  await outgoing.waitFor({ state: 'detached' });
  await wheel(-1);
  await expectPlaying('连播视频 D');
  results.push('stream source failure clears the old page and allows browsing back');
  await page.evaluate(() => { window.delayStreamAsset = true; });
  await wheel(1);
  await active('连播远端视频 E').waitFor();
  await page.waitForFunction(() => typeof window.releaseStreamAsset === 'function');
  await page.keyboard.press('Escape');
  await stream.waitFor({ state: 'detached' });
  await page.evaluate(() => window.releaseStreamAsset());
  await page.waitForTimeout(180);
  check(await stream.count() === 0, 'closing during a slow load reopened streaming playback');
  results.push('closing a stream during a slow load keeps the dialog closed');

  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-live');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.getByRole('textbox', { name: '搜索作品标题或作者' }).fill('实况图集');
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const readImage = api.image;
    const readVideo = api.albumVideoSource;
    api.image = async (id, index) => {
      if (id === 'visual-0' && index === 2) await new Promise(resolve => { window.releaseStreamStill = resolve; });
      return readImage(id, index);
    };
    api.albumVideoSource = async (id, index) => {
      if (id === 'visual-0' && index === 0 && window.holdStreamLiveWrap) await new Promise(resolve => { window.releaseStreamLiveWrap = resolve; });
      return readVideo(id, index);
    };
  });
  await openStream(row('visual-0').locator('.task-context-target'));
  await active('实况图集').waitFor();
  await page.waitForFunction(() => typeof window.releaseStreamStill === 'function', undefined, { timeout: 12000 });
  const retainedAlbumFrame = stream.locator('.album-stage canvas.media-work-frame');
  const hasFrame = () => retainedAlbumFrame.evaluate(canvas => canvas.getContext('2d').getImageData(0, 0, 1, 1).data[3] === 255);
  check(await hasFrame(), 'live-to-still slideshow did not retain the live frame while loading');
  await page.evaluate(() => window.releaseStreamStill());
  await stream.getByRole('img', { name: '第 3 张图片' }).waitFor();
  await retainedAlbumFrame.waitFor({ state: 'detached' });
  await page.evaluate(() => { window.holdStreamLiveWrap = true; });
  await page.waitForFunction(() => typeof window.releaseStreamLiveWrap === 'function');
  check(await hasFrame(), 'slideshow wrap to live media did not retain the previous image');
  await page.evaluate(() => window.releaseStreamLiveWrap());
  await page.waitForFunction(() => {
    const video = document.querySelector('.stream-playback-dialog .album-stage video');
    return video?.getAttribute('aria-label') === '第 1 张实况视频' && !video.paused && video.currentTime > 0;
  });
  await retainedAlbumFrame.waitFor({ state: 'detached' });
  results.push('mixed slideshow retains the previous frame on live and still loads and loops');
  return { passed: results.length, results };
}
