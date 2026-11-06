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
    const widths = [];
    for (let index = 0; index < sizes.length; index++) {
      const [imageWidth, imageHeight] = sizes[index];
      const thumb = dialog.getByRole('button', { name: `查看第 ${index + 1} 张图片`, exact: true });
      await thumb.focus();
      await page.keyboard.press('Enter');
      await page.waitForFunction(({ imageWidth, imageHeight }) => {
        const image = document.querySelector('.album-dialog .album-stage > img:last-child');
        return image?.complete && image.naturalWidth === imageWidth && image.naturalHeight === imageHeight;
      }, { imageWidth, imageHeight });
      const layout = await dialog.evaluate(element => {
        const stage = element.querySelector('.album-stage');
        const image = stage.querySelector('img:last-child');
        const rect = node => node.getBoundingClientRect().toJSON();
        return { dialog: rect(element), stage: rect(stage), image: rect(image),
          scrollHeight: element.scrollHeight, clientHeight: element.clientHeight,
          scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
          fit: getComputedStyle(image).objectFit, ratio: Number(getComputedStyle(element).getPropertyValue('--album-image-ratio')) };
      });
      check(Math.abs(layout.ratio - imageWidth / imageHeight) < 0.001, 'decoded image ratio not applied');
      check(Math.abs(layout.stage.width / layout.stage.height - imageWidth / imageHeight) < 0.02, 'stage has wrong aspect ratio');
      check(layout.dialog.x >= 23 && layout.dialog.y >= 23 && layout.dialog.right <= width - 23 && layout.dialog.bottom <= height - 23, 'dialog outside viewport');
      check(layout.scrollHeight <= layout.clientHeight + 1 && layout.scrollWidth <= layout.clientWidth + 1, 'gallery overflows dialog');
      check(layout.fit === 'contain', 'image cropped or stretched');
      check(await thumb.getAttribute('aria-pressed') === 'true', 'keyboard thumbnail selection failed');
      await page.keyboard.press('Enter');
      check(Math.abs(await dialog.evaluate(element => Number(getComputedStyle(element).getPropertyValue('--album-image-ratio'))) - imageWidth / imageHeight) < 0.001, 'reselecting current image lost decoded dimensions');
      widths.push(layout.dialog.width);
      results.push(`${width}x${height}: ${imageWidth}x${imageHeight}`);
      if (width === 1440 && index < 2) await page.screenshot({ path: `output/playwright/album-sizing-${index === 0 ? 'portrait' : 'landscape'}.png` });
    }
    check(widths[1] > widths[0] + 100 && widths[2] > widths[0], 'dialog width does not follow selected image');
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    check(await trigger.evaluate(element => element === document.activeElement), 'close did not restore focus');
    await page.goto(base + '/tests/fixtures/background-download.html?scenario=visual-album');
    const preview = page.locator('.media-preview .album-stage');
    await preview.waitFor();
    check(Math.abs((await preview.boundingBox()).height - (width > 1280 ? 360 : width > 1100 ? 320 : 300)) < 1, 'inline preview stage changed');
    results.push(`${width}x${height}: inline gallery keeps preview height`);
  }
  return { passed: results.length, results };
}
