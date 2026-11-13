// Playwright CLI run-code against tests/fixtures/weixin-channels.html.
async page => {
  const fixture = page.url();
  const check = (condition, message) => { if (!condition) throw Error(message); };
  for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    await page.setViewportSize({width,height});
    await page.goto(fixture);
    const input = page.locator('#video-input');
    await input.fill('分享 https://weixin.qq.com/sph/Fixture');
    await page.getByRole('combobox', {name:'解析引擎'}).getByText('微信视频号').waitFor();
    await input.press('Enter');
    const progress = page.locator('[data-slot=resolve-progress]');
    await progress.getByRole('alert').getByText(/请登录腾讯元宝/).waitFor();
    await progress.getByRole('button', {name:'打开腾讯元宝'}).press('Enter');
    await progress.getByRole('button', {name:'配置 Cookie'}).click();
    const tab = page.getByRole('tab',{name:'微信视频号',exact:true});
    check(await tab.getAttribute('aria-selected') === 'true', 'wrong settings tab');
    const form = page.locator('#profile-weixin');
    const browser = form.getByRole('checkbox',{name:/按需使用系统默认浏览器/});
    await browser.uncheck();
    await form.getByRole('textbox',{name:'腾讯元宝网页 Cookie'}).fill('fixture=not-a-real-cookie');
    await page.getByRole('button',{name:'保存平台配置'}).click();
    await page.waitForFunction(() => window.weixinTest.state().cookieSaved);
    check(await tab.evaluate(node => node.getBoundingClientRect().right <= innerWidth), 'account tabs overflow');
    await page.getByRole('button',{name:'下载作品',exact:true}).click();
    await progress.getByRole('button',{name:'重新解析',exact:true}).click();
    await page.getByText('视频号测试作品', {exact:true}).waitFor();
    await page.getByRole('radio',{name:/H\.265/}).press('Space');
    await page.waitForFunction(() => window.weixinTest.state().selected === 'h265');
    const state = await page.evaluate(() => window.weixinTest.state());
    check(state.opened === 1 && state.submitted.resolver === 'weixin', 'wrong platform or login action');
    await page.evaluate(() => window.weixinTest.queue());
    await page.getByRole('button',{name:'下载任务',exact:true}).click();
    await page.getByRole('tab',{name:'微信视频号',exact:true}).press('Enter');
    await page.getByText('视频号测试作品',{exact:true}).waitFor();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'page overflow');
  }
  return {passed:3, viewports:[1440,1280,1024]};
}
