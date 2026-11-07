// Production gallery with isolated image bytes and mocked native IPC.
async page => {
  page.setDefaultTimeout(8000);
  const base = page.url().split('/').slice(0, 3).join('/');
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const sizes = [[900, 1600], [1600, 900], [900, 900], [300, 2400], [2400, 300], [2600, 800]];
  const sameBox = (first, second) => ['x', 'y', 'width', 'height'].every(key => Math.abs(first[key] - second[key]) <= 0.5);
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
    const stage = dialog.locator('.album-stage');
    await stage.waitFor();
    const initial = { dialog: await dialog.boundingBox(), stage: await stage.boundingBox() };
    for (let index = 0; index < sizes.length; index++) {
      const [imageWidth, imageHeight] = sizes[index];
      const dot = dialog.getByRole('button', { name: `查看第 ${index + 1} 张图片`, exact: true });
      await dot.focus();
      await page.keyboard.press('Enter');
      await page.waitForFunction(({ imageWidth, imageHeight }) => {
        const image = document.querySelector('.album-dialog .album-stage .album-still:not([aria-hidden]) > img:last-child');
        return image?.complete && image.naturalWidth === imageWidth && image.naturalHeight === imageHeight;
      }, { imageWidth, imageHeight });
      const layout = await dialog.evaluate(element => {
        const stage = element.querySelector('.album-stage');
        const image = stage.querySelector('img:last-child');
        const rect = node => node.getBoundingClientRect().toJSON();
        return { dialog: rect(element), stage: rect(stage), image: rect(image), dots: rect(stage.querySelector('.album-dots')),
          details: rect(element.querySelector('.task-details-content')),
          scrollHeight: element.scrollHeight, clientHeight: element.clientHeight,
          scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
          fit: getComputedStyle(image).objectFit };
      });
      check(layout.image.width <= layout.stage.width + 1 && layout.image.height <= layout.stage.height + 1, 'image escapes its media pane');
      check(layout.details.x >= layout.stage.right - 1, 'details are not to the right of the image');
      check(layout.dialog.x >= 47 && layout.dialog.y >= 47 && layout.dialog.right <= width - 47 && layout.dialog.bottom <= height - 47, 'dialog outside viewport inset');
      check(layout.scrollHeight <= layout.clientHeight + 1 && layout.scrollWidth <= layout.clientWidth + 1, 'gallery overflows dialog');
      check(layout.fit === 'scale-down', 'image can be enlarged beyond its native size');
      check(sameBox(initial.dialog, layout.dialog) && sameBox(initial.stage, layout.stage), 'switching a slide resized the gallery');
      check(Math.abs(layout.stage.width - layout.stage.height * 2600 / 2400) <= 1, 'stage does not fit the album-wide dimensions');
      check(layout.dots.top > layout.stage.top && layout.dots.bottom <= layout.stage.bottom - 7, 'pagination is outside the image stage');
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
  await page.waitForFunction(() => document.querySelector('.album-dialog')?.style.getPropertyValue('--task-media-intrinsic-width') === '2600px');
  const initial = await dialog.locator('.album-stage').boundingBox();
  await page.setViewportSize({ width: 3200, height: 2900 });
  const grown = await dialog.boundingBox();
  const stage = await dialog.locator('.album-stage').boundingBox();
  check(stage.width > initial.width && stage.height > initial.height, 'large gallery did not grow with the window');
  check(Math.abs(stage.width / stage.height - initial.width / initial.height) < 0.01, 'image aspect ratio changed when resizing');
  check(Math.abs(stage.width - 2600) <= 1 && Math.abs(stage.height - 2400) <= 1, 'gallery did not reach the album-wide size when space was available');
  await page.setViewportSize({ width: 3600, height: 3000 });
  const capped = await dialog.boundingBox();
  check(capped.width === grown.width && capped.height === grown.height, 'gallery kept growing after reaching native size');
  await dialog.getByRole('button', { name: '查看第 3 张图片', exact: true }).click();
  check(sameBox(await dialog.boundingBox(), capped), 'gallery changed size after selecting a square slide');
  results.push('gallery grows proportionally to the widest and tallest album dimensions and stays fixed when switching');
  await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album-sizing&tiny=1');
  await page.getByRole('button', { name: '下载任务', exact: true }).click();
  await page.locator('[data-task-id="visual-0"]').getByRole('button', { name: '查看图集', exact: true }).click();
  const tinyDot = dialog.getByRole('button', { name: '查看第 2 张图片', exact: true });
  await tinyDot.click();
  check(await tinyDot.getAttribute('aria-pressed') === 'true', 'small images clip the pagination controls');
  results.push('tiny images keep both dot controls accessible');
  return { passed: results.length, results };
}
