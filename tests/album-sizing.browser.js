// Production gallery with isolated image bytes and mocked native IPC.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const sizes = [[900, 1600], [1600, 900], [900, 900], [300, 2400], [2400, 300], [1200, 800]];
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const [width, height] of [[1440, 900], [1280, 800], [1024, 680]]) {
    await page.setViewportSize({ width, height });
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-sizing');
    await page.getByRole('button', { name: '下载任务', exact: true }).click();
    const trigger = page.locator('[data-task-id="visual-0"]').getByRole('button', { name: '查看图集', exact: true });
    await trigger.focus();
    await page.keyboard.press('Enter');
    const dialog = page.locator('.album-dialog');
    await dialog.waitFor();
    for (let index = 0; index < sizes.length; index++) {
      const [imageWidth, imageHeight] = sizes[index];
      const dot = dialog.getByRole('button', { name: `查看第 ${index + 1} 张图片`, exact: true });
      await dot.focus();
      await page.keyboard.press('Enter');
      await page.waitForFunction(({ imageWidth, imageHeight }) => {
        const image = document.querySelector('.album-dialog .album-stage .album-still:not([aria-hidden]) > img:last-child');
        const dialog = document.querySelector('.album-dialog');
        return image?.complete && image.naturalWidth === imageWidth && image.naturalHeight === imageHeight
          && Number(dialog?.style.getPropertyValue('--task-media-aspect')) === imageWidth / imageHeight;
      }, { imageWidth, imageHeight });
      const layout = await dialog.evaluate(element => {
        const stage = element.querySelector('.album-stage');
        const image = stage.querySelector('img:last-child');
        const rect = node => node.getBoundingClientRect().toJSON();
        return { dialog: rect(element), stage: rect(stage), image: rect(image), details: rect(element.querySelector('.task-details-content')),
          scrollHeight: element.scrollHeight, clientHeight: element.clientHeight,
          scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
          fit: getComputedStyle(image).objectFit };
      });
      check(layout.image.width <= layout.stage.width + 1 && layout.image.height <= layout.stage.height + 1, 'image escapes its media pane');
      check(layout.details.x >= layout.stage.right - 1, 'details are not to the right of the image');
      check(layout.dialog.x >= 47 && layout.dialog.y >= 47 && layout.dialog.right <= width - 47 && layout.dialog.bottom <= height - 47, 'dialog outside viewport inset');
      check(layout.scrollHeight <= layout.clientHeight + 1 && layout.scrollWidth <= layout.clientWidth + 1, 'gallery overflows dialog');
      check(layout.fit === 'scale-down', 'image can be enlarged beyond its native size');
      check(Math.abs(layout.stage.width - layout.stage.height * imageWidth / imageHeight) <= 1, 'stage adds letterboxing to the current image');
      check(layout.stage.width <= imageWidth + 1 && layout.stage.height <= imageHeight + 1, 'gallery exceeded its native size');
      check(await dot.getAttribute('aria-pressed') === 'true', 'keyboard dot selection failed');
      await page.keyboard.press('Enter');
      check(await dot.getAttribute('aria-pressed') === 'true', 'reselecting current dot changed media');
      results.push(`${width}x${height}: ${imageWidth}x${imageHeight}`);
      if (index === 0) await dialog.screenshot({ path: `.local/task-media-viewer/album-${width}.png` });
    }
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    check(await trigger.evaluate(element => element === document.activeElement), 'close did not restore focus');
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album');
    const preview = page.locator('.media-preview .album-stage');
    await preview.waitFor();
    check(Math.abs((await preview.boundingBox()).height - (width > 1280 ? 360 : width > 1100 ? 320 : 300)) < 1, 'inline preview stage changed');
    results.push(`${width}x${height}: inline gallery keeps preview height`);
  }
  await page.setViewportSize({ width: 1024, height: 680 });
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-sizing');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.locator('[data-task-id="visual-0"]').getByRole('button', { name: '查看图集', exact: true }).click();
  const dialog = page.locator('.album-dialog.has-media-size');
  await dialog.getByRole('button', { name: '查看第 2 张图片', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.album-dialog')?.style.getPropertyValue('--task-media-intrinsic-width') === '1600px');
  const initial = await dialog.locator('.album-stage').boundingBox();
  await page.setViewportSize({ width: 2560, height: 1600 });
  const grown = await dialog.boundingBox();
  const stage = await dialog.locator('.album-stage').boundingBox();
  check(stage.width > initial.width && stage.height > initial.height, 'large gallery did not grow with the window');
  check(Math.abs(stage.width / stage.height - initial.width / initial.height) < 0.01, 'image aspect ratio changed when resizing');
  check(Math.abs(stage.width - 1600) <= 1 && Math.abs(stage.height - 900) <= 1, 'gallery did not reach the native size when space was available');
  await page.setViewportSize({ width: 3200, height: 1800 });
  const capped = await dialog.boundingBox();
  check(capped.width === grown.width && capped.height === grown.height, 'gallery kept growing after reaching native size');
  await dialog.getByRole('button', { name: '查看第 3 张图片', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.album-dialog')?.style.getPropertyValue('--task-media-intrinsic-width') === '900px');
  check((await dialog.boundingBox()).width < grown.width, 'switching images retained the previous image’s native size');
  results.push('gallery grows proportionally up to native size and adapts to the next image ratio');
  return { passed: results.length, results };
}
