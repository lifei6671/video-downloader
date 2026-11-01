// Synthetic IPC only: no real credentials, sessions, files or remote requests.
async page => {
  const p = await page.context().newPage(); p.setDefaultTimeout(7000);
  const errors = []; p.on('pageerror', e => errors.push(e.message));
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await p.addInitScript(() => {
    localStorage.setItem('yingqu.task-view', 'list');
    window.proxy = sessionStorage.getItem('test-telegram-proxy'); window.calls = []; window.proxyReadFails = true;
    const profile = () => ({ platform:'telegram', user_agent:'fixture', proxy:window.proxy, cookie_configured:false });
    const settings = () => ({ download_directory:'C:/fixture', filename_template:'{title}', max_downloads:2, profiles:[...Array.from(['douyin','tiktok','youtube'],platform=>({platform,user_agent:'fixture',proxy:null,cookie_configured:false})),profile()] });
    const metadata = { platform:'telegram', resolver:'grammers', kind:'video', title:'本机测试视频', author:'测试作者', video_id:'100_9', canonical_url:'https://t.me/c/100/9' };
    const task = { id:'local', revision:1, status:'completed', video:metadata, path:'C:/fixture/video.mp4', bytes:100, total_bytes:100, created_at:1, completed_at:2, canceling:false, file_missing:false, download_requested:true, cover_cached:false, files:[] };
    const space = { space_id:'space', library_id:'library', display_name:'测试备份', provider_type:'telegram', profile:'telegram', telegram:{account_id:'test',channel_id:'100'}, enabled:true, credentials_configured:false, can_delete_owned_media:false, last_error:null };
    const asset = { asset_id:'remote', task_id:null, metadata:{...metadata,title:'远端测试视频'}, original_filename:'video.mp4', content_bytes:100, content_sha256:'a'.repeat(64), asset_state:'active', path:null, local_state:'missing', space_id:'space', remote_state:'backed_up', has_cover:false, has_transcript:true, remote_revision:1, remote_deleted:false, has_pending_local_changes:false, conflict_count:0 };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
    window.__TAURI_INTERNALS__ = {convertFileSrc:x=>x,transformCallback:()=>1,unregisterCallback(){},invoke:async(command,args)=>{
      window.calls.push({command,args});
      if(command==='get_settings') return settings();
      if(command==='get_profile_cookie') return '';
      if(command==='get_profile_proxy') { if(args.platform!=='telegram') return null; if(window.proxyReadFails) throw 'CREDENTIAL_ERROR'; return window.proxy; }
      if(command==='save_profile') { if(window.saveFails) throw 'DATABASE_ERROR'; window.proxy=args.update.proxy||null; if(window.proxy)sessionStorage.setItem('test-telegram-proxy',window.proxy);else sessionStorage.removeItem('test-telegram-proxy'); return settings(); }
      if(command==='list_tasks') return [task];
      if(command==='get_backup_settings') return space;
      if(command==='list_backup_spaces') return [space];
      if(command==='get_backup_sync_enabled') return true;
      if(command==='list_backup_assets') return [asset];
      if(command==='list_video_transcripts'||command==='list_backup_operations') return [];
      if(command==='check_runtime_dependencies') return {platform:'windows',directory:'C:/fixture',tools:[]};
      if(command==='telegram_storage_status') return {configured:true,phase:'ready',operation_id:'session',account:{id:'test',name:'测试账号'}};
      return null;
    }};
  });
  try {
    await p.goto(page.url().split('/').slice(0,3).join('/')+'/');
    await p.getByRole('button',{name:'系统设置',exact:true}).click();
    await p.getByRole('button',{name:'平台账号',exact:true}).click();
    await p.getByRole('tab',{name:'Telegram',exact:true}).click();
    const form = p.getByRole('form',{name:'Telegram 网络代理'});
    const proxyCard = p.locator('[data-slot=card]').filter({has:form});
    await form.getByRole('alert').waitFor();
    check(await proxyCard.getByRole('button',{name:'保存代理',exact:true}).isDisabled(),'failed read allowed proxy overwrite');
    await p.evaluate(()=>window.proxyReadFails=false);
    await proxyCard.getByRole('button',{name:'重新读取代理',exact:true}).click();
    const save = () => proxyCard.getByRole('button',{name:'保存代理',exact:true});
    const address = () => form.getByRole('textbox',{name:'SOCKS5 地址',exact:true});
    const success = () => p.locator('[data-sonner-toast]').filter({hasText:'代理配置已保存'});
    check(await form.getByRole('radio',{name:'直连',exact:true}).isChecked(),'unconfigured Telegram is not direct');
    check(await save().isDisabled(),'unchanged direct mode can be saved');
    const beforeUnchangedSubmit=await p.evaluate(()=>window.calls.filter(c=>c.command==='save_profile').length);
    await form.evaluate(form=>form.requestSubmit());
    check(await p.evaluate(()=>window.calls.filter(c=>c.command==='save_profile').length)===beforeUnchangedSubmit,'unchanged form submission reaches the backend');
    await form.getByRole('radio',{name:'自定义代理',exact:true}).click();
    check(await save().isDisabled(),'blank proxy can be saved');
    await address().fill('socks5://127.0.0.1:1080');
    check(await save().isEnabled(),'changed proxy cannot be saved');
    await form.getByRole('radio',{name:'直连',exact:true}).click();
    check(await save().isDisabled(),'reverting proxy mode stays dirty');
    await form.getByRole('radio',{name:'自定义代理',exact:true}).click();
    await p.evaluate(()=>window.saveFails=true); await proxyCard.getByRole('button',{name:'保存代理',exact:true}).click();
    await form.getByRole('alert').waitFor(); check(await p.evaluate(()=>window.proxy)===null,'failed save mutated configuration');
    check(await save().isEnabled(),'failed save cleared unsaved changes');
    check(await success().count()===0,'failed save shows a success toast');
    await p.evaluate(()=>window.saveFails=false); await proxyCard.getByRole('button',{name:'保存代理',exact:true}).click();
    await success().first().waitFor(); check(await p.evaluate(()=>window.proxy)==='socks5://127.0.0.1:1080','proxy not saved');
    check(await form.getByText('代理配置已保存',{exact:true}).count()===0,'success message remains embedded in the form');
    check(await save().isDisabled(),'successful save remains dirty');
    await address().fill('  socks5://127.0.0.1:1080  ');
    check(await save().isDisabled(),'proxy whitespace enables a redundant save');
    await address().fill('socks5://127.0.0.1:1081');
    check(await save().isEnabled(),'editing a saved proxy address cannot be saved');
    await address().fill('socks5://127.0.0.1:1080');
    check(await save().isDisabled(),'reverting the address stays dirty');
    await p.reload();await p.getByRole('button',{name:'系统设置',exact:true}).click();await p.getByRole('button',{name:'平台账号',exact:true}).click();await p.getByRole('tab',{name:'Telegram',exact:true}).click();
    await form.getByRole('alert').waitFor();await p.evaluate(()=>window.proxyReadFails=false);await proxyCard.getByRole('button',{name:'重新读取代理',exact:true}).click();
    await p.waitForFunction(()=>document.querySelector('#telegram-proxy')?.value==='socks5://127.0.0.1:1080');
    check(await save().isDisabled(),'reading a saved custom proxy marks it dirty');
    await form.getByRole('radio',{name:'直连',exact:true}).click(); await proxyCard.getByRole('button',{name:'保存代理',exact:true}).click();
    await p.waitForFunction(()=>window.proxy===null);
    check(await save().isDisabled(),'saving direct mode remains dirty');
    check(await form.getByText('代理配置已保存',{exact:true}).count()===0,'direct save embeds a success message');
    await success().first().waitFor();
    const proxyStyles = async target => {
      await p.mouse.move(0,0);
      await target.evaluate(async form => {
        await Promise.all(form.getAnimations({subtree:true}).map(animation=>animation.finished.catch(()=>{})));
      });
      return target.evaluate(form => {
      const card = form.closest('[data-slot="card"]');
      const sample = element => {
        const css = getComputedStyle(element);
        return Object.fromEntries(['backgroundColor','color','fontSize','lineHeight','borderColor','borderWidth','borderRadius','height'].map(key=>[key,css[key]]));
      };
      const button = card.querySelector('button[type="submit"]');
      const edge = card.getBoundingClientRect().right-button.getBoundingClientRect().right;
      return {
        selected: sample(form.querySelector('[role="radio"][data-state="on"]')),
        unselected: sample(form.querySelector('[role="radio"][data-state="off"]')),
        input: sample(form.querySelector('input[id$="-proxy"]')),
        label: sample(form.querySelector('[data-slot="field-label"]:has(+ [data-slot="toggle-group"])')),
        save: sample(button),
        card: {background:getComputedStyle(card).backgroundColor,border:getComputedStyle(card).borderWidth},
        saveRightInset: Math.round(edge),
      };
      });
    };
    const styleViewports = [];
    for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      const references = [];
      const referenceWidths = [];
      for (const platform of ['抖音','TikTok','YouTube']) {
        await p.getByRole('tab',{name:platform,exact:true}).click();
        const peer = p.locator('form[id^="profile-"]:visible');
        await peer.getByRole('radio',{name:'自定义代理',exact:true}).click();
        await peer.getByRole('textbox',{name:'HTTP / SOCKS5 地址',exact:true}).fill('socks5://127.0.0.1:1080');
        references.push(await proxyStyles(peer));
        referenceWidths.push(await peer.evaluate(form=>form.closest('[data-slot="card"]').getBoundingClientRect().width));
      }
      await p.getByRole('tab',{name:'Telegram',exact:true}).click();
      await form.getByRole('radio',{name:'自定义代理',exact:true}).click();
      await form.getByRole('textbox',{name:'SOCKS5 地址',exact:true}).fill('socks5://127.0.0.1:1080');
      check(await proxyCard.getByText('测试账号',{exact:true}).isVisible(),'login account is outside the Telegram card');
      for(const label of ['切换其他账号','退出账号']) check(await proxyCard.getByRole('button',{name:label,exact:true}).isVisible(),label+' is outside the Telegram card');
      const accountLayout = await proxyCard.evaluate(card=>{
        const bounds=card.getBoundingClientRect();
        const proxy=card.querySelector('#telegram-proxy-mode').getBoundingClientRect();
        const label=[...card.querySelectorAll('[data-slot="field-label"]')].find(node=>node.textContent==='登录账号').getBoundingClientRect();
        const controls=[...card.querySelectorAll('button')].filter(button=>['切换其他账号','退出账号'].includes(button.textContent));
        return {aligned:Math.abs(proxy.left-label.left)<1,contained:controls.every(button=>{const rect=button.getBoundingClientRect();return rect.top>=bounds.top&&rect.bottom<=bounds.bottom&&rect.left>=bounds.left&&rect.right<=bounds.right;})};
      });
      check(accountLayout.aligned&&accountLayout.contained,'login fields or actions are not aligned inside the card');
      check(await p.locator('form form').count()===0,'account content nested a form inside the proxy form');
      const actual = await proxyStyles(form);
      const actualWidth = await proxyCard.evaluate(card=>card.getBoundingClientRect().width);
      check(referenceWidths.every(width=>Math.abs(width-actualWidth)<=16),'Telegram proxy card remains narrower than other platforms');
      for(const reference of references) check(JSON.stringify(actual)===JSON.stringify(reference),'Telegram proxy styles differ from other platforms at '+width+'px: '+JSON.stringify({actual,reference}));
      await form.getByRole('textbox',{name:'SOCKS5 地址',exact:true}).focus();
      for(const label of ['切换其他账号','退出账号','保存代理']) {
        await p.keyboard.press('Tab');
        check(await proxyCard.getByRole('button',{name:label,exact:true}).evaluate(button=>button===document.activeElement),'incorrect keyboard order for '+label);
      }
      await p.keyboard.press('Enter');
      await p.waitForFunction(()=>window.proxy==='socks5://127.0.0.1:1080');
      await proxyCard.getByRole('button',{name:'保存代理',exact:true}).waitFor({state:'visible'});
      await p.screenshot({path:'.local/telegram-proxy-save/telegram-'+width+'.png'});
      await form.getByRole('radio',{name:'直连',exact:true}).click();
      await proxyCard.getByRole('button',{name:'保存代理',exact:true}).click();
      await p.waitForFunction(()=>window.proxy===null);
      styleViewports.push(width);
    }
    await p.getByRole('button',{name:'下载任务',exact:true}).click();
    const menu = () => p.getByRole('menu',{name:'任务操作',exact:true});
    const local = () => p.locator('[data-task-id="local"]'); const remote = () => p.locator('[data-asset-id="remote"]');
    const entries = async target => { await target.click({button:'right',force:true}); const labels = await menu().getByRole('menuitem').allTextContents(); await p.keyboard.press('Escape'); return labels; };
    const surface = target => target.evaluate(async row=>{
      await Promise.all(row.getAnimations().map(animation=>animation.finished.catch(()=>{})));
      const css=getComputedStyle(row);
      return {border:css.borderWidth,shadow:css.boxShadow,background:css.backgroundColor};
    });
    const checkFlatRows = async layout => {
      for(const row of [local(),remote()]) {
        const style=await surface(row);
        check(style.border==='0px'&&style.shadow==='none',layout+' row has an unexpected frame: '+JSON.stringify(style));
      }
    };
    const cases = [];
    for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
      await p.setViewportSize({width,height});
      await p.mouse.move(0,0);
      await checkFlatRows('list');
      const idleBackground=(await surface(remote())).background;
      await remote().hover();
      check((await surface(remote())).background!==idleBackground,'remote row lost hover feedback');
      await p.mouse.move(0,0);
      await local().locator('.task-row-title').click({button:'right',force:true});
      await menu().waitFor();
      const selectedBackground=(await surface(local())).background;
      await p.keyboard.press('Escape'); await menu().waitFor({state:'hidden'});
      const listItems = await entries(local().locator('.task-row-title'));
      check(JSON.stringify(await entries(local().locator('[data-media-cover]')))===JSON.stringify(listItems),'list cover menu differs');
      const edge = await remote().evaluate(row=>({right:row.getBoundingClientRect().right,controls:row.querySelector('.task-row-controls').getBoundingClientRect().right}));
      check(edge.right-edge.controls<=20,'remote icons too far from right edge');
      check(await remote().getByRole('button',{name:'更多操作',exact:true}).count()===0,'remote ellipsis remains');
      await remote().click({button:'right'}); const remoteList = await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem').allTextContents();
      check((await surface(remote())).background===selectedBackground,'remote row lost open-menu selection feedback');
      await checkFlatRows('list menu');
      await p.keyboard.press('Escape');
      await p.screenshot({path:'.local/task-row-border/list-'+width+'.png'});
      await p.getByRole('button',{name:'切换为信息流',exact:true}).click();
      await checkFlatRows('feed');
      check(JSON.stringify(await entries(local().locator('.task-row-title')))===JSON.stringify(listItems),'list/card menu differs');
      await remote().click({button:'right'}); check(JSON.stringify(await p.getByRole('menu',{name:'远端视频操作',exact:true}).getByRole('menuitem').allTextContents())===JSON.stringify(remoteList),'remote list/card menu differs');
      check((await surface(remote())).background===selectedBackground,'remote feed card lost menu selection feedback');
      await p.keyboard.press('Escape');
      await p.screenshot({path:'.local/task-row-border/feed-'+width+'.png'});
      await p.getByRole('button',{name:'切换为列表',exact:true}).click();
      await remote().focus(); await p.keyboard.press('Tab'); await p.keyboard.press('Shift+Tab');
      check(await remote().evaluate(row=>row===document.activeElement&&row.matches(':focus-visible')&&parseFloat(getComputedStyle(row).outlineWidth)>0),'remote row lost its keyboard focus indicator');
      await p.keyboard.press('Shift+F10'); await p.getByRole('menu',{name:'远端视频操作',exact:true}).waitFor(); await p.keyboard.press('Escape');
      const trigger = local().locator('..'); await trigger.focus(); await p.keyboard.press('Shift+F10'); await menu().waitFor(); await p.keyboard.press('Escape');
      cases.push(width);
    }
    check(errors.length===0, errors.join('\n'));
    return {passed:true,viewports:cases,proxyStyleViewports:styleViewports,flows:['default direct and unchanged save disabled','proxy edit/revert/save baseline','success toast only','saved custom proxy reload','proxy read/save failure','SOCKS5 save','direct','local/remote menu parity','no ellipsis','right alignment','flat task rows','hover/menu selection','keyboard focus']};
  } finally { await p.close(); }
}
