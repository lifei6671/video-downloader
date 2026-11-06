// Real card components, isolated IPC, no account or network changes.
async page => {
  const base = page.url().split('/').slice(0, 3).join('/');
  const results = [];
  for (const [width, height] of [[1440,900],[1280,800],[1024,680]]) {
    for (const view of ['feed','list']) {
      await page.setViewportSize({width,height});
      await page.evaluate(view => localStorage.setItem('yingqu.task-view',view),view);
      await page.goto(base+'/tests/fixtures/background-download.html?scenario=unified-card-status');
      await page.getByRole('button',{name:'下载任务',exact:true}).click();
      const row = page.locator('[data-task-id="reconnecting"]');
      await row.getByText('正在重连 Telegram，连接恢复后自动继续',{exact:true}).waitFor();
      if (await row.locator('[role="alert"]').count()) throw Error('reconnect displayed as an error');
      const badge = row.locator(view==='feed'?'.task-feed-status':'.task-row-status .badge');
      if (view==='feed') await badge.filter({hasText:'等待连接'}).waitFor();
      await row.getByRole('button',{name:view==='feed'?'取消下载':'取消'}).waitFor();
      const message = row.locator('.task-card-message');
      await message.focus();
      const bounds = await message.boundingBox();
      if (!bounds || bounds.x<0 || bounds.x+bounds.width>width) throw Error('reconnect status overflow');
      results.push({width,height,view,waiting:true,cancelAvailable:true});
    }
  }
  return results;
}
